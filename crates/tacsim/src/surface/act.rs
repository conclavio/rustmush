//! **The acts: their world plans, their start and their finish** (doc 20 cut SM.3b; §1 beats 4
//! and 9, §4.1, §4.3, §6's authoritative-start shape, §8, §10; D-20-6, D-20-7, D-20-11; the
//! SM.3b brief's 3.3, 3.4 and 3.6 and its lead rulings 2, 3, 5 and 8).
//!
//! One module holds what the command, the two offers and the tick all read, so they cannot
//! drift:
//!
//! - **the world plans** — `launch_plan`,
//!   `recover_plan`, `dump_plan` and the test world's `probe_plan`: each act's whole
//!   world-side gate in a fixed order, pure reads of the `World`, used by `+tv`'s preview,
//!   by its `go` and by the start's offer. **SM.4 prepends `launch_plan`** for the launch its
//!   first scan, sound or drive from the bay implies (lead ruling 5);
//! - **`tacsim_surface_act`** — every act's **start**, submitted by `+tv/<n> <act> go` as the
//!   operator. Its offer holds the world and passes server-authored fixed parameters (§6:
//!   every rig's plans, the battery's **id**, the world's rigs); its prepare writes the rig's
//!   `tacsim.excursion` record — the act, its due instant, the charge reserved for it under
//!   the 5 % reserve rule (§4.1, enforced here and nowhere else, lead ruling 3), the caps
//!   (§8, lead ruling 8) and a dump's frozen hopper quantities — in one bundle;
//! - **`tacsim_surface_done`** — every act's **finish**, submitted by the `tacsim.surface`
//!   tick as the operator (`crate::surface::tick`) once it has written the placement. Its
//!   offer reads the world **at landing** (decision 5) and its prepare records `Done` or
//!   `Refused`, clears the act, settles the charge lazily at the act's own `due_ic` (lead
//!   ruling 2) and, for a dump, moves the frozen pools to the site's spoil vault through the
//!   delegated inventory families — conserved, no mint, no lot (§4.3).
//!
//! **The control lease, read as the lessee aboard** (the brief's decision 14, a deviation
//! recorded for the owner): doc 20 §4.1, §8 and §19 item 3 speak of "the operator's control
//! lease"; SM.3b reads it as the live lessee (`TvLease.pilot`) aboard the settled hull — the
//! teleoperation from the bridge §4.1 describes — and takes no mush-sim control lease on the
//! TV (its cab has no station to grant). It applies to every act alike.
//!
//! Neither provider ever reads `World::now`: `now` is the offer's IC instant.

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, ActionTxn,
    AuthoritativeActionOutput, CanonicalId, CmdError, Ctx, DefinitionRef, IdempotencyKeyText,
    ObjId, PerceivedRef, ProjectedParameter, ProjectedParameterKind, ProjectedParameterSchema,
    ProjectedTargetSchema, ProviderOfferDraft, RevisionSet, RulesValue, StateFamilyHandle,
    SubjectKey, World,
};
use mush_economy::VaultId;
use mush_inventory::InventoryFamilies;
use mush_inventory::record::EndpointRef;
use mush_sim::data::{Pose, SpatialParent, SpatialState};
use mush_sim::{DomainId, SlotId};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::definitions::tools::{BatteryDef, ToolDef};
use crate::port::coop::{name_of, port_name};
use crate::port::def::PortDef;
use crate::port::families::PortHandles;
use crate::port::map::PortMap;
use crate::port::ship::{Berth, HullView, berth_of, hull_view};
use crate::port::tv::{self, TvLease, TvLeaseStatus};
use crate::port_tick::port_error;
use crate::store::TacsimStore;
use crate::surface::excursion::{
    Act, ActKind, ActParams, ActResult, CapRefused, Charge, DUMP_S, Excursion, ExcursionCounts,
    ExcursionId, LAUNCH_S, MAX_DUMP_GOODS, Outcome, RECOVER_S, RefusedCode, RigKey, RigRecord,
    cap_refusal, cells_apart, charge_at, needs_battery, ramp_pose, reserve_check, reserved_total,
    settle_free, site_cell,
};

/// The start's action id (lead ruling 3, "the planner's call"; doc 20 §6 names the mining
/// start `tacsim_surface_mine` — it becomes `act = mine` of this action in SM.7, the brief's
/// decision 25).
pub const SURFACE_ACT_ACTION: &str = "tacsim_surface_act";
/// The start's outcome id.
pub const SURFACE_ACT_OUTCOME: &str = "tacsim_surface_act_result";
/// The finish's action id.
pub const SURFACE_DONE_ACTION: &str = "tacsim_surface_done";
/// The finish's outcome id.
pub const SURFACE_DONE_OUTCOME: &str = "tacsim_surface_done_result";
/// Both actions' idempotency namespace: the start is `tacsim.surface/{tv}/{ordinal}/act`,
/// the finish `tacsim.surface/{tv}/{ordinal}/done`.
pub const SURFACE_KEY_PREFIX: &str = "tacsim.surface/";
/// The most a test-world probe may cost, cu (its parameter's bound).
pub const MAX_PROBE_CU: i64 = 1_000_000;
/// The longest a test-world probe may run, IC seconds (its parameter's bound).
pub const MAX_PROBE_S: i64 = 86_400;

/// The start's key: `tacsim.surface/{tv}/{ordinal}/act`.
#[must_use]
pub fn act_key(tv: ObjId, ordinal: u32) -> Option<IdempotencyKeyText> {
    IdempotencyKeyText::parse_canonical(format!("{SURFACE_KEY_PREFIX}{}/{ordinal}/act", tv.0)).ok()
}

/// The finish's key: `tacsim.surface/{tv}/{ordinal}/done`.
#[must_use]
pub fn done_key(tv: ObjId, ordinal: u32) -> Option<IdempotencyKeyText> {
    IdempotencyKeyText::parse_canonical(format!("{SURFACE_KEY_PREFIX}{}/{ordinal}/done", tv.0)).ok()
}

// ---- the actor's rigs and where a rig is ----------------------------------------------

/// **One of an operator's rigs** (the brief's 3.3): a live lease record of the actor's on
/// the actor's helm hull.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RigAt {
    /// The terrain vehicle.
    pub tv: ObjId,
    /// Its bay slot.
    pub slot: u8,
    /// Its registry mark.
    pub mark: String,
    /// The hull it is leased into.
    pub hull: ObjId,
    /// Its live lease record.
    pub record: TvLease,
}

/// **The actor's rigs** (the brief's 3.3): the actor's live lease records
/// (`TacsimStore::tv_leases_of`, `live()`) whose hull is the actor's helm hull
/// (`crate::port::coop::helm_of`), by slot.
#[must_use]
pub(crate) fn operator_rigs(w: &World, actor: ObjId) -> Vec<RigAt> {
    let Some(helm) = crate::port::coop::helm_of(w, actor) else {
        return Vec::new();
    };
    let Some(store) = w.module_state::<TacsimStore>() else {
        return Vec::new();
    };
    let mut rigs: Vec<RigAt> = store
        .tv_leases_of(actor)
        .filter(|record| record.live() && record.hull == helm.hull)
        .map(|record| RigAt {
            tv: record.tv,
            slot: record.slot,
            mark: record.mark.clone(),
            hull: record.hull,
            record: record.clone(),
        })
        .collect();
    rigs.sort_by_key(|rig| (rig.slot, rig.tv));
    rigs
}

/// **Where a rig stands** (the brief's 3.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Where {
    /// `Interior` in some programme hull's bay room.
    Bay {
        /// That hull.
        hull: ObjId,
    },
    /// `Direct` on a domain.
    Ground {
        /// The domain.
        domain: DomainId,
        /// East, centimetres.
        x_cm: i64,
        /// North, centimetres.
        y_cm: i64,
        /// Heading, millidegrees.
        heading_mdeg: u32,
    },
    /// Anywhere else (a shed, a staff move).
    Elsewhere,
}

/// **Where `tv` stands** (the brief's 3.3): `Interior` in a programme hull's bay room
/// (`crate::port::vehicle::bay_rooms` over the programme port's hull rows) is `Bay`;
/// `Direct` is `Ground`; anything else `Elsewhere`.
#[must_use]
pub(crate) fn where_is(w: &World, tv: ObjId) -> Where {
    let Some(st) = w.module_state::<SpatialState>() else {
        return Where::Elsewhere;
    };
    match st.parent_of(tv) {
        SpatialParent::Interior { space, .. } => match crate::port::ship::hull_of_room(w, space) {
            Some((view, _)) if crate::port::vehicle::bay_rooms(w, &view).contains(&space) => {
                Where::Bay { hull: view.hull() }
            }
            _ => Where::Elsewhere,
        },
        SpatialParent::Direct { domain, pose } => Where::Ground {
            domain,
            x_cm: pose.x_cm,
            y_cm: pose.y_cm,
            heading_mdeg: pose.heading_mdeg,
        },
        SpatialParent::Attached { .. } | SpatialParent::Unplaced => Where::Elsewhere,
    }
}

/// **A hull settled on a site's pad, resolved** (the brief's 3.3; decision 3).
#[derive(Clone, Debug)]
pub(crate) struct SiteBerth<'w> {
    /// The site's port entry.
    pub entry: CanonicalId,
    /// The site's name (`port_name`).
    pub name: String,
    /// The site's port entry definition.
    #[allow(dead_code)] // SM.4's reader
    pub def: &'w PortDef,
    /// The site's port map.
    pub map: &'w PortMap,
    /// The site's passive domain (the hull's own placement's).
    pub domain: DomainId,
    /// The ramp as a site pose (`ramp_pose`).
    pub ramp: Pose,
    /// The egress's label (`stern ramp`).
    pub ramp_label: String,
    /// The hull's bay room.
    pub bay_room: ObjId,
    /// The bay's compartment.
    pub bay_slot: SlotId,
}

