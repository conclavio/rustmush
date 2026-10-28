//! Staff recovery is an attributed authoritative act. It never rewrites a lease,
//! refunds a deposit, creates ore, or clears inventory replay markers. A forced
//! home persists its intent before the surface tick moves the rig; its completion
//! is a second, ordinal-bound act, so a restart between the two is harmless.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, ActionTxn,
    AuthoritativeActionOutput, CanonicalId, CmdError, Ctx, DefinitionRef, IdempotencyKeyText,
    ObjId, OwnerId, PerceivedRef, ProjectedParameterKind, ProviderOfferDraft,
    ResolvedActionInvocation, RevisionKey, RevisionKind, RevisionValidator,
    RevisionValidatorContext, RulesValue, StateFamilyHandle, SubjectKey, World,
};
use mush_economy::VaultId;
use mush_inventory::{InventoryFamilies, record::EndpointRef};
use mush_roll::commerce::ic_claims::IcCloseKind;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::port::families::PortHandles;
use crate::port::ship::{Berth, berth_of, hull_view};
use crate::port::site_map::{EngineReads, SurfaceReads};
use crate::port_tick::port_error;
use crate::surface::act::{self, Where};
use crate::surface::cursor::{PocketCursor, PocketLock};
use crate::surface::excursion::{ActKind, Charge, RigKey, RigRecord, TvState};

/// Registered action and result; kept separate from player work permissions.
pub const RECOVERY_ACTION: &str = "tacsim_surface_recovery";
pub const RECOVERY_OUTCOME: &str = "tacsim_surface_recovery_result";
pub const RECOVERY_KEY_PREFIX: &str = "tacsim.recovery/";
/// Normal-world facts that must still hold when the recovery commits.
pub(crate) fn revision_kind() -> Result<RevisionKind, ActionError> {
    CanonicalId::new("surface_recovery_world")
        .map(RevisionKind::new)
        .map_err(|_| integrity("the recovery revision kind is invalid"))
}
/// Recovery visits share the surface fire's work budget.
const MAX_RECOVERIES: usize = 4;

/// The staff operation stored in the receipt, never a player act kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryOperation {
    /// Cancel work and close the excursion, leaving the rig where it stands.
    Close,
    /// Bring the rig into its assigned hull's bay, retaining its hopper.
    Home,
    /// Move the whole hopper into its assigned hull's hold.
    Unload,
}

impl RecoveryOperation {
    fn parse(word: &str) -> Option<Self> {
        match word {
            "close" => Some(Self::Close),
            "home" => Some(Self::Home),
            "unload" => Some(Self::Unload),
            _ => None,
        }
    }

    fn word(self) -> &'static str {
        match self {
            Self::Close => "close",
            Self::Home => "home",
            Self::Unload => "unload",
        }
    }
}

/// Last staff recovery and, for home, the durable placement intent. Its ordinal
/// is the rig's replay fence. A later ordinary act makes this receipt historical.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryReceipt {
    pub ordinal: u32,
    pub staff: ObjId,
    pub operator: ObjId,
    pub hull: ObjId,
    pub slot: u8,
    pub operation: RecoveryOperation,
    pub at_ic: i64,
    pub pending: bool,
    /// Frozen home destination; rechecked against the current assignment (doc 23 cut S.3: the
    /// recoverer's bay when [`RecoveryReceipt::into`] is set).
    pub bay_room: Option<ObjId>,
    pub bay_slot: Option<String>,
    pub moved: BTreeMap<CanonicalId, u64>,
    /// **The hull a home brings another's abandoned rig into** (doc 23 cut S.3, the lead's A4;
    /// §2 "Recovery"): `Some` only on a home naming a second carrier, the bay fields then that
    /// hull's and the tick's completion fenced on its berth. [`RecoveryReceipt::hull`] stays the
    /// assigned hull, so every other reader of the receipt is unchanged, and the lease is
    /// untouched. **The S.3 fix round's R1**: such a home leaves the rig's epoch open and
    /// abandoned until its completion, whose finish closes it. Skipped when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub into: Option<ObjId>,
    /// **Whether this receipt owes the owner's one notice of the abandonment** (doc 23 cut S.3,
    /// the lead's A1; D-23-4: "the owner is told once"): the close that first flagged the open
    /// epoch `abandoned` — or (the fix round's R4) any receipt that replaced such a close before
    /// the tick told it, carrying the notice forward. A later close of the already-abandoned rig,
    /// or a close of a rig with no epoch open, says the plain close. Skipped when `false`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub abandoned: bool,
}