/// **The hull's site berth** (the brief's 3.3): `Docked` at a port whose `def.site` is
/// `Some`, the domain from the hull's own placement, the ramp from its bay's egress.
///
/// # Errors
///
/// [`Refusal::UnderWay`] under way; [`Refusal::NotSite`] at a port, elsewhere or unplaced;
/// [`Refusal::Dark`] when the bay or its ramp cannot be read.
pub(crate) fn site_berth<'w>(w: &'w World, view: &HullView<'w>) -> Result<SiteBerth<'w>, Refusal> {
    let hull = view.hull();
    let name = name_of(w, hull);
    let (map, def) = match berth_of(w, hull) {
        Berth::UnderWay => return Err(Refusal::UnderWay { name }),
        Berth::Docked { map, def } if def.site.is_some() => (map, def),
        Berth::Docked { .. } | Berth::Elsewhere => return Err(Refusal::NotSite { name }),
    };
    let Some((domain, _)) = mush_sim::pose_of(w, hull) else {
        return Err(Refusal::NotSite { name });
    };
    let Some((bay, bay_room)) = view.bay() else {
        tracing::warn!(hull = hull.0, "tacsim: a rig's hull has no readable bay");
        return Err(Refusal::Dark { name });
    };
    let ramp = ramp_pose(bay).map_err(|error| {
        tracing::warn!(hull = hull.0, %error, "tacsim: a bay's ramp does not read as a site pose");
        Refusal::Dark { name: name.clone() }
    })?;
    let bay_slot =
        SlotId::new(bay.slot.id.as_str()).ok_or_else(|| Refusal::Dark { name: name.clone() })?;
    Ok(SiteBerth {
        entry: map.entry.clone(),
        name: port_name(def),
        def,
        map,
        domain,
        ramp,
        ramp_label: bay.egress.label.clone(),
        bay_room,
        bay_slot,
    })
}

/// The hull's bay's egress label (`stern ramp`), else `ramp`.
#[must_use]
pub(crate) fn ramp_label(w: &World, hull: ObjId) -> String {
    hull_view(w, hull)
        .and_then(|view| view.bay().map(|(bay, _)| bay.egress.label.clone()))
        .unwrap_or_else(|| "ramp".to_owned())
}

/// A site port entry's name (`port_name`), else the entry id.
#[must_use]
pub(crate) fn site_name(w: &World, entry: &CanonicalId) -> String {
    let Some(store) = w.module_state::<TacsimStore>() else {
        return entry.to_string();
    };
    let (_, ports) = crate::port::readiness::report_ports(store, w);
    ports
        .into_iter()
        .find(|(id, _)| id == entry)
        .map_or_else(|| entry.to_string(), |(_, def)| port_name(def))
}

/// The site port entry whose domain is `domain`, when one is.
#[must_use]
pub(crate) fn site_of_domain(w: &World, domain: DomainId) -> Option<CanonicalId> {
    let store = w.module_state::<TacsimStore>()?;
    let map = store.port_maps().find(|map| {
        map.domain == Some(domain.0) && map.pack.as_str() == crate::definitions::content::LANE_PACK
    })?;
    let (_, ports) = crate::port::readiness::report_ports(store, w);
    ports
        .into_iter()
        .find(|(id, def)| *id == map.entry && def.site.is_some())
        .map(|(id, _)| id)
}

// ---- the refusals ---------------------------------------------------------------------------

/// **Why an act refuses** (the brief's 3.3 and 3.6): one variant per public line, each
/// carrying the words its line names; [`Refusal::line`] renders it, [`Refusal::word`] is its
/// code on an offer's wire and [`Refusal::code`] the start's `port_error` message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// `TV_DARK`.
    Dark {
        /// The hull's name.
        name: String,
    },
    /// `TV_UNWIRED`.
    Unwired,
    /// `TV_NOT_ABOARD`.
    NotAboard {
        /// The hull's name.
        name: String,
    },
    /// `LAUNCH_NOT_SITE`.
    NotSite {
        /// The hull's name.
        name: String,
    },
    /// `LAUNCH_UNDER_WAY`.
    UnderWay {
        /// The hull's name.
        name: String,
    },
    /// `LAUNCH_CLEARED`.
    Cleared {
        /// The hull's name.
        name: String,
        /// The destination's name.
        port: String,
    },
    /// `LAUNCH_OUT`.
    Out {
        /// The slot.
        n: u8,
    },
    /// `LAUNCH_OPEN`.
    Open {
        /// The slot.
        n: u8,
    },
    /// `LAUNCH_FORFEIT`.
    Forfeit {
        /// The slot.
        n: u8,
        /// Occator's counter.
        counter: String,
        /// Its port.
        port: String,
    },
    /// `LAUNCH_ENDED`.
    Ended {
        /// The slot.
        n: u8,
        /// Occator's counter.
        counter: String,
        /// Its port.
        port: String,
    },
    /// `RECOVER_IN_BAY`.
    InBay {
        /// The slot.
        n: u8,
    },
    /// `RECOVER_ELSEWHERE`.
    Elsewhere {
        /// The slot.
        n: u8,
        /// The hull's name.
        name: String,
    },
    /// `RECOVER_FAR`.
    Far {
        /// The slot.
        n: u8,
        /// Metres from the ramp (cells × 10).
        m: u64,
        /// The egress's label.
        ramp: String,
    },
    /// `DUMP_NOT_SITE`.
    DumpNotSite {
        /// The slot.
        n: u8,
        /// The hull's name.
        name: String,
    },
    /// `DUMP_NO_SPOIL`.
    NoSpoil {
        /// The site's name.
        site: String,
    },
    /// `DUMP_EMPTY`.
    Empty {
        /// The slot.
        n: u8,
    },
    /// `RIG_BUSY`.
    Busy {
        /// The slot.
        n: u8,
        /// What it is doing.
        doing: String,
        /// When it is done.
        when: String,
    },
    /// `RESERVE_LOW`.
    Reserve {
        /// The slot.
        n: u8,
        /// The charge now, whole percent.
        now: u32,
        /// What the act would leave, whole percent.
        after: u32,
        /// The floor, whole percent.
        floor: u32,
    },
    /// `CAP_OPERATOR`, `CAP_SITE` or `CAP_WORLD`.
    Cap {
        /// Which cap.
        cap: CapRefused,
        /// The site's name.
        site: String,
    },
}

/// The offer-wire words and the start's messages, in one table (the prepare's
/// `port_error` needs a `&'static str`).
const REFUSAL_CODES: [(&str, &str); 19] = [
    ("tv_dark", "that rig cannot be read"),
    ("tv_unwired", "rigs cannot be worked on this server"),
    ("not_aboard", "the operator is not aboard the hull"),
    ("not_site", "the hull is not settled on a site's pad"),
    ("under_way", "the hull is under way"),
    ("cleared", "the hull is cleared to leave"),
    ("out", "that rig is already out of the bay"),
    ("open", "that rig has an excursion open"),
    ("forfeit", "that rig's deposit is forfeit"),
    ("ended", "that rig's lease has ended"),
    ("in_bay", "that rig is in the bay"),
    ("elsewhere", "that rig is not beside the hull"),
    ("far", "that rig is too far from the ramp"),
    ("dump_not_site", "the hull is not settled on a site's pad"),
    ("no_spoil", "the site has no spoil laid out"),
    ("empty", "the hopper is empty"),
    ("busy", "that rig is busy"),
    ("reserve", "the rig's charge is under the reserve"),
    ("cap", "that is as many rigs out as may be"),
];

/// The start's message for an offer-wire word; an unknown word reads as dark.
#[must_use]
pub(crate) fn code_of_word(word: &str) -> &'static str {
    REFUSAL_CODES
        .iter()
        .find(|(w, _)| *w == word)
        .map_or("that rig cannot be read", |(_, message)| message)
}

impl Refusal {
    /// The offer-wire word.
    #[must_use]
    pub fn word(&self) -> &'static str {
        match self {
            Refusal::Dark { .. } => "tv_dark",
            Refusal::Unwired => "tv_unwired",
            Refusal::NotAboard { .. } => "not_aboard",
            Refusal::NotSite { .. } => "not_site",
            Refusal::UnderWay { .. } => "under_way",
            Refusal::Cleared { .. } => "cleared",
            Refusal::Out { .. } => "out",
            Refusal::Open { .. } => "open",
            Refusal::Forfeit { .. } => "forfeit",
            Refusal::Ended { .. } => "ended",
            Refusal::InBay { .. } => "in_bay",
            Refusal::Elsewhere { .. } => "elsewhere",
            Refusal::Far { .. } => "far",
            Refusal::DumpNotSite { .. } => "dump_not_site",
            Refusal::NoSpoil { .. } => "no_spoil",
            Refusal::Empty { .. } => "empty",
            Refusal::Busy { .. } => "busy",
            Refusal::Reserve { .. } => "reserve",
            Refusal::Cap { .. } => "cap",
        }
    }

    /// The start's `port_error` message: a short lowercase sentence.
    #[must_use]
    pub fn code(&self) -> &'static str {
        code_of_word(self.word())
    }

    /// The line the player hears.
    #[must_use]
    pub fn line(&self) -> String {
        match self {
            Refusal::Dark { name } => tv::TV_DARK.replace("{name}", name),
            Refusal::Unwired => tv::TV_UNWIRED.to_owned(),
            Refusal::NotAboard { name } => tv::TV_NOT_ABOARD.replace("{name}", name),
            Refusal::NotSite { name } => tv::LAUNCH_NOT_SITE.replace("{name}", name),
            Refusal::UnderWay { name } => tv::LAUNCH_UNDER_WAY.replace("{name}", name),
            Refusal::Cleared { name, port } => tv::LAUNCH_CLEARED
                .replace("{name}", name)
                .replace("{port}", port),
            Refusal::Out { n } => tv::LAUNCH_OUT.replace("{n}", &n.to_string()),
            Refusal::Open { n } => tv::LAUNCH_OPEN.replace("{n}", &n.to_string()),
            Refusal::Forfeit { n, counter, port } => tv::LAUNCH_FORFEIT
                .replace("{n}", &n.to_string())
                .replace("{counter}", counter)
                .replace("{port}", port),
            Refusal::Ended { n, counter, port } => tv::LAUNCH_ENDED
                .replace("{n}", &n.to_string())
                .replace("{counter}", counter)
                .replace("{port}", port),
            Refusal::InBay { n } => tv::RECOVER_IN_BAY.replace("{n}", &n.to_string()),
            Refusal::Elsewhere { n, name } => tv::RECOVER_ELSEWHERE
                .replace("{n}", &n.to_string())
                .replace("{name}", name),
            Refusal::Far { n, m, ramp } => tv::RECOVER_FAR
                .replace("{n}", &n.to_string())
                .replace("{m}", &m.to_string())
                .replace("{ramp}", ramp),
            Refusal::DumpNotSite { n, name } => tv::DUMP_NOT_SITE
                .replace("{n}", &n.to_string())
                .replace("{name}", name),
            Refusal::NoSpoil { site } => tv::DUMP_NO_SPOIL.replace("{site}", site),
            Refusal::Empty { n } => tv::DUMP_EMPTY.replace("{n}", &n.to_string()),
            Refusal::Busy { n, doing, when } => tv::RIG_BUSY
                .replace("{n}", &n.to_string())
                .replace("{doing}", doing)
                .replace("{when}", when),
            Refusal::Reserve {
                n,
                now,
                after,
                floor,
            } => tv::RESERVE_LOW
                .replace("{n}", &n.to_string())
                .replace("{now}", &now.to_string())
                .replace("{after}", &after.to_string())
                .replace("{floor}", &floor.to_string()),
            Refusal::Cap { cap, site } => {
                let template = match cap {
                    CapRefused::Operator => tv::CAP_OPERATOR,
                    CapRefused::Site => tv::CAP_SITE,
                    CapRefused::World => tv::CAP_WORLD,
                };
                template
                    .replace("{cap}", &cap.cap().to_string())
                    .replace("{site}", site)
            }
        }
    }
}