impl RecoveryReceipt {
    pub(crate) fn check(&self, record: &RigRecord) -> Result<(), String> {
        if self.ordinal == 0
            || self.ordinal > record.ordinal
            || self.slot == 0
            || u32::from(self.slot) > crate::definitions::logistics::MAX_BAY_ITEMS
            || self.moved.len() > crate::surface::excursion::MAX_DUMP_GOODS
            || self.moved.values().any(|quantity| *quantity == 0)
            || self
                .moved
                .values()
                .try_fold(0_u64, |sum, quantity| sum.checked_add(*quantity))
                .is_none()
            || (self.operation != RecoveryOperation::Unload && !self.moved.is_empty())
            // Doc 23 cut S.3 (A4): a recoverer is a home's, another hull than the assigned one.
            || self.into.is_some_and(|into| {
                self.operation != RecoveryOperation::Home || into == self.hull || into.0 == 0
            })
        {
            return Err("the staff recovery receipt is out of bounds".to_owned());
        }
        if self.pending
            && (self.operation != RecoveryOperation::Home
                || self.ordinal != record.ordinal
                || self.bay_room.is_none()
                || self
                    .bay_slot
                    .as_ref()
                    .is_none_or(|slot| mush_sim::SlotId::new(slot).is_none())
                || record.act.is_some()
                || record.run.is_some()
                || record.queued.is_some()
                || record.plan.is_some()
                || record.excursion.is_some())
        {
            return Err("a pending recovery owns an otherwise idle rig".to_owned());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RigFacts {
    record_digest: String,
    operator: ObjId,
    hull: ObjId,
    slot: u8,
    assigned: bool,
    docked: bool,
    in_bay: bool,
    at_home: bool,
    bay_room: Option<ObjId>,
    bay_slot: Option<String>,
    hopper: Option<u64>,
    goods: Vec<CanonicalId>,
    hold: Option<u64>,
    hold_plan: Option<crate::port::market::HoldPlan>,
    /// Authenticated physical charge at offer time, never submitter-supplied.
    charge: Charge,
    staff: bool,
    finish: bool,
    /// **Doc 23 cut S.1** (L6): whether the rig, its epoch open, is parked short of its way
    /// home ([`act::parked_short`]) — a home then closes the epoch `RecoveredToBase`.
    parked: bool,
    /// **The S.1 fix round's F3**: whether the rig stands in any bay (the tick's placement of a
    /// leaving act puts it in none).
    in_any_bay: bool,
    /// **The S.1 fix round's F3**: what the act in flight's finish would open the epoch with,
    /// when it leaves the bay (`crate::surface::black_box::epoch_facts`) — a close that fences
    /// that finish out opens the epoch from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    epoch: Option<crate::surface::black_box::EpochFacts>,
    /// **Doc 23 cut S.3** (A4): the hulls a staff home may bring this rig into instead of its
    /// own, by id — read only for staff, only for a rig whose epoch is open and abandoned and
    /// which stands on a site's ground: every other hull settled on that site with a readable
    /// bay ([`recoverer_berth`]), any owner's (ownership and salvage are doc 23 §8's).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    recoverers: BTreeMap<String, Recoverer>,
    /// **Doc 23 cut S.3** (A4): the berth of the hull a pending home brings the rig into, when
    /// its receipt names one ([`RecoveryReceipt::into`]) — what the tick's completion and the
    /// finish fence on instead of the assigned hull's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    into: Option<HomeBerth>,
    /// **The S.3 fix round's R4**: the rig's last receipt is an abandoning close whose notice
    /// the tick has not told yet — a receipt that replaces it carries the owner's one notice
    /// forward ([`RecoveryReceipt::abandoned`]).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    owed_abandonment: bool,
}

/// **A second carrier's bay, as a home into it reads it** (doc 23 cut S.3, A4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Recoverer {
    bay_room: ObjId,
    bay_slot: String,
    /// **The S.3 fix round's R1 (c)**: whether the bay's compartment has room for one more rig
    /// now; a home into a full bay is refused at the order (a bay that fills between the order
    /// and the placement waits on the tick's "cannot be placed").
    room: bool,
}

/// **Which second carriers a fact reading looks for** (doc 23 cut S.3, A4; the fix round's R5):
/// every candidate hull (the offer, its hull list built once), the one hull an order names (the
/// command, the revision check of an order naming one), or none.
#[derive(Clone, Copy)]
enum Seek<'a> {
    All(&'a BTreeSet<ObjId>),
    Only(ObjId),
    Skip,
}

/// **The berth a pending home lands in** (doc 23 cut S.3, A4): the hull, whether it is ready
/// for the rig (the assigned hull: docked; a recoverer: settled on the site the rig stands on,
/// or the rig already in its bay), whether the rig stands in its bay, and the bay.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HomeBerth {
    hull: ObjId,
    ready: bool,
    at_home: bool,
    bay_room: Option<ObjId>,
    bay_slot: Option<String>,
}

impl RigFacts {
    /// The berth a home into `into` (a recoverer) or into the assigned hull (`None`) lands in.
    fn home_berth(&self, into: Option<ObjId>) -> HomeBerth {
        match into {
            None => HomeBerth {
                hull: self.hull,
                ready: self.docked,
                at_home: self.at_home,
                bay_room: self.bay_room,
                bay_slot: self.bay_slot.clone(),
            },
            Some(hull) => self
                .into
                .clone()
                .filter(|berth| berth.hull == hull)
                .unwrap_or(HomeBerth {
                    hull,
                    ready: false,
                    at_home: false,
                    bay_room: None,
                    bay_slot: None,
                }),
        }
    }
}

/// **Every hull the programme ports and the yards record** (doc 23 cut S.3, A4): the candidates
/// for a second carrier, bounded by the port maps' hull rows.
pub(crate) fn world_hulls(w: &World) -> BTreeSet<ObjId> {
    let mut hulls: BTreeSet<ObjId> = crate::port::coop::programme_ports(w)
        .iter()
        .flat_map(|port| port.map.hulls.iter().map(|row| ObjId(row.hull)))
        .collect();
    hulls.extend(
        crate::port::yard::yards(w)
            .iter()
            .flat_map(|yard| yard.row.hulls.iter().map(|row| ObjId(row.hull))),
    );
    hulls
}

/// **`hull` as a second carrier** (doc 23 cut S.3, A4): a carrier with a readable bay settled
/// on a site (`crate::surface::tick::hull_view_berth`), its domain and its bay. The cheap
/// reads first — a bay declared, docked at a site — so a hull that is no carrier is never
/// asked for its berth (which pages a dark bay).
fn recoverer_berth(w: &World, hull: ObjId) -> Option<(mush_sim::DomainId, Recoverer)> {
    let maximum = hull_view(w, hull)?.bay()?.0.slot.maximum_items;
    if !matches!(berth_of(w, hull), Berth::Docked { def, .. } if def.site.is_some()) {
        return None;
    }
    let (_, domain, _, bay_room, bay_slot) = crate::surface::tick::hull_view_berth(w, hull)?;
    let room = w
        .module_state::<mush_sim::SpatialState>()
        .is_some_and(|state| {
            state.occupants(bay_room, Some(&bay_slot)) < usize::try_from(maximum).unwrap_or(0)
        });
    Some((
        domain,
        Recoverer {
            bay_room,
            bay_slot: bay_slot.as_str().to_owned(),
            room,
        },
    ))
}

/// **The second carriers `tv` may be brought into** (doc 23 cut S.3, A4): none unless its epoch
/// is open and abandoned and it stands on a site's ground; then every hull but `assigned`
/// settled on that site with a readable bay.
fn recoverers_of(
    w: &World,
    tv: ObjId,
    record: &RigRecord,
    assigned: ObjId,
    seek: Seek<'_>,
) -> BTreeMap<String, Recoverer> {
    let mut found = BTreeMap::new();
    let only;
    let hulls = match seek {
        Seek::Skip => return found,
        Seek::All(hulls) => hulls,
        Seek::Only(hull) => {
            only = BTreeSet::from([hull]);
            &only
        }
    };
    if !record.sortie.as_ref().is_some_and(|open| open.abandoned) {
        return found;
    }
    let Where::Ground { domain, .. } = act::where_is(w, tv) else {
        return found;
    };
    for &hull in hulls {
        if hull == assigned || hull == tv {
            continue;
        }
        if let Some((on, bay)) = recoverer_berth(w, hull)
            && on == domain
        {
            found.insert(hull.0.to_string(), bay);
        }
    }
    found
}

/// **The berth of the recoverer a pending home names** (doc 23 cut S.3, A4): ready while it is
/// settled on the site the rig stands on (or the rig already stands in its bay), with its bay.
fn into_berth(w: &World, tv: ObjId, hull: ObjId) -> HomeBerth {
    let berth = recoverer_berth(w, hull);
    let at_home = berth.as_ref().is_some_and(|(_, bay)| {
        w.module_state::<mush_sim::SpatialState>().is_some_and(|state| {
            matches!(state.parent_of(tv), mush_sim::SpatialParent::Interior { space, slot: Some(slot) }
                if space == bay.bay_room && slot.as_str() == bay.bay_slot)
        })
    });
    let on_site = berth.as_ref().is_some_and(|(domain, _)| {
        matches!(act::where_is(w, tv), Where::Ground { domain: on, .. } if on == *domain)
    });
    HomeBerth {
        hull,
        ready: berth.is_some() && (at_home || on_site),
        at_home,
        bay_room: berth.as_ref().map(|(_, bay)| bay.bay_room),
        bay_slot: berth.map(|(_, bay)| bay.bay_slot),
    }
}

fn identity(w: &World, tv: ObjId, record: &RigRecord) -> Option<(ObjId, ObjId, u8)> {
    if let Some(lease) = crate::port::yard::rig_lease(w, tv) {
        return Some((lease.pilot, lease.hull, lease.slot));
    }
    if let Some(open) = &record.excursion {
        return Some((open.operator, open.hull, open.slot));
    }
    if let Some(act) = &record.act {
        return Some((act.operator, act.hull, act.slot));
    }
    if let Some(run) = &record.run {
        return Some((run.operator, run.hull, run.slot));
    }
    if let Some(queue) = &record.queued {
        return Some((queue.operator, queue.hull, queue.slot));
    }
    if let Some(plan) = &record.plan {
        return Some((plan.author, plan.hull, plan.slot));
    }
    record
        .recovery
        .as_ref()
        .map(|r| (r.operator, r.hull, r.slot))
}

fn facts(
    w: &World,
    actor: ObjId,
    tv: ObjId,
    record: &RigRecord,
    reads: crate::surface::black_box::EpochReads<'_>,
    seek: Seek<'_>,
) -> Option<RigFacts> {
    let (operator, hull, slot) = identity(w, tv, record)?;
    let assigned = crate::port::yard::rig_lease(w, tv)
        .is_some_and(|lease| lease.hull == hull && lease.slot == slot);
    let view = hull_view(w, hull);
    let docked = matches!(berth_of(w, hull), Berth::Docked { .. });
    let bay = view.as_ref().and_then(|view| view.bay());
    let in_bay = act::where_is(w, tv) == (Where::Bay { hull });
    let at_home = bay.is_some_and(|(bay, room)| {
        w.module_state::<mush_sim::SpatialState>().is_some_and(|state| {
            matches!(state.parent_of(tv), mush_sim::SpatialParent::Interior { space, slot: Some(slot) }
                if space == room && slot.as_str() == bay.slot.id.as_str())
        })
    });
    let now = w.world_ic_now().at;
    let battery = w
        .module_state::<PortHandles>()
        .and_then(|handles| handles.surface_tools.as_ref())
        .and_then(|tools| act::rig_battery_id(w, tv).and_then(|id| act::battery_of(tools, &id)));
    let value = battery.map_or(record.charge.value_cu, |battery| {
        let settled_at = record
            .act
            .as_ref()
            .filter(|act| !in_bay && act.kind == crate::surface::excursion::ActKind::Launch)
            .map_or(now, |act| now.min(act.due_ic));
        crate::surface::excursion::charge_shown(record, settled_at, battery)
    });
    // A staff move may already have taken the rig out of the bay. Do not grant
    // recharge for an unknown interval on the ground.
    let known_launch = record.act.as_ref().is_some_and(|act| {
        act.kind == crate::surface::excursion::ActKind::Launch || act.params.launches()
    });
    let value = if !in_bay && record.charge.in_bay && !known_launch {
        record.charge.value_cu
    } else {
        value
    };
    let staff = crate::commands::is_staff(w, actor);
    // Doc 23 cut S.3 (A4): a pending home into a second carrier fences on that carrier's berth.
    let into = record
        .recovery
        .as_ref()
        .filter(|r| r.pending && r.ordinal == record.ordinal)
        .and_then(|r| r.into)
        .map(|into| into_berth(w, tv, into));
    let home = |r: &RecoveryReceipt| match (r.into, into.as_ref()) {
        (None, _) => {
            docked
                && at_home
                && r.bay_room == bay.map(|(_, room)| room)
                && r.bay_slot.as_deref() == bay.map(|(bay, _)| bay.slot.id.as_str())
        }
        (Some(hull), Some(berth)) => {
            berth.hull == hull
                && berth.ready
                && berth.at_home
                && r.bay_room == berth.bay_room
                && r.bay_slot == berth.bay_slot
        }
        (Some(_), None) => false,
    };
    let finish = record.recovery.as_ref().is_some_and(|r| {
        r.pending
            && r.ordinal == record.ordinal
            && r.staff == actor
            && r.hull == hull
            && r.slot == slot
            && r.operator == operator
            && assigned
            && home(r)
    });
    // Doc 23 cut S.3 (A4): the second carriers, read for staff only.
    let recoverers = if staff {
        recoverers_of(w, tv, record, hull, seek)
    } else {
        BTreeMap::new()
    };
    // The S.3 fix round's R4: an abandoning close not yet told.
    let owed_abandonment = record.recovery.as_ref().is_some_and(|r| {
        r.abandoned
            && r.ordinal == record.ordinal
            && !matches!(
                w.module_state::<crate::store::TacsimStore>().and_then(|store| store.tv_rec(tv)),
                Some(crate::surface::excursion::TvRecState::Present(rec))
                    if rec.recovery_told == Some(r.ordinal)
            )
    });
    // Doc 23 cut S.1 (L6): read only for a rig whose epoch is open (a home then closes it).
    let parked = record.sortie.is_some() && act::parked_short(w, record, operator, battery);
    // The S.1 fix round's F3: the placement facts a close fencing out a finish reads.
    let in_any_bay = matches!(act::where_is(w, tv), Where::Bay { .. });
    let epoch = crate::surface::black_box::epoch_facts(w, reads, record);
    let hopper = crate::port::vehicle::hopper_of(w, tv);
    let hold = view
        .as_ref()
        .and_then(|view| crate::port::ship::hold_vault(w, view));
    let active = |vault: VaultId| {
        matches!(
            mush_inventory::provider::endpoint_status(w, EndpointRef(vault)),
            mush_economy::EndpointState::Live
        )
    };
    Some(RigFacts {
        record_digest: record_digest(record)?,
        operator,
        hull,
        slot,
        assigned,
        docked,
        in_bay,
        at_home,
        bay_room: bay.map(|(_, room)| room),
        bay_slot: bay.map(|(bay, _)| bay.slot.id.to_string()),
        hopper: hopper.filter(|vault| active(*vault)).map(|vault| vault.0),
        goods: crate::port::tv::hopper_goods_of(w, tv).map_or_else(Vec::new, |(goods, _)| goods),
        hold: hold.filter(|vault| active(*vault)).map(|vault| vault.0),
        hold_plan: view.as_ref().and_then(crate::port::market::hold_plan),
        charge: Charge {
            value_cu: value,
            at_ic: now,
            in_bay,
        },
        staff,
        finish,
        parked,
        in_any_bay,
        epoch,
        recoverers,
        into,
        owed_abandonment,
    })
}

/// **The recovery's idempotency key**: the rig, the ordinal and the operation — and (the S.3
/// fix round's R6) the second carrier a home names, so `home #<rig>` and `home #<rig> #<hull>`
/// at one ordinal are two requests.
fn key(
    tv: ObjId,
    ordinal: u32,
    operation: &str,
    into: Option<ObjId>,
) -> Option<IdempotencyKeyText> {
    let text = match into {
        None => format!("{RECOVERY_KEY_PREFIX}{}/{ordinal}/{operation}", tv.0),
        Some(into) => format!(
            "{RECOVERY_KEY_PREFIX}{}/{ordinal}/{operation}/{}",
            tv.0, into.0
        ),
    };
    IdempotencyKeyText::parse_canonical(text).ok()
}

/// **The revision facet**: `staff_<actor>`, and (the S.3 fix round's R5)
/// `staff_<actor>_into_<hull>` for a home naming a second carrier — the one order whose
/// revision reads the recoverers.
fn facet_of(actor: ObjId, into: Option<ObjId>) -> String {
    match into {
        None => format!("staff_{}", actor.0),
        Some(into) => format!("staff_{}_into_{}", actor.0, into.0),
    }
}

/// [`facet_of`]'s inverse: the actor and the named second carrier.
fn parse_facet(facet: &str) -> Option<(ObjId, Option<ObjId>)> {
    let rest = facet.strip_prefix("staff_")?;
    match rest.split_once("_into_") {
        None => crate::port::families::strict_decimal_u32(rest).map(|actor| (ObjId(actor), None)),
        Some((actor, into)) => Some((
            ObjId(crate::port::families::strict_decimal_u32(actor)?),
            Some(ObjId(crate::port::families::strict_decimal_u32(into)?)),
        )),
    }
}

fn record_digest(record: &RigRecord) -> Option<String> {
    mush_core::CanonicalHash::for_canonical_value(
        "tacsim.recovery_record.v1",
        1,
        &serde_json::to_value(record).ok()?,
        65_536,
    )
    .ok()
    .map(mush_core::CanonicalHash::to_hex)
}

/// Recheck the chosen rig's placement, assignment, tools and actor authority at
/// commit. Inventory and the full rig record also have their typed-family fences.
pub(crate) struct RecoveryRevisionValidator {
    pub excursions: StateFamilyHandle<RigKey, RigRecord>,
}

impl RevisionValidator for RecoveryRevisionValidator {
    fn current_generation(
        &self,
        context: &RevisionValidatorContext<'_>,
        key: &RevisionKey,
    ) -> Result<Option<u64>, ActionError> {
        let SubjectKey::Object { id: tv } = key.subject else {
            return Ok(None);
        };
        let Some((actor, into)) = key
            .facet
            .as_ref()
            .and_then(|facet| parse_facet(facet.as_str()))
        else {
            return Ok(None);
        };
        let record = context
            .action_state(&self.excursions, &RigKey(tv))
            .map_err(|_| integrity("the recovery revision cannot read its rig"))?
            .cloned()
            .unwrap_or_else(|| RigRecord::fresh_unread(tv));
        let w = context.world();
        // The S.3 fix round's R5: the recoverers are read (and fence the commit) only for an
        // order naming one, and then that one alone — a hull landing on the site does not make
        // a plain close or an own-hull home of an abandoned rig ask again.
        let seek = into.map_or(Seek::Skip, Seek::Only);
        let Some(fixed) = facts(w, actor, tv, &record, (&|_| None, &|_| None), seek) else {
            return Ok(None);
        };
        let hull = fixed.hull;
        let mut facts = serde_json::to_value(fixed)
            .map_err(|_| integrity("the recovery revision cannot encode its facts"))?;
        // Passage of time alone does not change authority or placement. Charge
        // was settled at the offer instant; its input record remains guarded.
        let map = facts
            .as_object_mut()
            .ok_or_else(|| integrity("the recovery revision facts are not a map"))?;
        map.remove("charge");
        // The S.1 fix round's F3: the epoch facts are what a fenced-out finish would record
        // (the seal, the class, the site's open epochs, read across other rigs); they carry no
        // authority, so another rig's epoch moving between the offer and the commit does not
        // fence this recovery. The placement they depend on is guarded above.
        map.remove("epoch");
        let spatial = w.module_state::<mush_sim::SpatialState>();
        let battery = w
            .module_state::<PortHandles>()
            .and_then(|handles| handles.surface_tools.as_ref())
            .and_then(|tools| {
                act::rig_battery_id(w, tv).and_then(|id| act::battery_of(tools, &id))
            });
        let hash = mush_core::CanonicalHash::for_canonical_value(
            "tacsim.recovery_world.v1",
            1,
            &json!({
                "facts": facts,
                "rig_parent": spatial.map(|state| state.parent_of(tv)),
                "hull_parent": spatial.map(|state| state.parent_of(hull)),
                "lease": crate::port::yard::rig_lease(w, tv),
                "battery": battery,
                "actor_gone": crate::port_tick::gone(w, actor),
                "enabled": w.module_state::<crate::store::TacsimStore>()
                    .is_some_and(crate::store::TacsimStore::enabled),
            }),
            65_536,
        )
        .map_err(|_| integrity("the recovery revision is out of bounds"))?;
        let mut bytes = [0; 8];
        bytes.copy_from_slice(
            hash.as_bytes()
                .get(..8)
                .ok_or_else(|| integrity("the recovery revision digest is too short"))?,
        );
        Ok(Some(u64::from_le_bytes(bytes)))
    }
}

fn invalid(message: &'static str) -> ActionError {
    port_error(ActionErrorCode::InvalidRequest, message)
}
fn rejected(message: &'static str) -> ActionError {
    port_error(ActionErrorCode::ProviderRejected, message)
}
fn integrity(message: &'static str) -> ActionError {
    port_error(ActionErrorCode::Integrity, message)
}

fn parameters(invocation: &ActionInvocation) -> Result<&Map<String, Value>, ActionError> {
    if invocation.subject() != &PerceivedRef::SelfActor || !invocation.targets().is_empty() {
        return Err(invalid("staff recovery is self-scoped and has no targets"));
    }
    invocation
        .parameters()
        .as_object()
        .ok_or_else(|| invalid("recovery needs parameters"))
}

fn request(p: &Map<String, Value>) -> Result<(ObjId, u32, &str), ActionError> {
    let tv = p
        .get("tv")
        .and_then(Value::as_str)
        .and_then(crate::port::families::strict_decimal_u32)
        .map(ObjId)
        .ok_or_else(|| invalid("recovery names a rig"))?;
    let ordinal = p
        .get("ordinal")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n > 0)
        .ok_or_else(|| invalid("recovery names a positive ordinal"))?;
    let operation = p
        .get("operation")
        .and_then(Value::as_str)
        .filter(|word| *word == "finish" || RecoveryOperation::parse(word).is_some())
        .ok_or_else(|| invalid("recovery names close, home or unload"))?;
    Ok((tv, ordinal, operation))
}

/// **Doc 23 cut S.3** (A4): the optional `into` parameter — the hull a home brings another's
/// abandoned rig into, strict decimal text; a home's only.
fn into_of(p: &Map<String, Value>, operation: &str) -> Result<Option<ObjId>, ActionError> {
    let Some(value) = p.get("into") else {
        return Ok(None);
    };
    let into = value
        .as_str()
        .and_then(crate::port::families::strict_decimal_u32)
        .filter(|n| *n > 0)
        .map(ObjId)
        .ok_or_else(|| invalid("recovery into names one hull"))?;
    if operation != RecoveryOperation::Home.word() {
        return Err(invalid("only a home brings a rig into another hull"));
    }
    Ok(Some(into))
}

/// Provider registered by the port composition with delegated inventory handles.
pub(crate) struct RecoveryProvider {
    pub action: DefinitionRef,
    pub outcome: DefinitionRef,
    pub excursions: StateFamilyHandle<RigKey, RigRecord>,
    pub pockets: StateFamilyHandle<mush_industry::OreDepositId, PocketCursor>,
    pub inventory: InventoryFamilies,
    /// **Doc 23 cut S.1** (L4, L6): the black box participant's handle for this action — a
    /// staff home's close line.
    pub black_box: Option<
        mush_core::participant::ParticipantHandle<crate::surface::black_box::BlackBoxParticipant>,
    >,
}