// ---- the world plans --------------------------------------------------------------------------

/// **One act, planned against the world** (the brief's 3.3): what the preview prints, what
/// the `go` submits and what the start's offer fixes.
#[derive(Clone, Debug)]
pub(crate) struct ActPlan {
    /// What act.
    #[allow(dead_code)] // SM.4's chaining reads it (the implied launch's plan)
    pub kind: ActKind,
    /// The rig.
    #[allow(dead_code)] // SM.4's chaining reads it (the implied launch's plan)
    pub rig: RigAt,
    /// The site entry the hull is settled at.
    pub site: CanonicalId,
    /// How long it takes, IC seconds.
    pub secs: u32,
    /// What it spends, cu.
    pub cost_cu: u32,
    /// What the trip home spends, cu (0 for every SM.3b act; SM.4's drive adds it).
    pub home_cu: u32,
    /// Its parameters (a dump's goods are read later: the command by `read_pool`, the
    /// prepare by the transaction).
    pub params: ActParams,
    /// Read only by a recover: the rig already stands in this hull's bay (decision 21).
    pub from_bay: bool,
}

/// The common head of every plan: the layer on, the hull's view readable and not held.
fn view_of<'w>(w: &'w World, rig: &RigAt) -> Result<(HullView<'w>, String), Refusal> {
    let name = name_of(w, rig.hull);
    let on = w
        .module_state::<TacsimStore>()
        .is_some_and(TacsimStore::enabled);
    if !on {
        return Err(Refusal::Dark { name });
    }
    let Some(view) = hull_view(w, rig.hull) else {
        tracing::warn!(
            hull = rig.hull.0,
            "tacsim: a rig's hull has no programme view"
        );
        return Err(Refusal::Dark { name });
    };
    if view.unit_held {
        tracing::warn!(
            hull = rig.hull.0,
            "tacsim: a rig's hull's unit record is held"
        );
        return Err(Refusal::Dark { name });
    }
    Ok((view, name))
}

/// **The launch's whole world-side gate, in order** (the brief's 3.3; §4.3: "legal only
/// while the hull is settled at a site and the operator is aboard"): the layer on; the hull's
/// view (dark or held → `TV_DARK`); under way → `LAUNCH_UNDER_WAY`; an unreadable sailing
/// plan → `TV_DARK`, logged (finding F11: fail-closed, as `prepare_departure` refuses
/// `VoyageState::Unreadable`); a clearance filed → `LAUNCH_CLEARED`; not settled at a site →
/// `LAUNCH_NOT_SITE`; the actor not aboard → `TV_NOT_ABOARD`; the lease record `Forfeit` →
/// `LAUNCH_FORFEIT`; its economy lease not live → `LAUNCH_ENDED` (SM.3a's operate gate R7;
/// an Active lease past its paid term still launches, decision 11); the rig not in this hull's
/// bay → `LAUNCH_OUT`. `LAUNCH_S`, no cost, no trip home.
///
/// **SM.4 prepends this** for the launch its first scan, sound or drive from the bay implies
/// (lead ruling 5): keep the refusal order stable.
///
/// # Errors
///
/// The first [`Refusal`] in that order.
pub(crate) fn launch_plan(w: &World, actor: ObjId, rig: &RigAt) -> Result<ActPlan, Refusal> {
    let (view, name) = view_of(w, rig)?;
    if matches!(berth_of(w, rig.hull), Berth::UnderWay) {
        return Err(Refusal::UnderWay { name });
    }
    if crate::port::voyage::voyage_dark(w, rig.hull) {
        tracing::warn!(
            hull = rig.hull.0,
            "tacsim: a launch found the hull's sailing plan unreadable; it is refused"
        );
        return Err(Refusal::Dark { name });
    }
    if let Some(pending) = crate::port::voyage::pending_of(w, rig.hull) {
        return Err(Refusal::Cleared {
            name,
            port: crate::port::voyage::venue_name(w, &pending.to_venue),
        });
    }
    let berth = site_berth(w, &view)?;
    if !crate::port::voyage::aboard(w, rig.hull, actor) {
        return Err(Refusal::NotAboard { name });
    }
    let (counter, port) = tv::counter_words(w);
    if rig.record.status == TvLeaseStatus::Forfeit {
        return Err(Refusal::Forfeit {
            n: rig.slot,
            counter,
            port,
        });
    }
    if !tv::rig_lease_live(w, &rig.record) {
        return Err(Refusal::Ended {
            n: rig.slot,
            counter,
            port,
        });
    }
    if where_is(w, rig.tv) != (Where::Bay { hull: rig.hull }) {
        return Err(Refusal::Out { n: rig.slot });
    }
    Ok(ActPlan {
        kind: ActKind::Launch,
        rig: rig.clone(),
        site: berth.entry,
        secs: LAUNCH_S,
        cost_cu: 0,
        home_cu: 0,
        params: ActParams::None {},
        from_bay: true,
    })
}

/// **The recover's world-side gate, in order** (the brief's 3.3; §7.1's placement half): the
/// layer, the view, under way, not at a site, not aboard as the launch; then the rig: in this
/// hull's bay → `Ok` with `from_bay` (the command and the prepare refuse `RECOVER_IN_BAY`
/// unless an excursion is still open, which this recover closes — decision 21); on another
/// domain, another hull's bay or elsewhere → `RECOVER_ELSEWHERE`; on the hull's site domain
/// more than one cell from the ramp → `RECOVER_FAR` naming `cells × 10` m (the drive to the
/// ramp is SM.4's). No lease-state gate (§8 / §19 item 3: rights never stop a rig's way
/// home). `RECOVER_S` either way (one rule), no cost.
///
/// # Errors
///
/// The first [`Refusal`] in that order.
pub(crate) fn recover_plan(w: &World, actor: ObjId, rig: &RigAt) -> Result<ActPlan, Refusal> {
    let (view, name) = view_of(w, rig)?;
    if matches!(berth_of(w, rig.hull), Berth::UnderWay) {
        return Err(Refusal::UnderWay { name });
    }
    let berth = site_berth(w, &view)?;
    if !crate::port::voyage::aboard(w, rig.hull, actor) {
        return Err(Refusal::NotAboard { name });
    }
    let from_bay = match where_is(w, rig.tv) {
        Where::Bay { hull } if hull == rig.hull => true,
        Where::Ground {
            domain, x_cm, y_cm, ..
        } if domain == berth.domain => {
            let cells = cells_apart(
                site_cell(x_cm, y_cm),
                site_cell(berth.ramp.x_cm, berth.ramp.y_cm),
            );
            if cells > 1 {
                return Err(Refusal::Far {
                    n: rig.slot,
                    m: cells.saturating_mul(10),
                    ramp: berth.ramp_label,
                });
            }
            false
        }
        Where::Bay { .. } | Where::Ground { .. } | Where::Elsewhere => {
            return Err(Refusal::Elsewhere { n: rig.slot, name });
        }
    };
    Ok(ActPlan {
        kind: ActKind::Recover,
        rig: rig.clone(),
        site: berth.entry,
        secs: RECOVER_S,
        cost_cu: 0,
        home_cu: 0,
        params: ActParams::None {},
        from_bay,
    })
}

/// **The dump's world-side gate, in order** (the brief's 3.3; §4.3): the layer, the view,
/// under way; not at a site → `DUMP_NOT_SITE`; not aboard; the rig neither in this hull's
/// bay nor on its site's domain → `RECOVER_ELSEWHERE`; the site's spoil vault not laid →
/// `DUMP_NO_SPOIL`. No lease gate. `params` names the hopper and the spoil; the goods are
/// read later (the command by `read_pool`, the prepare by its transaction).
///
/// # Errors
///
/// The first [`Refusal`] in that order.
pub(crate) fn dump_plan(w: &World, actor: ObjId, rig: &RigAt) -> Result<ActPlan, Refusal> {
    let (view, name) = view_of(w, rig)?;
    if matches!(berth_of(w, rig.hull), Berth::UnderWay) {
        return Err(Refusal::UnderWay { name });
    }
    let berth = site_berth(w, &view).map_err(|refusal| match refusal {
        Refusal::NotSite { name } => Refusal::DumpNotSite { n: rig.slot, name },
        other => other,
    })?;
    if !crate::port::voyage::aboard(w, rig.hull, actor) {
        return Err(Refusal::NotAboard { name });
    }
    match where_is(w, rig.tv) {
        Where::Bay { hull } if hull == rig.hull => {}
        Where::Ground { domain, .. } if domain == berth.domain => {}
        Where::Bay { .. } | Where::Ground { .. } | Where::Elsewhere => {
            return Err(Refusal::Elsewhere { n: rig.slot, name });
        }
    }
    let Some(spoil) = crate::port::site::spoil_vault(w, berth.map) else {
        return Err(Refusal::NoSpoil { site: berth.name });
    };
    let Some(hopper) = crate::port::vehicle::hopper_of(w, rig.tv) else {
        tracing::warn!(tv = rig.tv.0, "tacsim: a dump found no hopper vault");
        return Err(Refusal::Dark { name });
    };
    Ok(ActPlan {
        kind: ActKind::Dump,
        rig: rig.clone(),
        site: berth.entry,
        secs: DUMP_S,
        cost_cu: 0,
        home_cu: 0,
        params: ActParams::Dump {
            hopper: hopper.0,
            spoil: spoil.0,
            goods: BTreeMap::new(),
        },
        from_bay: false,
    })
}