impl ActionProvider for RecoveryProvider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        let p = parameters(invocation)?;
        let (_, _, operation) = request(p)?;
        into_of(p, operation).map(|_| ())
    }

    fn resolved_revision_keys(
        &self,
        resolved: &ResolvedActionInvocation,
    ) -> Result<Vec<RevisionKey>, ActionError> {
        let invocation = resolved.invocation();
        let p = parameters(invocation)?;
        let (tv, _, operation) = request(p)?;
        let into = into_of(p, operation)?;
        Ok(vec![RevisionKey {
            owner: OwnerId::new("tacsim")
                .map_err(|_| integrity("the recovery owner is invalid"))?,
            kind: revision_kind()?,
            subject: SubjectKey::Object { id: tv },
            facet: Some(
                CanonicalId::new(facet_of(invocation.actor(), into))
                    .map_err(|_| invalid("the recovery actor cannot be addressed"))?,
            ),
        }])
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        if !act::self_query(action, &self.action, query) {
            return Ok(None);
        }
        let w = &engine.world;
        if !w
            .module_state::<crate::store::TacsimStore>()
            .is_some_and(crate::store::TacsimStore::enabled)
        {
            return Ok(None);
        }
        let reads = EngineReads(engine);
        // The S.1 fix round's F3: the epoch facts' site count reads the world's rigs.
        let read_rig = |other: ObjId| reads.rig(other).ok().flatten();
        // Doc 23 cut S.2 (V7): the carrier's open epoch, its typed record.
        let read_carrier = |hull: ObjId| crate::port::epoch::open_id_in(engine, hull);
        let mut rigs = BTreeMap::new();
        // The S.3 fix round's R5: the second carriers' candidates, once an offer.
        let hulls = world_hulls(w);
        for tv in act::world_rigs(w) {
            let Ok(record) = reads.rig(tv) else { continue };
            let record = record.unwrap_or_else(|| RigRecord::fresh_unread(tv));
            if let Some(fixed) = facts(
                w,
                query.actor(),
                tv,
                &record,
                (&read_rig, &read_carrier),
                Seek::All(&hulls),
            ) && (fixed.staff || fixed.finish)
            {
                rigs.insert(tv.0.to_string(), fixed);
            }
        }
        if rigs.is_empty() {
            return Ok(None);
        }
        let choices = rigs.keys().cloned().collect();
        act::offer_draft(
            action,
            engine,
            json!({"rigs": rigs, "now": w.world_ic_now().at}),
            vec![
                act::parameter("tv", true, ProjectedParameterKind::Choice { choices })?,
                act::parameter(
                    "ordinal",
                    true,
                    ProjectedParameterKind::Integer {
                        minimum: 1,
                        maximum: i64::from(u32::MAX),
                    },
                )?,
                act::parameter(
                    "operation",
                    true,
                    ProjectedParameterKind::Choice {
                        choices: ["close", "home", "unload", "finish"]
                            .map(str::to_owned)
                            .to_vec(),
                    },
                )?,
                // Doc 23 cut S.3 (A4): a home's optional second carrier, checked in the prepare
                // against the rig's offered `recoverers`.
                act::parameter(
                    "into",
                    false,
                    ProjectedParameterKind::Text { maximum_bytes: 10 },
                )?,
            ],
            (
                "tacsim-surface-recovery",
                "Recover a rig",
                "Staff recovery of a stranded rig, preserving its ore and lease.",
            ),
        )
        .map(Some)
    }

    /// **Doc 23 cut S.1** (L4, L6): the recovery takes its preparation context, through which a
    /// staff home appends the epoch's close line in this act's own bundle.
    fn prepare_authoritative_with_context(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        context: &mut mush_core::PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let p = parameters(invocation)?;
        let (tv, ordinal, operation) = request(p)?;
        let now = p
            .get("now")
            .and_then(Value::as_i64)
            .ok_or_else(|| invalid("recovery fixes the IC instant"))?;
        let fixed: RigFacts = p
            .get("rigs")
            .and_then(Value::as_object)
            .and_then(|rigs| rigs.get(&tv.0.to_string()))
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .ok_or_else(|| {
                port_error(
                    ActionErrorCode::Unauthorized,
                    "that rig is not available for recovery",
                )
            })?;
        let found = act::read_record(txn, &self.excursions, tv)?;
        let existed = found.is_some();
        let mut record = found.unwrap_or_else(|| RigRecord::fresh_unread(tv));
        // Doc 23 cut S.1: the epoch's close line a staff home owes, appended below.
        let mut closing = None;
        record.check().map_err(|_| {
            integrity("the rig record cannot be recovered until its corruption is repaired")
        })?;
        if record_digest(&record).as_deref() != Some(fixed.record_digest.as_str()) {
            return Err(port_error(
                ActionErrorCode::Conflict,
                "the rig changed; request recovery again",
            ));
        }
        if operation == "finish" {
            let finishing = record.recovery.as_ref().map(|r| (r.into, r.staff));
            let receipt = record
                .recovery
                .as_mut()
                .filter(|r| {
                    r.pending
                        && r.ordinal == ordinal
                        && r.staff == invocation.actor()
                        && r.hull == fixed.hull
                        && r.slot == fixed.slot
                        && r.operator == fixed.operator
                })
                .ok_or_else(|| rejected("that recovery is no longer pending"))?;
            if !fixed.finish || record.ordinal != ordinal {
                return Err(rejected("the rig has not reached its assigned bay"));
            }
            receipt.pending = false;
            receipt.at_ic = now;
            // The S.3 fix round's R1: a home into a second carrier closes the epoch here, at its
            // completion — the placement is confirmed in this same submission, so the line names
            // the bay the rig reached. (An own-hull home closed it at its receipt: its base is
            // fixed, and a retry lands in the same base.)
            if let Some((Some(into), staff)) = finishing
                && let Some(open) = record.sortie.take()
            {
                closing = Some(
                    crate::surface::black_box::BlackBoxLine::sortie_closed(
                        tv,
                        &open,
                        now,
                        IcCloseKind::RecoveredToBase { base: into },
                    )
                    .recovered_by(staff),
                );
            }
            // Recharge starts when the durable finish confirms placement, never
            // while it was still stranded or waiting for room in the bay.
            record.charge = Charge {
                value_cu: record.charge.value_cu,
                at_ic: now,
                in_bay: true,
            };
        } else {
            if !fixed.staff {
                return Err(port_error(
                    ActionErrorCode::Unauthorized,
                    "only staff may start recovery",
                ));
            }
            if record.ordinal.checked_add(1) != Some(ordinal) {
                return Err(port_error(
                    ActionErrorCode::Conflict,
                    "the rig changed; request recovery again",
                ));
            }
            let into = into_of(p, operation)?.filter(|into| *into != fixed.hull);
            let operation =
                RecoveryOperation::parse(operation).ok_or_else(|| invalid("unknown recovery"))?;
            // Doc 23 cut S.3 (A4): a home into a second carrier — the rig's epoch open and
            // abandoned, the carrier one the offer read as settled on the rig's site with a
            // readable bay, the lease still standing (it is untouched).
            let recoverer = match into {
                None => None,
                Some(into) => {
                    if !record.sortie.as_ref().is_some_and(|open| open.abandoned) {
                        return Err(rejected(
                            "that rig is not abandoned; home it into its own hull",
                        ));
                    }
                    let bay = fixed.recoverers.get(&into.0.to_string()).ok_or_else(|| {
                        rejected("that hull is not settled on the rig's site with a readable bay")
                    })?;
                    if !bay.room {
                        return Err(rejected("that hull's bay has no room for the rig"));
                    }
                    if !fixed.assigned {
                        return Err(rejected(
                            "that rig's lease must stand to recover it into another hull",
                        ));
                    }
                    Some((into, bay.clone()))
                }
            };
            if operation != RecoveryOperation::Close
                && recoverer.is_none()
                && (!fixed.assigned
                    || !fixed.docked
                    || fixed.bay_room.is_none()
                    || fixed.bay_slot.is_none())
            {
                return Err(rejected(
                    "the rig's assigned hull must be docked with a readable bay",
                ));
            }
            // Doc 23 cut S.3 (A1): whether this close is the one that abandons the epoch.
            let mut abandons = false;
            if operation == RecoveryOperation::Unload && !fixed.in_bay {
                return Err(rejected(
                    "bring the rig into its assigned bay before unloading",
                ));
            }
            let moved = if operation == RecoveryOperation::Unload {
                self.unload(txn, &fixed)?
            } else {
                BTreeMap::new()
            };
            self.release_locks(txn, tv, &record)?;
            // Doc 23 cut S.1 (§6, D-23-1, D-23-4; L5, L6): the epoch at a staff recovery. A
            // close cancels the work and keeps the epoch, flagged abandoned — abandonment never
            // closes one; the close receipt's own line is the owner's one notice. A home closes
            // it at the receipt (the placement is the tick's, but the custody decision is the
            // staff's and is durable here): `RecoveredToBase` for a rig that was parked short of
            // its way home (the record's and the ground's facts, `RigFacts::parked`), else
            // `ReturnedToBase`, into the assigned hull. An unload, from the bay, moves neither.
            // `PoweredDownInBase` is unused (a rig's park is the bay) and `Destroyed` reserved.
            //
            // The S.1 fix round's F3: a close that lands between the tick's placement and the
            // finish (which it fences out) settles the custody fact the finish would have — an
            // act leaving the bay whose rig the tick already put out of every bay opens the
            // epoch here, flagged abandoned on the record (never on the open line); a recover
            // whose rig the tick already put in its bay closes it `ReturnedToBase`.
            //
            // Doc 23 cut S.3 (A1): the close that abandons an epoch records its instant
            // (`OpenSortie::abandoned_ic`), and a second close keeps the first; the flag and the
            // instant then live until a home or a recover closes the epoch. (A2, A3) A staff home
            // of an abandoned rig is a recovery, as is one of a parked rig (L6): it closes
            // `RecoveredToBase`, naming the staff actor as the recoverer; a home of a rig
            // mid-excursion with the charge to come home closes `ReturnedToBase`, as S.1 read it.
            // (A4) A home into a second carrier closes `RecoveredToBase` into that carrier.
            // (A6) `Destroyed` is reserved for a carrier destroyed with its rigs out: no
            // destruction path exists (doc 23 §8, §9's S.3 row), so nothing writes it.
            match operation {
                RecoveryOperation::Close => {
                    let leaving = record
                        .act
                        .as_ref()
                        .filter(|act| act.kind == ActKind::Launch || act.params.launches());
                    let recovering = record
                        .act
                        .as_ref()
                        .filter(|act| act.kind == ActKind::Recover);
                    if let (Some(act), None, Some(excursion), false) = (
                        leaving,
                        record.sortie.as_ref(),
                        record.excursion.as_ref(),
                        fixed.in_any_bay,
                    ) {
                        let (mut opened, line) = crate::surface::black_box::open_epoch(
                            tv,
                            excursion,
                            act.due_ic,
                            fixed.epoch.as_ref(),
                        );
                        opened.abandoned = true;
                        opened.abandoned_ic = Some(now);
                        abandons = true;
                        record.sortie = Some(opened);
                        closing = Some(line);
                    } else if recovering.is_some() && fixed.in_bay {
                        if let Some(open) = record.sortie.take() {
                            closing = Some(crate::surface::black_box::BlackBoxLine::sortie_closed(
                                tv,
                                &open,
                                now,
                                IcCloseKind::ReturnedToBase { base: fixed.hull },
                            ));
                        }
                    } else if let Some(open) = record.sortie.as_mut()
                        && !open.abandoned
                    {
                        open.abandoned = true;
                        open.abandoned_ic = Some(now);
                        abandons = true;
                    }
                }
                // The S.3 fix round's R1: an own-hull home closes the epoch at its receipt (S.1's
                // L6: its base is fixed, and a retry lands in the same base); a home into a
                // second carrier leaves it open and abandoned, and its completion's finish closes
                // it into the bay the rig reached — so a placement that never completes names no
                // bay, and a re-targeted home or a close still finds the rig abandoned.
                RecoveryOperation::Home if recoverer.is_some() => {}
                RecoveryOperation::Home => {
                    if let Some(open) = record.sortie.take() {
                        let base = fixed.hull;
                        let recovered = open.abandoned || fixed.parked;
                        closing = Some(
                            crate::surface::black_box::BlackBoxLine::sortie_closed(
                                tv,
                                &open,
                                now,
                                if recovered {
                                    IcCloseKind::RecoveredToBase { base }
                                } else {
                                    IcCloseKind::ReturnedToBase { base }
                                },
                            )
                            .recovered_by(invocation.actor()),
                        );
                    }
                }
                RecoveryOperation::Unload => {}
            }
            record.ordinal = ordinal;
            record.charge = fixed.charge;
            if operation == RecoveryOperation::Home {
                record.charge.in_bay = false;
            }
            record.act = None;
            record.run = None;
            record.queued = None;
            record.plan = None;
            record.excursion = None;
            record.last = None;
            record.recovery = Some(Box::new(RecoveryReceipt {
                ordinal,
                staff: invocation.actor(),
                operator: fixed.operator,
                hull: fixed.hull,
                slot: fixed.slot,
                operation,
                at_ic: now,
                pending: operation == RecoveryOperation::Home,
                bay_room: recoverer
                    .as_ref()
                    .map_or(fixed.bay_room, |(_, bay)| Some(bay.bay_room)),
                bay_slot: recoverer
                    .as_ref()
                    .map_or(fixed.bay_slot, |(_, bay)| Some(bay.bay_slot.clone())),
                moved,
                into: recoverer.as_ref().map(|(into, _)| *into),
                // The S.3 fix round's R4: an untold abandoning close's notice is carried forward.
                abandoned: abandons || fixed.owed_abandonment,
            }));
        }
        let receipt = record
            .recovery
            .as_ref()
            .ok_or_else(|| integrity("recovery has no receipt"))?;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({
                "actor": invocation.actor().0, "tv": tv.0, "ordinal": ordinal,
                "operation": operation, "pending": receipt.pending, "moved": receipt.moved,
            }),
        )
        .map_err(|_| integrity("the recovery result is out of bounds"))?;
        // The line rides this bundle; an open whose line was not written is recorded on the
        // epoch in the same record write (the S.1 fix round's F2).
        if let Some(line) = closing {
            let opens = matches!(
                line,
                crate::surface::black_box::BlackBoxLine::MindSealed { .. }
            );
            let written =
                crate::surface::black_box::append(context, self.black_box.as_ref(), tv, line)?;
            if opens
                && !written
                && let Some(open) = record.sortie.as_mut()
            {
                open.unrecorded = true;
            }
        }
        act::write_record(txn, &self.excursions, record, existed)?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

impl RecoveryProvider {
    fn release_locks(
        &self,
        txn: &mut ActionTxn<'_>,
        tv: ObjId,
        record: &RigRecord,
    ) -> Result<(), ActionError> {
        let Some(run) = &record.run else {
            return Ok(());
        };
        let ours = PocketLock {
            tv,
            ordinal: run.ordinal,
        };
        for row in &run.lock_rows {
            let before = act::read_cursor(txn, &self.pockets, row.deposit)?
                .ok_or_else(|| integrity("a mining lock is missing; repair its cursor first"))?;
            if !before
                .rows
                .iter()
                .any(|cursor| cursor.pocket == row.pocket && cursor.lock.as_ref() == Some(&ours))
            {
                return Err(integrity(
                    "a mining lock changed owner; repair its cursor first",
                ));
            }
            let after = crate::surface::mine::cursor_released(&before, row.pocket, &ours);
            act::write_cursor(txn, &self.pockets, row.deposit, true, after)?;
        }
        Ok(())
    }

    fn unload(
        &self,
        txn: &mut ActionTxn<'_>,
        fixed: &RigFacts,
    ) -> Result<BTreeMap<CanonicalId, u64>, ActionError> {
        let source = EndpointRef(VaultId(
            fixed
                .hopper
                .ok_or_else(|| rejected("the hopper is unavailable"))?,
        ));
        let destination = EndpointRef(VaultId(
            fixed
                .hold
                .ok_or_else(|| rejected("the hull's hold is unavailable"))?,
        ));
        let plan = fixed
            .hold_plan
            .as_ref()
            .ok_or_else(|| rejected("the hold's capacity cannot be read"))?;
        let mut moved = BTreeMap::new();
        let mut pool_count = 0_u32;
        for good in &fixed.goods {
            if let Some(pool) = self
                .inventory
                .pool(txn, source, good)
                .map_err(|_| integrity("a hopper pool cannot be read"))?
            {
                pool_count += 1;
                if pool.available != pool.quantity {
                    return Err(rejected(
                        "the hopper contains reserved ore; resolve its reservations first",
                    ));
                }
                if pool.quantity > 0 {
                    moved.insert(good.clone(), pool.quantity);
                }
            }
        }
        let census = txn
            .get(&self.inventory.census, &source)
            .map_err(|_| integrity("the hopper's pool census cannot be read"))?
            .copied()
            .unwrap_or_default()
            .get();
        if census != pool_count {
            return Err(integrity(
                "the hopper contains unknown pools; repair its inventory first",
            ));
        }
        // Check every incoming good cumulatively, including all measures and
        // existing reserved stock, before staging a single inventory mutation.
        let mut additions: BTreeMap<CanonicalId, u64> = BTreeMap::new();
        for (good, quantity) in &moved {
            crate::port::market::plan_check(plan, good, *quantity, &mut |id| {
                let held = self
                    .inventory
                    .pool(txn, destination, id)
                    .map_err(|_| mush_market::CapacityReadError)?
                    .map_or(0, |pool| pool.quantity);
                held.checked_add(additions.get(id).copied().unwrap_or(0))
                    .ok_or(mush_market::CapacityReadError)
            })
            .map_err(|_| {
                rejected("the hold cannot take the whole hopper; make room before unloading")
            })?;
            additions.insert(good.clone(), *quantity);
        }
        if !moved.is_empty() {
            let moves: Vec<_> = moved
                .iter()
                .map(|(good, qty)| (good.clone(), *qty))
                .collect();
            self.inventory
                .transfer_many(txn, source, destination, &moves)
                .map_err(|_| rejected("the hopper transfer failed; nothing was changed"))?;
        }
        Ok(moved)
    }
}