/// **The test world's probe** (the brief's 3.7): the layer, the view, at a site, aboard.
///
/// # Errors
///
/// The first [`Refusal`].
pub(crate) fn probe_plan(
    w: &World,
    actor: ObjId,
    rig: &RigAt,
    cost_cu: u32,
    secs: u32,
) -> Result<ActPlan, Refusal> {
    let (view, name) = view_of(w, rig)?;
    let berth = site_berth(w, &view)?;
    if !crate::port::voyage::aboard(w, rig.hull, actor) {
        return Err(Refusal::NotAboard { name });
    }
    Ok(ActPlan {
        kind: ActKind::Probe,
        rig: rig.clone(),
        site: berth.entry,
        secs,
        cost_cu,
        home_cu: 0,
        params: ActParams::Probe { cost_cu },
        from_bay: false,
    })
}

/// **Which battery a rig carries** (finding F5): the lessor pool's tools
/// (`VehiclesDef.tools`) whose row in the one tools table (`PortHandles.surface_tools`) is a
/// battery — the world names **which**; the numbers always come from the server-held table.
#[must_use]
pub(crate) fn rig_battery_id(w: &World, rig_tv: ObjId) -> Option<CanonicalId> {
    crate::port::vehicle::rig_row(w, rig_tv)?;
    let tools = w.module_state::<PortHandles>()?.surface_tools.clone()?;
    let port = crate::port::coop::programme_port(w)?;
    let pool = port.def.vehicles.as_deref()?;
    pool.tools
        .iter()
        .find(|id| tools.get(*id).and_then(ToolDef::battery).is_some())
        .cloned()
}

/// **A battery's figures** by its id, from a tools table (`ToolDef::battery`).
#[must_use]
pub(crate) fn battery_of<'t>(
    tools: &'t BTreeMap<CanonicalId, ToolDef>,
    id: &CanonicalId,
) -> Option<&'t BatteryDef> {
    tools.get(id).and_then(ToolDef::battery)
}

/// **Every rig of every lessor's pool** (the caps' census, lead ruling 8): each port map's
/// rig rows, bounded by `MAX_VEHICLE_POOL` a lessor.
#[must_use]
pub(crate) fn world_rigs(w: &World) -> Vec<ObjId> {
    w.module_state::<TacsimStore>()
        .map(|store| {
            store
                .port_maps()
                .filter_map(|map| map.vehicles.as_ref())
                .flat_map(|row| row.rigs.iter().map(|rig| ObjId(rig.tv)))
                .collect()
        })
        .unwrap_or_default()
}

/// Whole percent of a battery: `⌊cu × 100 / capacity⌋`.
#[must_use]
pub(crate) fn percent(cu: u32, battery: &BatteryDef) -> u32 {
    if battery.capacity_cu == 0 {
        return 0;
    }
    u32::try_from(u64::from(cu).saturating_mul(100) / u64::from(battery.capacity_cu))
        .unwrap_or(u32::MAX)
}

/// **What an act in flight is doing** (`RIG_BUSY`'s and `ACT_LINE`'s `{doing}`).
#[must_use]
pub(crate) fn doing_words(kind: ActKind, ramp: &str) -> String {
    match kind {
        ActKind::Launch => format!("going down the {ramp}"),
        ActKind::Recover => format!("climbing the {ramp}"),
        ActKind::Dump => "dumping its hopper".to_owned(),
        ActKind::Probe => "running a test".to_owned(),
    }
}

// ---- the command: preview and go ------------------------------------------------------------

/// The typed record a command reads: `ctx.action_state` on the family, absent as fresh
/// ([`RigRecord::fresh_unread`] when the battery cannot be read, the start's rule).
fn record_of(
    ctx: &Ctx<'_>,
    handles: &PortHandles,
    tv: ObjId,
    battery: Option<&BatteryDef>,
) -> Result<RigRecord, ()> {
    match ctx.action_state(&handles.families.excursions, &RigKey(tv)) {
        Ok(Some(record)) => Ok(record.clone()),
        Ok(None) => Ok(battery.map_or_else(
            || RigRecord::fresh_unread(tv),
            |battery| RigRecord::fresh(tv, battery),
        )),
        Err(error) => {
            tracing::warn!(tv = tv.0, %error, "tacsim: a rig's excursion record does not read");
            Err(())
        }
    }
}

/// **The goods a rig's dump would move now**: each of the hopper's accepted goods' pool's
/// **available** quantity (`crate::port_tick::read_available`: the balance less every live
/// hold) — the very figure the start's prepare freezes, so the preview names exactly what `go`
/// commits (D-20-9; the SM.3b fixer round's item 9).
fn hopper_contents(ctx: &Ctx<'_>, tv: ObjId) -> Result<BTreeMap<CanonicalId, u64>, String> {
    let vault = crate::port::vehicle::hopper_of(ctx.w, tv)
        .ok_or_else(|| format!("rig #{} has no hopper vault", tv.0))?;
    let (accepted, _) = tv::hopper_goods_of(ctx.w, tv)
        .ok_or_else(|| format!("rig #{}'s unit cannot be read", tv.0))?;
    let mut out = BTreeMap::new();
    for good in accepted {
        let qty = crate::port_tick::read_available(ctx, EndpointRef(vault), &good)
            .map_err(|error| format!("rig #{}'s hopper {good}: {error:?}", tv.0))?;
        if qty > 0 {
            out.insert(good, qty);
        }
    }
    Ok(out)
}