/// Submit a request; confirmation is emitted by the tick only after its receipt
/// lands. The command must not promise success before authoritative prepare.
///
/// **Doc 23 cut S.3** (A4): `into` is a home's optional second carrier
/// (`@tacsim/tv home #<rig> #<hull>`); naming the rig's own assigned hull is the plain home.
pub(crate) fn command(
    ctx: &mut Ctx<'_>,
    word: &str,
    tv: ObjId,
    into: Option<ObjId>,
) -> Result<(), CmdError> {
    crate::commands::require_enabled(ctx)?;
    crate::commands::require_staff(ctx)?;
    let operation = RecoveryOperation::parse(word)
        .ok_or_else(|| CmdError::msg("Unknown recovery operation."))?;
    if into.is_some() && operation != RecoveryOperation::Home {
        return Err(CmdError::msg("Only a home brings a rig into another hull."));
    }
    let handles = ctx
        .w
        .module_state::<PortHandles>()
        .cloned()
        .ok_or_else(|| CmdError::msg("Staff rig recovery is unavailable."))?;
    let action = handles
        .surface_recovery
        .ok_or_else(|| CmdError::msg("Staff rig recovery is unavailable."))?;
    if !act::world_rigs(ctx.w).contains(&tv) {
        return Err(CmdError::msg("No such terrain vehicle."));
    }
    let record = ctx
        .rig(tv)
        .map_err(CmdError::msg)?
        .unwrap_or_else(|| RigRecord::fresh_unread(tv));
    let seek = into.map_or(Seek::Skip, Seek::Only);
    let fixed = facts(ctx.w, ctx.actor, tv, &record, (&|_| None, &|_| None), seek)
        .ok_or_else(|| CmdError::msg("That rig has no recoverable assignment or excursion."))?;
    let into = into.filter(|into| *into != fixed.hull);
    if let Some(into) = into {
        if !record.sortie.as_ref().is_some_and(|open| open.abandoned) {
            return Err(CmdError::msg(
                "That rig is not abandoned; home it into its own hull.",
            ));
        }
        let Some(bay) = fixed.recoverers.get(&into.0.to_string()) else {
            return Err(CmdError::msg(
                "That hull is not settled on the rig's site with a readable bay.",
            ));
        };
        if !bay.room {
            return Err(CmdError::msg("That hull's bay has no room for the rig."));
        }
        if !fixed.assigned {
            return Err(CmdError::msg(
                "That rig's lease must stand to recover it into another hull.",
            ));
        }
    }
    if operation != RecoveryOperation::Close
        && into.is_none()
        && (!fixed.assigned
            || !fixed.docked
            || fixed.bay_room.is_none()
            || fixed.bay_slot.is_none())
    {
        return Err(CmdError::msg(
            "The rig's assigned hull must be docked with a readable bay.",
        ));
    }
    if operation == RecoveryOperation::Unload && !fixed.in_bay {
        return Err(CmdError::msg(
            "Bring the rig into its assigned bay before unloading.",
        ));
    }
    let ordinal = record
        .ordinal
        .checked_add(1)
        .ok_or_else(|| CmdError::msg("The rig's ordinal is exhausted."))?;
    let key = key(tv, ordinal, word, into)
        .ok_or_else(|| CmdError::msg("The recovery key is unavailable."))?;
    let mut parameters = json!({"tv": tv.0.to_string(), "ordinal": ordinal, "operation": word});
    if let (Some(into), Some(map)) = (into, parameters.as_object_mut()) {
        map.insert("into".to_owned(), Value::String(into.0.to_string()));
    }
    ctx.submit_authoritative(action, PerceivedRef::SelfActor, Vec::new(), parameters, key);
    ctx.out.info(
        ctx.actor,
        format!(
            "Recovery requested: {} rig #{}. Its committed result will be reported.",
            operation.word(),
            tv.0
        ),
    );
    Ok(())
}