/// `{load}`: `"{q} t of {good label}"` joined `", "`.
#[must_use]
pub(crate) fn load_words(w: &World, goods: &BTreeMap<CanonicalId, u64>) -> String {
    let table = w.module_state::<TacsimStore>().and_then(TacsimStore::goods);
    goods
        .iter()
        .map(|(good, qty)| {
            let label = table
                .and_then(|table| table.good(good))
                .map_or_else(|| good.to_string(), |def| def.label.clone());
            format!("{qty} t of {label}")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// **`+tv/<n> <act> [go]`** (the brief's 3.6; Q-20-8, D-20-9's `go` rule): the act's planner,
/// then the typed record (unreadable → `TV_DARK`), mirroring every refusal of the prepare it
/// can read in the prepare's order (finding F10: `RIG_BUSY`, `LAUNCH_OPEN`,
/// `RECOVER_IN_BAY`, the battery, the launch's caps, the reserve, the dump's hopper). Bare,
/// the preview line — nothing written, nothing queued. With `go`, ONE
/// `tacsim_surface_act` submission as the actor and the filed line; the handler writes
/// nothing else (the projection is the tick's).
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line.
#[allow(clippy::too_many_lines)] // one ordered refusal list, then the preview or the go
pub(crate) fn act_cmd(
    ctx: &mut Ctx<'_>,
    rig: &RigAt,
    kind: ActKind,
    go: bool,
) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let handles = ctx
        .w
        .module_state::<PortHandles>()
        .cloned()
        .ok_or_else(|| CmdError::msg(tv::TV_UNWIRED))?;
    let Some(action) = handles.surface_act.clone() else {
        return Err(CmdError::msg(tv::TV_UNWIRED));
    };
    let w: &World = ctx.w;
    let name = name_of(w, rig.hull);
    let dark = || CmdError::msg(tv::TV_DARK.replace("{name}", &name));
    // 1. The world plan (the start's offer runs the same function).
    let plan = match kind {
        ActKind::Launch => launch_plan(w, actor, rig),
        ActKind::Recover => recover_plan(w, actor, rig),
        ActKind::Dump => dump_plan(w, actor, rig),
        ActKind::Probe => return Err(CmdError::msg(tv::TV_USAGE)),
    }
    .map_err(|refusal| CmdError::msg(refusal.line()))?;
    // 2. The battery (finding F5: the one tools table), read only where the act needs it —
    // a spend or a launch (`needs_battery`); a recover or a dump never does, so a rig whose
    // battery row stops resolving still comes home (§4.1; the fixer round's item 3).
    let battery = handles
        .surface_tools
        .as_ref()
        .and_then(|tools| rig_battery_id(w, rig.tv).and_then(|id| battery_of(tools, &id).cloned()));
    if battery.is_none() && needs_battery(kind, plan.cost_cu, plan.home_cu) {
        return Err(dark());
    }
    // 3. The typed record, and every refusal of the prepare it can read, in its order.
    let record = record_of(ctx, &handles, rig.tv, battery.as_ref()).map_err(|()| dark())?;
    let ramp = ramp_label(w, rig.hull);
    if let Some(act) = &record.act {
        return Err(CmdError::msg(
            Refusal::Busy {
                n: rig.slot,
                doing: doing_words(act.kind, &ramp),
                when: tv::due_words(w, act.due_ic),
            }
            .line(),
        ));
    }
    if kind == ActKind::Launch && record.excursion.is_some() {
        return Err(CmdError::msg(Refusal::Open { n: rig.slot }.line()));
    }
    if kind == ActKind::Recover && plan.from_bay && record.excursion.is_none() {
        return Err(CmdError::msg(Refusal::InBay { n: rig.slot }.line()));
    }
    if kind == ActKind::Launch {
        let mut counts = ExcursionCounts::default();
        for tv in world_rigs(w) {
            let found = ctx
                .action_state(&handles.families.excursions, &RigKey(tv))
                .map_err(|_| dark())?;
            if let Some(excursion) = found.and_then(|record| record.excursion.as_ref()) {
                counts.world = counts.world.saturating_add(1);
                if excursion.site == plan.site {
                    counts.site = counts.site.saturating_add(1);
                }
                if excursion.operator == actor {
                    counts.operator = counts.operator.saturating_add(1);
                }
            }
        }
        if let Some(cap) = cap_refusal(&counts) {
            return Err(CmdError::msg(
                Refusal::Cap {
                    cap,
                    site: site_name(w, &plan.site),
                }
                .line(),
            ));
        }
    }
    let now_ic = w.world_ic_now().at;
    // `(now %, after %)`, `None` with no battery (a zero spend: the rule passes it unread).
    let charge = match &battery {
        Some(battery) => {
            let now_cu = charge_at(&record.charge, now_ic, battery);
            match reserve_check(
                now_cu,
                reserved_total(&record),
                plan.cost_cu,
                plan.home_cu,
                battery,
            ) {
                Ok(after) => Some((percent(now_cu, battery), percent(after.after_home, battery))),
                Err(short) => {
                    let after = u32::try_from(short.after_home.max(0)).unwrap_or(0);
                    return Err(CmdError::msg(
                        Refusal::Reserve {
                            n: rig.slot,
                            now: percent(now_cu, battery),
                            after: percent(after, battery),
                            floor: percent(short.floor, battery),
                        }
                        .line(),
                    ));
                }
            }
        }
        None => None,
    };
    let load = if kind == ActKind::Dump {
        let goods = hopper_contents(ctx, rig.tv).map_err(|error| {
            tracing::warn!(tv = rig.tv.0, %error, "tacsim: a dump could not read the hopper");
            dark()
        })?;
        if goods.is_empty() {
            return Err(CmdError::msg(Refusal::Empty { n: rig.slot }.line()));
        }
        load_words(w, &goods)
    } else {
        String::new()
    };
    let n = rig.slot.to_string();
    let site = site_name(w, &plan.site);
    let real = match crate::port::voyage::real_until(w, now_ic.saturating_add(i64::from(plan.secs)))
    {
        Some(real) => format!(", about {}", tv::span_words(real)),
        None => String::new(),
    };
    if !go {
        let line = match kind {
            ActKind::Launch => {
                // `needs_battery` holds for every launch, so `charge` is `Some` here.
                let Some((now, after)) = charge else {
                    return Err(dark());
                };
                tv::LAUNCH_PREVIEW
                    .replace("{n}", &n)
                    .replace("{mark}", &rig.mark)
                    .replace("{ramp}", &ramp)
                    .replace("{site}", &site)
                    .replace("{secs}", &plan.secs.to_string())
                    .replace("{real}", &real)
                    .replace("{now}", &now.to_string())
                    .replace("{after}", &after.to_string())
            }
            ActKind::Recover => {
                let hopper = match tv::hopper_load_of(ctx, rig.tv) {
                    Ok((kg, cap)) => tv::hopper_text(kg, cap),
                    Err(error) => {
                        tracing::warn!(tv = rig.tv.0, %error, "tacsim: a recover could not read the hopper");
                        return Err(dark());
                    }
                };
                match charge {
                    Some((now, after)) => tv::RECOVER_PREVIEW
                        .replace("{now}", &now.to_string())
                        .replace("{after}", &after.to_string()),
                    None => tv::RECOVER_PREVIEW_UNREAD.to_owned(),
                }
                .replace("{n}", &n)
                .replace("{mark}", &rig.mark)
                .replace("{ramp}", &ramp)
                .replace("{name}", &name)
                .replace("{secs}", &plan.secs.to_string())
                .replace("{real}", &real)
                .replace("{hopper}", &hopper)
            }
            ActKind::Dump | ActKind::Probe => tv::DUMP_PREVIEW
                .replace("{n}", &n)
                .replace("{mark}", &rig.mark)
                .replace("{load}", &load)
                .replace("{site}", &site),
        };
        crate::port::coop::say(ctx, line);
        return Ok(());
    }
    // `go`: ONE submission as the actor, keyed `tacsim.surface/{tv}/{ordinal}/act`.
    let Some(ordinal) = record.ordinal.checked_add(1) else {
        tracing::error!(tv = rig.tv.0, "tacsim: a rig's act ordinal is exhausted");
        return Err(dark());
    };
    let Some(key) = act_key(rig.tv, ordinal) else {
        tracing::error!(tv = rig.tv.0, "tacsim: a rig act's key did not form");
        return Err(dark());
    };
    let due = now_ic.saturating_add(i64::from(plan.secs));
    let when = tv::due_words(ctx.w, due);
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        Vec::new(),
        json!({
            "tv": rig.tv.0.to_string(),
            "act": kind.word(),
            "ordinal": ordinal,
        }),
        key,
    );
    let line = match kind {
        ActKind::Launch => tv::LAUNCH_FILED
            .replace("{n}", &n)
            .replace("{ramp}", &ramp)
            .replace("{when}", &when),
        ActKind::Recover => tv::RECOVER_FILED
            .replace("{n}", &n)
            .replace("{ramp}", &ramp)
            .replace("{when}", &when),
        ActKind::Dump | ActKind::Probe => tv::DUMP_FILED.replace("{n}", &n),
    };
    crate::port::coop::say(ctx, line);
    Ok(())
}

// ---- the offers' fixed parameters ------------------------------------------------------------

/// One act's plan as the start's offer fixes it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixedPlan {
    ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    site: Option<CanonicalId>,
    #[serde(default)]
    secs: u32,
    #[serde(default)]
    cost_cu: u32,
    #[serde(default)]
    home_cu: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    params: Option<ActParams>,
    #[serde(default)]
    from_bay: bool,
    /// A dump's hopper's accepted goods (the prepare reads each through its transaction).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    goods: Vec<CanonicalId>,
}

impl FixedPlan {
    fn refused(word: &str) -> FixedPlan {
        FixedPlan {
            ok: false,
            code: Some(word.to_owned()),
            ..FixedPlan::default()
        }
    }

    fn of(plan: Result<ActPlan, Refusal>) -> FixedPlan {
        match plan {
            Ok(plan) => FixedPlan {
                ok: true,
                code: None,
                site: Some(plan.site),
                secs: plan.secs,
                cost_cu: plan.cost_cu,
                home_cu: plan.home_cu,
                params: Some(plan.params),
                from_bay: plan.from_bay,
                goods: Vec::new(),
            },
            Err(refusal) => FixedPlan::refused(refusal.word()),
        }
    }
}

/// One rig as the start's offer fixes it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixedRig {
    slot: u8,
    hull: ObjId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    battery_id: Option<CanonicalId>,
    plans: BTreeMap<String, FixedPlan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    probe: Option<FixedPlan>,
}

/// The world as the finish's offer reads it at landing (decision 5).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "where", rename_all = "snake_case", deny_unknown_fields)]
enum Facts {
    /// In a programme hull's bay.
    Bay { hull: ObjId },
    /// On a domain: the site whose domain it is (if any) and the cells from that site's
    /// ramp (through the hull of the rig's live record).
    Ground {
        domain: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        site: Option<CanonicalId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cells: Option<u64>,
    },
    /// Anywhere else.
    Elsewhere,
}

/// One rig as the finish's offer fixes it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DoneRig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    battery_id: Option<CanonicalId>,
    facts: Facts,
    /// **The live spoil vault of the site the rig's hull is settled on** (`spoil_vault`: the
    /// spoil Thing live, the vault the sites' system principal's and Active), when there is
    /// one: a dump whose frozen spoil is not this one records `SpoilGone` (the fixer round's
    /// item 4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    spoil: Option<u64>,
    /// **The act ordinals of this rig whose finish was handed to the engine and did not land**
    /// (the tick's counted attempts, `crate::surface::tick::finish_retried`: ascending, at most
    /// `MAX_RETRIED_ORDINALS`). A dump's transfer that refuses mid-bundle cannot be refused
    /// inside its own bundle — `ActionTxn` keeps no savepoint and refuses a second mutation of
    /// one key, so the debit it staged would commit — so its next finish records `SpoilGone`
    /// and moves nothing (the fixer round's item 4: a failed dump transfer records a refusal,
    /// it never wedges the act). Keyed by the act (review finding F1): only the finishing act's
    /// own ordinal counts, never a stale attempt of an earlier act of the rig.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    failed: Vec<u32>,
}

/// The live spoil vault of the site `hull` is settled on (`crate::port::site::spoil_vault`).
fn spoil_of(w: &World, hull: ObjId) -> Option<u64> {
    let view = hull_view(w, hull)?;
    let berth = site_berth(w, &view).ok()?;
    crate::port::site::spoil_vault(w, berth.map).map(|vault| vault.0)
}

/// The facts of `tv` at landing, its ramp read through `hull` (its live record's).
fn facts_of(w: &World, tv: ObjId, hull: ObjId) -> Facts {
    match where_is(w, tv) {
        Where::Bay { hull } => Facts::Bay { hull },
        Where::Ground {
            domain, x_cm, y_cm, ..
        } => {
            let site = site_of_domain(w, domain);
            let cells = hull_view(w, hull)
                .and_then(|view| view.bay().and_then(|(bay, _)| ramp_pose(bay).ok()))
                .map(|ramp| cells_apart(site_cell(x_cm, y_cm), site_cell(ramp.x_cm, ramp.y_cm)));
            Facts::Ground {
                domain: domain.0,
                site,
                cells,
            }
        }
        Where::Elsewhere => Facts::Elsewhere,
    }
}

// ---- the providers' shared plumbing -------------------------------------------------------

/// The offer's guard set and presentation (the burn's shape).
fn offer_draft(
    action: &DefinitionRef,
    engine: &mush_core::Engine,
    fixed: Value,
    parameters: Vec<ProjectedParameter>,
    presentation: (&str, &str, &str),
) -> Result<ProviderOfferDraft, ActionError> {
    let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
    let definitions = engine
        .registry
        .definitions()
        .ok_or_else(|| integrity("the surface act's definition registry is unavailable"))?;
    let mut guards = engine
        .registry
        .server_guard_set_builder(RevisionSet::default())
        .map_err(|_| integrity("the surface act's guard registry is unavailable"))?;
    guards
        .add_definition(action.clone(), definitions)
        .map_err(|_| integrity("the surface act's definition guard is unavailable"))?;
    Ok(ProviderOfferDraft {
        fixed_parameters: fixed,
        parameters: ProjectedParameterSchema::new(parameters)
            .map_err(|_| integrity("the surface act's parameter schema is invalid"))?,
        targets: ProjectedTargetSchema::None,
        required_channels: Vec::new(),
        expires: None,
        presentation: mush_core::ActionPresentation::new(
            presentation.0,
            presentation.1,
            presentation.2,
        )
        .map_err(|_| integrity("the surface act's presentation is invalid"))?,
        guards: guards
            .finish()
            .map_err(|_| integrity("the surface act's offer guards are invalid"))?,
    })
}

/// One projected parameter.
fn parameter(
    id: &str,
    required: bool,
    kind: ProjectedParameterKind,
) -> Result<ProjectedParameter, ActionError> {
    Ok(ProjectedParameter {
        id: CanonicalId::new(id).map_err(|_| {
            port_error(
                ActionErrorCode::Integrity,
                "the surface act's parameter id is invalid",
            )
        })?,
        required,
        kind,
    })
}

/// Whether the query is this provider's self-scoped one.
fn self_query(
    action: &DefinitionRef,
    ours: &DefinitionRef,
    query: &mush_core::LegalActionQuery,
) -> bool {
    let SubjectKey::Object { id: subject } = query.truth_subject() else {
        return false;
    };
    action == ours
        && query.actor() == *subject
        && query.projected_subject() == &PerceivedRef::SelfActor
}

fn invalid(message: &'static str) -> ActionError {
    port_error(ActionErrorCode::InvalidRequest, message)
}

/// The invocation's parameters: self-scoped, no targets, an object.
fn parameters_of(invocation: &ActionInvocation) -> Result<&Map<String, Value>, ActionError> {
    if invocation.subject() != &PerceivedRef::SelfActor || !invocation.targets().is_empty() {
        return Err(invalid("a surface act is self-scoped and takes no targets"));
    }
    invocation
        .parameters()
        .as_object()
        .ok_or_else(|| invalid("a surface act names a rig and an act"))
}

/// The `tv` parameter: strict decimal text.
fn tv_of(parameters: &Map<String, Value>) -> Result<ObjId, ActionError> {
    parameters
        .get("tv")
        .and_then(Value::as_str)
        .and_then(RigKey::parse)
        .map(|key| key.0)
        .ok_or_else(|| invalid("a surface act names one rig"))
}

/// The `ordinal` parameter: `1..=u32::MAX`.
fn ordinal_of(parameters: &Map<String, Value>) -> Result<u32, ActionError> {
    parameters
        .get("ordinal")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n >= 1)
        .ok_or_else(|| invalid("a surface act names its ordinal"))
}

/// An optional bounded integer parameter, as `u32`.
fn optional_u32(parameters: &Map<String, Value>, id: &str, max: i64) -> Result<u32, ActionError> {
    match parameters.get(id) {
        None => Ok(0),
        Some(value) => value
            .as_u64()
            .filter(|n| i64::try_from(*n).is_ok_and(|n| n <= max))
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| invalid("a probe's figures are out of bounds")),
    }
}

/// The fixed `now`.
fn now_of(parameters: &Map<String, Value>) -> Result<i64, ActionError> {
    parameters
        .get("now")
        .and_then(Value::as_i64)
        .ok_or_else(|| invalid("a surface act's offer fixes the instant"))
}

/// The family's record, cloned; `None` when absent.
fn read_record(
    txn: &mut ActionTxn<'_>,
    family: &StateFamilyHandle<RigKey, RigRecord>,
    tv: ObjId,
) -> Result<Option<RigRecord>, ActionError> {
    let found = txn
        .get(family, &RigKey(tv))
        .map_err(|_| {
            port_error(
                ActionErrorCode::Integrity,
                "the rig records are unavailable",
            )
        })?
        .cloned();
    if let Some(record) = &found
        && record.tv != tv
    {
        // The families rule: readers re-check the key.
        return Err(port_error(
            ActionErrorCode::Integrity,
            "a rig record disagrees with its own key",
        ));
    }
    Ok(found)
}

/// Write `record` under its own key (insert or replace).
fn write_record(
    txn: &mut ActionTxn<'_>,
    family: &StateFamilyHandle<RigKey, RigRecord>,
    record: RigRecord,
    existed: bool,
) -> Result<(), ActionError> {
    record.check().map_err(|_| {
        port_error(
            ActionErrorCode::Integrity,
            "the rig record is out of bounds",
        )
    })?;
    let key = record.key();
    let staged = if existed {
        txn.replace(family, &key, record)
    } else {
        txn.insert(family, &key, record)
    };
    staged.map_err(|_| port_error(ActionErrorCode::Integrity, "the rig record did not stage"))
}

// ---- tacsim_surface_act: the start ---------------------------------------------------------------

/// **`tacsim_surface_act`** — every act's start (the brief's 3.4).
pub(crate) struct SurfaceActProvider {
    pub action: DefinitionRef,
    pub outcome: DefinitionRef,
    pub excursions: StateFamilyHandle<RigKey, RigRecord>,
    pub inventory: InventoryFamilies,
    pub tools: Arc<BTreeMap<CanonicalId, ToolDef>>,
    pub test_world: bool,
}

impl SurfaceActProvider {
    /// The act words this provider offers.
    fn words(&self) -> Vec<String> {
        let mut words: Vec<String> = ["dump", "launch", "recover"]
            .iter()
            .map(|w| (*w).to_owned())
            .collect();
        if self.test_world {
            words.push("probe".to_owned());
        }
        words
    }
}