/// Durable recovery work, run by the surface tick. The stored staff identity is
/// the authority for this exact finish even if its role is subsequently revoked;
/// the provider grants that actor no permission to begin another recovery.
pub(crate) fn tick(
    ctx: &mut Ctx<'_>,
    handles: &PortHandles,
    records: &[RigRecord],
    stats: &mut crate::surface::tick::SurfaceFireStats,
) {
    let mut worked = 0_u32;
    let mut actors = BTreeSet::new();
    for record in records {
        let Some(receipt) = record.recovery.as_ref() else {
            continue;
        };
        if worked as usize >= MAX_RECOVERIES {
            break;
        }
        if receipt.pending {
            if ctx.authoritative_pending(receipt.staff) || actors.contains(&receipt.staff) {
                stats.deferred += 1;
                continue;
            }
            let fixed = facts(
                ctx.w,
                receipt.staff,
                record.tv,
                record,
                (&|_| None, &|_| None),
                Seek::Skip,
            );
            // Doc 23 cut S.3 (A4): a home into a second carrier fences on that carrier's berth —
            // settled on the rig's site, the same bay — instead of the assigned hull's; the
            // lease still names the assigned hull and is fenced as before.
            let berth = fixed.as_ref().map(|fixed| fixed.home_berth(receipt.into));
            let reason = if crate::port_tick::gone(ctx.w, receipt.staff) {
                Some("the staff actor who requested it no longer exists")
            } else if fixed.as_ref().is_none_or(|fixed| {
                !fixed.assigned
                    || fixed.hull != receipt.hull
                    || fixed.slot != receipt.slot
                    || fixed.operator != receipt.operator
            }) {
                Some("the rig's assigned hull or operator changed")
            } else if berth.as_ref().is_some_and(|berth| !berth.ready) {
                Some(if receipt.into.is_some() {
                    "the recovering hull is no longer settled on the rig's site"
                } else {
                    "the assigned hull is no longer docked"
                })
            } else if berth.as_ref().is_some_and(|berth| {
                berth.bay_room != receipt.bay_room || berth.bay_slot != receipt.bay_slot
            }) {
                Some(if receipt.into.is_some() {
                    "the recovering hull's bay changed"
                } else {
                    "the assigned bay changed"
                })
            } else {
                None
            };
            if let Some(reason) = reason {
                if wait_notice(ctx, record, receipt, reason) {
                    worked += 1;
                    stats.told += 1;
                }
                continue;
            }
            let Some(action) = handles.surface_recovery.clone() else {
                continue;
            };
            let Some(berth) = berth else { continue };
            let Some(room) = receipt.bay_room else {
                continue;
            };
            let Some(slot) = receipt
                .bay_slot
                .as_ref()
                .and_then(|s| mush_sim::SlotId::new(s))
            else {
                continue;
            };
            if !berth.at_home && mush_sim::place_interior(ctx, record.tv, room, Some(slot)).is_err()
            {
                if wait_notice(
                    ctx,
                    record,
                    receipt,
                    if receipt.into.is_some() {
                        "the rig cannot be placed in the recovering hull's bay"
                    } else {
                        "the rig cannot be placed in its assigned bay"
                    },
                ) {
                    worked += 1;
                    stats.told += 1;
                }
                continue;
            }
            let Some(key) = key(record.tv, receipt.ordinal, "finish", None) else {
                continue;
            };
            // One closed typed call, not a general command dispatch. Unlike player
            // automation this may finish a recovery that GOD explicitly filed.
            let saved = ctx.actor;
            ctx.actor = receipt.staff;
            actors.insert(receipt.staff);
            ctx.submit_authoritative(action, PerceivedRef::SelfActor, Vec::new(),
                json!({"tv": record.tv.0.to_string(), "ordinal": receipt.ordinal, "operation": "finish"}), key);
            ctx.actor = saved;
            worked += 1;
            stats.submitted += 1;
        } else if project(ctx, record, receipt) {
            worked += 1;
            stats.told += 1;
        }
    }
    stats.worked += worked;
}