impl ActionProvider for SurfaceActProvider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        let parameters = parameters_of(invocation)?;
        tv_of(parameters)?;
        ordinal_of(parameters)?;
        let kind = parameters
            .get("act")
            .and_then(Value::as_str)
            .and_then(ActKind::from_word)
            .ok_or_else(|| invalid("a surface act names launch, recover or dump"))?;
        if kind == ActKind::Probe && !self.test_world {
            return Err(invalid("a probe is a test world's"));
        }
        optional_u32(parameters, "probe_cu", MAX_PROBE_CU)?;
        optional_u32(parameters, "probe_s", MAX_PROBE_S)?;
        Ok(())
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        if !self_query(action, &self.action, query) {
            return Ok(None);
        }
        let w: &World = &engine.world;
        let actor = query.actor();
        let rigs = operator_rigs(w, actor);
        if rigs.is_empty() {
            return Ok(None);
        }
        let mut fixed_rigs: BTreeMap<String, FixedRig> = BTreeMap::new();
        for rig in &rigs {
            let battery_id = rig_battery_id(w, rig.tv);
            let mut plans: BTreeMap<String, FixedPlan> = BTreeMap::new();
            // A launch and a probe need the battery (`needs_battery`: a launch always, a
            // probe's cost is named at the invocation); a recover and a dump never do, so they
            // plan whether or not its row resolves (§4.1; the fixer round's item 3).
            let has_battery = battery_id.is_some();
            plans.insert(
                ActKind::Launch.word().to_owned(),
                if has_battery {
                    FixedPlan::of(launch_plan(w, actor, rig))
                } else {
                    FixedPlan::refused("tv_dark")
                },
            );
            plans.insert(
                ActKind::Recover.word().to_owned(),
                FixedPlan::of(recover_plan(w, actor, rig)),
            );
            let mut dump = FixedPlan::of(dump_plan(w, actor, rig));
            if dump.ok {
                dump.goods = tv::hopper_goods_of(w, rig.tv)
                    .map(|(goods, _)| goods)
                    .unwrap_or_default();
                if dump.goods.is_empty() {
                    dump = FixedPlan::refused("tv_dark");
                }
            }
            plans.insert(ActKind::Dump.word().to_owned(), dump);
            let probe = self.test_world.then(|| {
                if has_battery {
                    FixedPlan::of(probe_plan(w, actor, rig, 0, 0))
                } else {
                    FixedPlan::refused("tv_dark")
                }
            });
            fixed_rigs.insert(
                rig.tv.0.to_string(),
                FixedRig {
                    slot: rig.slot,
                    hull: rig.hull,
                    battery_id,
                    plans,
                    probe,
                },
            );
        }
        let world: Vec<u32> = world_rigs(w).iter().map(|tv| tv.0).collect();
        let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
        let fixed = json!({
            "now": w.world_ic_now().at,
            "rigs": serde_json::to_value(&fixed_rigs)
                .map_err(|_| integrity("the rigs' plans do not encode"))?,
            "world": world,
        });
        let mut parameters = vec![
            parameter(
                "tv",
                true,
                ProjectedParameterKind::Choice {
                    choices: rigs.iter().map(|rig| rig.tv.0.to_string()).collect(),
                },
            )?,
            parameter(
                "act",
                true,
                ProjectedParameterKind::Choice {
                    choices: self.words(),
                },
            )?,
            parameter(
                "ordinal",
                true,
                ProjectedParameterKind::Integer {
                    minimum: 1,
                    maximum: i64::from(u32::MAX),
                },
            )?,
        ];
        if self.test_world {
            parameters.push(parameter(
                "probe_cu",
                false,
                ProjectedParameterKind::Integer {
                    minimum: 0,
                    maximum: MAX_PROBE_CU,
                },
            )?);
            parameters.push(parameter(
                "probe_s",
                false,
                ProjectedParameterKind::Integer {
                    minimum: 0,
                    maximum: MAX_PROBE_S,
                },
            )?);
        }
        offer_draft(
            action,
            engine,
            fixed,
            parameters,
            (
                "tacsim-surface-act",
                "Work a rig",
                "Send a rig down the ramp, bring it in, or empty its hopper onto the spoil.",
            ),
        )
        .map(Some)
    }

    #[allow(clippy::too_many_lines)] // one ordered refusal list, then one bundle
    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.validate_invocation(invocation)?;
        let parameters = parameters_of(invocation)?;
        let actor = invocation.actor();
        let tv = tv_of(parameters)?;
        let ordinal = ordinal_of(parameters)?;
        let now = now_of(parameters)?;
        let kind = parameters
            .get("act")
            .and_then(Value::as_str)
            .and_then(ActKind::from_word)
            .ok_or_else(|| invalid("a surface act names launch, recover or dump"))?;
        // 1. The rig is the actor's and the act's fixed plan is ok.
        let rigs: BTreeMap<String, FixedRig> = parameters
            .get("rigs")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .ok_or_else(|| invalid("a surface act's offer fixes the rigs"))?;
        let Some(rig) = rigs.get(&tv.0.to_string()) else {
            return Err(port_error(
                ActionErrorCode::Unauthorized,
                "that rig is not yours to work",
            ));
        };
        let world: Vec<u32> = parameters
            .get("world")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .ok_or_else(|| invalid("a surface act's offer fixes the world's rigs"))?;
        let mut plan = match kind {
            ActKind::Probe => rig
                .probe
                .clone()
                .ok_or_else(|| invalid("a probe is a test world's"))?,
            ActKind::Launch | ActKind::Recover | ActKind::Dump => rig
                .plans
                .get(kind.word())
                .cloned()
                .ok_or_else(|| invalid("that act is not planned"))?,
        };
        if !plan.ok {
            return Err(port_error(
                ActionErrorCode::ProviderRejected,
                code_of_word(plan.code.as_deref().unwrap_or("tv_dark")),
            ));
        }
        if kind == ActKind::Probe {
            let cost_cu = optional_u32(parameters, "probe_cu", MAX_PROBE_CU)?;
            plan.cost_cu = cost_cu;
            plan.secs = optional_u32(parameters, "probe_s", MAX_PROBE_S)?;
            plan.params = Some(ActParams::Probe { cost_cu });
        }
        let site = plan
            .site
            .clone()
            .ok_or_else(|| invalid("an act's plan names its site"))?;
        // The battery, only where the act needs it (`needs_battery`; §4.1, the fixer round's
        // item 3): a recover or a dump starts with the row unresolvable.
        let battery = rig
            .battery_id
            .as_ref()
            .and_then(|id| battery_of(&self.tools, id));
        if battery.is_none() && needs_battery(kind, plan.cost_cu, plan.home_cu) {
            return Err(port_error(
                ActionErrorCode::ProviderRejected,
                "that rig cannot be read",
            ));
        }
        // 2. The record and the ordinal.
        let existing = read_record(txn, &self.excursions, tv)?;
        let existed = existing.is_some();
        let mut record = existing.unwrap_or_else(|| {
            battery.map_or_else(
                || RigRecord::fresh_unread(tv),
                |battery| RigRecord::fresh(tv, battery),
            )
        });
        if record.ordinal.checked_add(1) != Some(ordinal) {
            return Err(port_error(
                ActionErrorCode::Conflict,
                "that rig's orders moved on",
            ));
        }
        // 3. One act at a time.
        if record.act.is_some() {
            return Err(port_error(ActionErrorCode::Conflict, "that rig is busy"));
        }
        // 4. The launch's open excursion and the caps; a recover from the bay closes one.
        match kind {
            ActKind::Launch => {
                if record.excursion.is_some() {
                    return Err(port_error(
                        ActionErrorCode::ProviderRejected,
                        "that rig has an excursion open",
                    ));
                }
                let mut counts = ExcursionCounts::default();
                for other in world {
                    if let Some(found) = read_record(txn, &self.excursions, ObjId(other))?
                        && let Some(excursion) = &found.excursion
                    {
                        counts.world = counts.world.saturating_add(1);
                        if excursion.site == site {
                            counts.site = counts.site.saturating_add(1);
                        }
                        if excursion.operator == actor {
                            counts.operator = counts.operator.saturating_add(1);
                        }
                    }
                }
                if let Some(cap) = cap_refusal(&counts) {
                    return Err(port_error(
                        ActionErrorCode::ProviderRejected,
                        match cap {
                            CapRefused::Operator => {
                                "you have as many rigs out as one pilot may run"
                            }
                            CapRefused::Site => "that site has as many rigs out as it holds",
                            CapRefused::World => "the Registry has as many rigs out as it allows",
                        },
                    ));
                }
            }
            ActKind::Recover => {
                if plan.from_bay && record.excursion.is_none() {
                    return Err(port_error(
                        ActionErrorCode::ProviderRejected,
                        "that rig is in the bay",
                    ));
                }
            }
            ActKind::Dump | ActKind::Probe => {}
        }
        // 5. The reserve rule (§4.1; lead ruling 3): here and nowhere else. With no battery the
        // act spends nothing (`needs_battery` refused every spend above), which the rule
        // always passes.
        if let Some(battery) = battery
            && reserve_check(
                charge_at(&record.charge, now, battery),
                reserved_total(&record),
                plan.cost_cu,
                plan.home_cu,
                battery,
            )
            .is_err()
        {
            return Err(port_error(
                ActionErrorCode::ProviderRejected,
                "the rig's charge is under the reserve",
            ));
        }
        // 6. A dump freezes what the hopper holds now: each pool's **available** quantity
        // (its balance less every live hold), the one figure the finish compares and the
        // inventory's `transfer` can move — freezing the balance would refuse every dump of
        // a pool with a hold on it as `HopperChanged` (review finding tests/T4).
        let mut params = plan
            .params
            .clone()
            .ok_or_else(|| invalid("an act's plan names its parameters"))?;
        if let ActParams::Dump { hopper, goods, .. } = &mut params {
            for good in &plan.goods {
                let held = self
                    .inventory
                    .pool(txn, EndpointRef(VaultId(*hopper)), good)
                    .map_err(|_| {
                        port_error(ActionErrorCode::Integrity, "the hopper cannot be read")
                    })?
                    .map_or(0, |pool| pool.available);
                if held > 0 {
                    goods.insert(good.clone(), held);
                }
            }
            if goods.is_empty() {
                return Err(port_error(
                    ActionErrorCode::ProviderRejected,
                    "the hopper is empty",
                ));
            }
            if goods.len() > MAX_DUMP_GOODS {
                return Err(port_error(
                    ActionErrorCode::Integrity,
                    "the hopper holds more goods than a dump freezes",
                ));
            }
        }
        // 7. The write: the act, its due instant and the charge it reserves; a launch opens
        // the excursion. The charge itself is not written (lazy).
        let due_ic = now.checked_add(i64::from(plan.secs)).ok_or_else(|| {
            port_error(
                ActionErrorCode::Integrity,
                "the act's due instant overflows",
            )
        })?;
        record.ordinal = ordinal;
        record.act = Some(Act {
            ordinal,
            kind,
            operator: actor,
            hull: rig.hull,
            slot: rig.slot,
            site: site.clone(),
            started_ic: now,
            due_ic,
            reserved_cu: plan.cost_cu,
            params,
        });
        if kind == ActKind::Launch {
            record.excursion = Some(Excursion {
                id: ExcursionId { tv, launch_ic: now },
                operator: actor,
                hull: rig.hull,
                slot: rig.slot,
                site,
            });
        }
        write_record(txn, &self.excursions, record, existed)?;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({
                "act": kind.word(),
                "actor": actor.0,
                "due_ic": due_ic,
                "ordinal": ordinal,
                "tv": tv.0,
            }),
        )
        .map_err(|_| {
            port_error(
                ActionErrorCode::BoundsExceeded,
                "the surface act's outcome exceeds canonical bounds",
            )
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

// ---- tacsim_surface_done: the finish ------------------------------------------------------------

/// **`tacsim_surface_done`** — every act's finish (the brief's 3.4).
pub(crate) struct SurfaceDoneProvider {
    pub action: DefinitionRef,
    pub outcome: DefinitionRef,
    pub excursions: StateFamilyHandle<RigKey, RigRecord>,
    pub inventory: InventoryFamilies,
    pub tools: Arc<BTreeMap<CanonicalId, ToolDef>>,
}

/// The charge at the act's own stamp, less its cost; an underflow is `Integrity` (the
/// reserve already guaranteed it).
fn spent(
    charge: &Charge,
    act: &Act,
    battery: &BatteryDef,
    in_bay: bool,
) -> Result<Charge, ActionError> {
    let value = charge_at(charge, act.due_ic, battery)
        .checked_sub(act.reserved_cu)
        .ok_or_else(|| {
            port_error(
                ActionErrorCode::Integrity,
                "the act spent more than the charge",
            )
        })?;
    Ok(Charge {
        value_cu: value,
        at_ic: act.due_ic,
        in_bay,
    })
}

impl ActionProvider for SurfaceDoneProvider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        let parameters = parameters_of(invocation)?;
        tv_of(parameters)?;
        ordinal_of(parameters)?;
        Ok(())
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        if !self_query(action, &self.action, query) {
            return Ok(None);
        }
        let w: &World = &engine.world;
        let actor = query.actor();
        // Finding F6: only to the pilot of a live lease, over that pilot's own rigs —
        // whatever the helm, so a pilot whose charter is in trouble still finishes an act.
        let Some(store) = w.module_state::<TacsimStore>() else {
            return Ok(None);
        };
        let mut records: Vec<&TvLease> = store
            .tv_leases_of(actor)
            .filter(|record| record.live())
            .collect();
        records.sort_by_key(|record| record.tv);
        records.dedup_by_key(|record| record.tv);
        if records.is_empty() {
            return Ok(None);
        }
        let mut rigs: BTreeMap<String, DoneRig> = BTreeMap::new();
        for record in &records {
            rigs.insert(
                record.tv.0.to_string(),
                DoneRig {
                    battery_id: rig_battery_id(w, record.tv),
                    facts: facts_of(w, record.tv, record.hull),
                    spoil: spoil_of(w, record.hull),
                    failed: crate::surface::tick::finish_retried(w, record.tv),
                },
            );
        }
        let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
        let fixed = json!({
            "now": w.world_ic_now().at,
            "rigs": serde_json::to_value(&rigs)
                .map_err(|_| integrity("the rigs' facts do not encode"))?,
        });
        let parameters = vec![
            parameter(
                "tv",
                true,
                ProjectedParameterKind::Choice {
                    choices: records
                        .iter()
                        .map(|record| record.tv.0.to_string())
                        .collect(),
                },
            )?,
            parameter(
                "ordinal",
                true,
                ProjectedParameterKind::Integer {
                    minimum: 1,
                    maximum: i64::from(u32::MAX),
                },
            )?,
        ];
        offer_draft(
            action,
            engine,
            fixed,
            parameters,
            (
                "tacsim-surface-done",
                "Finish a rig's act",
                "Record how a rig's act came out, at its own due instant.",
            ),
        )
        .map(Some)
    }

    #[allow(clippy::too_many_lines)] // one ordered refusal list, then one decision per kind
    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let parameters = parameters_of(invocation)?;
        let actor = invocation.actor();
        let tv = tv_of(parameters)?;
        let ordinal = ordinal_of(parameters)?;
        let now = now_of(parameters)?;
        let rigs: BTreeMap<String, DoneRig> = parameters
            .get("rigs")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .ok_or_else(|| invalid("a finish's offer fixes the rigs"))?;
        let Some(rig) = rigs.get(&tv.0.to_string()) else {
            return Err(port_error(
                ActionErrorCode::Unauthorized,
                "that rig is not yours to work",
            ));
        };
        if rig.failed.len() > crate::surface::tick::MAX_RETRIED_ORDINALS {
            return Err(invalid(
                "a finish's offer fixes at most a bounded set of retried acts",
            ));
        }
        // 1. The act in flight, this one, this operator's, and due.
        let Some(mut record) = read_record(txn, &self.excursions, tv)? else {
            return Err(port_error(
                ActionErrorCode::Conflict,
                "that act is already finished",
            ));
        };
        let Some(act) = record.act.clone().filter(|act| act.ordinal == ordinal) else {
            return Err(port_error(
                ActionErrorCode::Conflict,
                "that act is already finished",
            ));
        };
        if act.operator != actor {
            return Err(port_error(
                ActionErrorCode::Unauthorized,
                "that act is another pilot's",
            ));
        }
        if act.due_ic > now {
            return Err(port_error(ActionErrorCode::Conflict, "not due"));
        }
        // The charge at the act's stamp: through the battery when its row resolves; without
        // it only a zero spend that does not leave the bay settles (`settle_free`), so a
        // recover lands with the row unresolvable and a launch fails closed (§4.1 "never
        // stranded by its battery"; the fixer round's item 3). A dump never touches it.
        let battery = rig
            .battery_id
            .as_ref()
            .and_then(|id| battery_of(&self.tools, id));
        let settle = |charge: &Charge, in_bay: bool| -> Result<Charge, ActionError> {
            match battery {
                Some(battery) => spent(charge, &act, battery, in_bay),
                None if act.reserved_cu == 0 => {
                    settle_free(charge, act.due_ic, in_bay).ok_or_else(|| {
                        port_error(ActionErrorCode::Integrity, "that rig cannot be read")
                    })
                }
                None => Err(port_error(
                    ActionErrorCode::Integrity,
                    "that rig cannot be read",
                )),
            }
        };
        // 2. Decide from the facts at landing and the kind (decision 5).
        let mut moved: BTreeMap<CanonicalId, u64> = BTreeMap::new();
        let result = match act.kind {
            ActKind::Launch => {
                let beside = matches!(
                    &rig.facts,
                    Facts::Ground { site: Some(site), cells: Some(cells), .. }
                        if *site == act.site && *cells <= 1
                );
                if beside {
                    record.charge = settle(&record.charge, false)?;
                    ActResult::Done
                } else {
                    // The excursion closes; the charge is untouched (the reservation
                    // released).
                    record.excursion = None;
                    ActResult::Refused {
                        code: RefusedCode::NeverLeft,
                    }
                }
            }
            ActKind::Recover => {
                if matches!(&rig.facts, Facts::Bay { hull } if *hull == act.hull) {
                    record.excursion = None;
                    record.charge = settle(&record.charge, true)?;
                    ActResult::Done
                } else {
                    ActResult::Refused {
                        code: RefusedCode::NotBeside,
                    }
                }
            }
            ActKind::Dump => {
                let ActParams::Dump {
                    hopper,
                    spoil,
                    goods,
                } = &act.params
                else {
                    return Err(port_error(
                        ActionErrorCode::Integrity,
                        "a dump carries its frozen goods",
                    ));
                };
                let spoil_vault = *spoil;
                let (hopper, spoil) = (EndpointRef(VaultId(*hopper)), EndpointRef(VaultId(*spoil)));
                let mut holds = true;
                for (good, qty) in goods {
                    let available = self
                        .inventory
                        .pool(txn, hopper, good)
                        .map_err(|_| {
                            port_error(ActionErrorCode::Integrity, "the hopper cannot be read")
                        })?
                        .map_or(0, |pool| pool.available);
                    if available < *qty {
                        holds = false;
                    }
                }
                // The spoil, before anything is staged (the fixer round's item 4): still the
                // site's live spoil (the offer's fact), every pool on it readable (a read stages
                // nothing), and no earlier finish of this dump (its own ordinal, `failed`) refused
                // mid-transfer. Any of them records `SpoilGone` with nothing moved: the bundle
                // then holds only the record, so the hopper is exactly as it was.
                let why = if !holds {
                    None
                } else if rig.spoil != Some(spoil_vault) {
                    Some("the site's spoil is gone or no longer the site's")
                } else if rig.failed.contains(&act.ordinal) {
                    Some("an earlier finish of this dump refused mid-transfer")
                } else if goods
                    .keys()
                    .any(|good| self.inventory.pool(txn, spoil, good).is_err())
                {
                    Some("a spoil pool will not read")
                } else {
                    None
                };
                if !holds {
                    ActResult::Refused {
                        code: RefusedCode::HopperChanged,
                    }
                } else if let Some(why) = why {
                    tracing::error!(
                        tv = tv.0,
                        act = ordinal,
                        spoil = spoil_vault,
                        site = %act.site,
                        why,
                        "tacsim: a dump's spoil would not take the load; the finish records it \
                         refused (SpoilGone) and nothing moved; staff check the site's spoil \
                         vault and its pools"
                    );
                    ActResult::Refused {
                        code: RefusedCode::SpoilGone,
                    }
                } else {
                    for (good, qty) in goods {
                        // A refusal here cannot be recorded in this bundle (its debit may be
                        // staged): the whole bundle is refused, nothing is written, and the
                        // tick's next finish records `SpoilGone` (`failed`).
                        self.inventory
                            .transfer(txn, hopper, spoil, good, *qty)
                            .map_err(|error| {
                                tracing::warn!(
                                    tv = tv.0,
                                    act = ordinal,
                                    spoil = spoil_vault,
                                    good = %good,
                                    %error,
                                    "tacsim: a dump's transfer onto the spoil refused; the next \
                                     finish records it refused"
                                );
                                port_error(
                                    ActionErrorCode::ProviderRejected,
                                    "the hopper could not be emptied onto the spoil",
                                )
                            })?;
                    }
                    moved.clone_from(goods);
                    ActResult::Done
                }
            }
            ActKind::Probe => {
                let in_bay = record.charge.in_bay;
                record.charge = settle(&record.charge, in_bay)?;
                ActResult::Done
            }
        };
        // 3. The act clears; the outcome is stamped at the act's own due instant.
        record.act = None;
        record.last = Some(Outcome {
            ordinal,
            kind: act.kind,
            operator: act.operator,
            slot: act.slot,
            site: act.site.clone(),
            at_ic: act.due_ic,
            result,
            moved,
        });
        write_record(txn, &self.excursions, record, true)?;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({
                "actor": actor.0,
                "ordinal": ordinal,
                "result": match result {
                    ActResult::Done => "done",
                    ActResult::Refused { .. } => "refused",
                },
                "tv": tv.0,
            }),
        )
        .map_err(|_| {
            port_error(
                ActionErrorCode::BoundsExceeded,
                "the finish's outcome exceeds canonical bounds",
            )
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every refusal's word is in the table and its message is the table's.
    #[test]
    fn sm3b_every_refusal_word_has_a_message() {
        let refusals = [
            Refusal::Dark {
                name: String::new(),
            },
            Refusal::Unwired,
            Refusal::NotAboard {
                name: String::new(),
            },
            Refusal::NotSite {
                name: String::new(),
            },
            Refusal::UnderWay {
                name: String::new(),
            },
            Refusal::Cleared {
                name: String::new(),
                port: String::new(),
            },
            Refusal::Out { n: 1 },
            Refusal::Open { n: 1 },
            Refusal::Forfeit {
                n: 1,
                counter: String::new(),
                port: String::new(),
            },
            Refusal::Ended {
                n: 1,
                counter: String::new(),
                port: String::new(),
            },
            Refusal::InBay { n: 1 },
            Refusal::Elsewhere {
                n: 1,
                name: String::new(),
            },
            Refusal::Far {
                n: 1,
                m: 20,
                ramp: String::new(),
            },
            Refusal::DumpNotSite {
                n: 1,
                name: String::new(),
            },
            Refusal::NoSpoil {
                site: String::new(),
            },
            Refusal::Empty { n: 1 },
            Refusal::Busy {
                n: 1,
                doing: String::new(),
                when: String::new(),
            },
            Refusal::Reserve {
                n: 1,
                now: 1,
                after: 0,
                floor: 5,
            },
            Refusal::Cap {
                cap: CapRefused::Operator,
                site: String::new(),
            },
        ];
        let mut words: Vec<&str> = refusals.iter().map(Refusal::word).collect();
        for refusal in &refusals {
            assert!(
                REFUSAL_CODES
                    .iter()
                    .any(|(w, m)| *w == refusal.word() && *m == refusal.code()),
                "{refusal:?}"
            );
            assert!(
                !refusal.line().contains('{'),
                "{refusal:?}: {}",
                refusal.line()
            );
        }
        words.sort_unstable();
        words.dedup();
        assert_eq!(words.len(), REFUSAL_CODES.len(), "one word per line");
        assert_eq!(
            Refusal::Far {
                n: 1,
                m: 20,
                ramp: "stern ramp".to_owned()
            }
            .line(),
            "Rig 1 is about 20 m from the stern ramp; it climbs aboard from beside it."
        );
        assert_eq!(code_of_word("nonsense"), "that rig cannot be read");
    }

    /// The keys of both actions form.
    #[test]
    fn sm3b_the_act_and_done_keys_form() {
        assert_eq!(
            act_key(ObjId(900), 3).map(|k| k.as_str().to_owned()),
            Some("tacsim.surface/900/3/act".to_owned())
        );
        assert_eq!(
            done_key(ObjId(900), 3).map(|k| k.as_str().to_owned()),
            Some("tacsim.surface/900/3/done".to_owned())
        );
    }
}