fn wait_notice(
    ctx: &mut Ctx<'_>,
    record: &RigRecord,
    receipt: &RecoveryReceipt,
    reason: &str,
) -> bool {
    let mut projection = crate::surface::tick::projection_for(
        ctx.w,
        record.tv,
        receipt.slot,
        receipt.operator,
        None,
    );
    if projection.recovery_wait_told == Some(receipt.ordinal) {
        return false;
    }
    // The S.3 fix round's R1 (d): a home into a second carrier names its own retry.
    let retry = match receipt.into {
        Some(into) => format!("#{} #{}", record.tv.0, into.0),
        None => format!("#{}", record.tv.0),
    };
    let line = format!(
        "Rig #{} recovery is waiting: {reason}. Staff can retry @tacsim/tv home {retry} or cancel with @tacsim/tv close #{}.",
        record.tv.0, record.tv.0
    );
    projection.recovery_wait_told = Some(receipt.ordinal);
    projection.push_log(ctx.w.world_ic_now().at, &line);
    if !crate::surface::tick::write_projection(ctx, &projection) {
        return false;
    }
    if !crate::port_tick::gone(ctx.w, receipt.staff) {
        crate::port_tick::notify(ctx, receipt.staff, &line);
    }
    if receipt.operator != receipt.staff {
        crate::port_tick::notify(ctx, receipt.operator, &line);
    }
    true
}

fn project(ctx: &mut Ctx<'_>, record: &RigRecord, receipt: &RecoveryReceipt) -> bool {
    let mut projection = if receipt.ordinal == record.ordinal {
        crate::surface::tick::projection_for(ctx.w, record.tv, receipt.slot, receipt.operator, None)
    } else {
        // Do not run the excursion privacy reset against a newer act's view.
        match ctx
            .w
            .module_state::<crate::store::TacsimStore>()
            .and_then(|store| store.tv_rec(record.tv))
        {
            Some(crate::surface::excursion::TvRecState::Present(rec)) => rec.clone(),
            Some(crate::surface::excursion::TvRecState::Unreadable) | None => {
                crate::surface::excursion::TvRec::new(record.tv, receipt.slot)
            }
        }
    };
    if projection.recovery_told == Some(receipt.ordinal) {
        return false;
    }
    // Doc 23 cut S.3 (A1; D-23-4): the close that abandoned the rig's epoch is the owner's one
    // notice of it, told once by `recovery_told` (and not again after a reboot); (A4) a home
    // into a second carrier names that carrier.
    let abandoned_line = format!(
        "Staff closed rig #{}'s excursion and cancelled its work. It stands where it is, abandoned, until staff bring it home; its hopper is unchanged.",
        record.tv.0
    );
    let line = match receipt.operation {
        RecoveryOperation::Close if receipt.abandoned => abandoned_line.clone(),
        RecoveryOperation::Close => format!(
            "Staff closed rig #{}'s excursion and cancelled its work. Its location and hopper are unchanged.",
            record.tv.0
        ),
        RecoveryOperation::Home => format!(
            "Staff brought rig #{} into hull #{}'s bay. Its hopper is unchanged; unload it before departure.",
            record.tv.0,
            receipt.into.unwrap_or(receipt.hull).0
        ),
        RecoveryOperation::Unload => format!(
            "Staff unloaded rig #{} into hull #{}'s hold ({} t). Its pending work is cleared.",
            record.tv.0,
            receipt.hull.0,
            receipt.moved.values().sum::<u64>()
        ),
    };
    // The S.3 fix round's R4: a receipt that replaced an untold abandoning close tells the
    // abandonment first, then its own line.
    let lines: Vec<String> = match receipt.operation {
        RecoveryOperation::Home | RecoveryOperation::Unload if receipt.abandoned => {
            vec![abandoned_line, line]
        }
        RecoveryOperation::Close | RecoveryOperation::Home | RecoveryOperation::Unload => {
            vec![line]
        }
    };
    // An ordinary act may already have started after a completed close/unload.
    // Report the durable receipt without erasing that newer projection.
    if receipt.ordinal == record.ordinal {
        projection.operator = Some(receipt.operator);
        projection.hull = Some(receipt.hull);
        projection.excursion = None;
        projection.site = None;
        // The S.3 fix round's R3: `Bay` in the rig's own hull's bay only — a rig a second
        // carrier brought in reads `Out`, as the tick's `projected_state` reads a foreign bay.
        projection.state = match act::where_is(ctx.w, record.tv) {
            Where::Bay { hull } if hull == receipt.hull => TvState::Bay,
            Where::Bay { .. } => TvState::Out,
            Where::Ground { .. } | Where::Elsewhere => TvState::Parked,
        };
        projection.pose = crate::surface::tick::pose_of(ctx.w, record.tv);
        projection.act = None;
        projection.leg = None;
        projection.run = None;
        projection.plan = None;
        projection.plan_told = None;
        projection.applied = receipt.ordinal;
        projection.told = receipt.ordinal;
        projection.told_cycle = 0;
    }
    projection.recovery_told = Some(receipt.ordinal);
    projection.recovery_wait_told = None;
    if projection
        .operator
        .is_none_or(|operator| operator == receipt.operator)
    {
        for line in &lines {
            projection.push_log(receipt.at_ic, line);
        }
    }
    if !crate::surface::tick::write_projection(ctx, &projection) {
        return false;
    }
    for line in &lines {
        crate::port_tick::notify(ctx, receipt.staff, line);
        if receipt.operator != receipt.staff {
            crate::port_tick::notify(ctx, receipt.operator, line);
        }
    }
    true
}
