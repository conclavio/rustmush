//! **The acts: their world plans, their start and their finish** (doc 20 cut SM.3b; §1 beats 4
//! and 9, §4.1, §4.3, §6's authoritative-start shape, §8, §10; D-20-6, D-20-7, D-20-11; the
//! SM.3b brief's 3.3, 3.4 and 3.6 and its lead rulings 2, 3, 5 and 8).
//!
//! One module holds what the command, the two offers and the tick all read, so they cannot
//! drift:
//!
//! - **the world plans** — `launch_plan`,
//!   `recover_plan`, `dump_plan` and, since doc 20 cut SM.4a, `scan_plan` and `sound_plan`:
//!   each act's whole world-side gate in a fixed order, pure reads of the `World`, used by
//!   `+tv`'s preview, by its `go` and by the start's offer. **A scan or a sounding from the
//!   bay prepends `launch_plan`** (the implied launch, SM.4a's lead ruling 2: one act whose
//!   plan has `from_bay` set, `LAUNCH_S` longer, opening the excursion as a launch does);
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
//! **Doc 20 cut SM.4a** adds the knowing acts on the same two actions (no third): a **scan**
//! and a **sounding** (loud or quiet; the start's `act` word `scan`, `sound` or `sound_quiet`,
//! parsed once by `act_of_word`). The start's prepare also reads the operator's chart
//! (`crate::surface::chart`) for "You learn nothing new here." and the body caps; the finish
//! writes it — coverage and provenance, stamped at the act's own `due_ic` — and decides the
//! site-wide heard line's budget. No act reads a licence or a tenure (§8 "rights are per act";
//! lead ruling 9). The probe kind SM.3b kept for its tests is retired (D15).
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

use crate::definitions::tools::{BatteryDef, ScannerDef, ToolDef};
use crate::port::coop::{name_of, port_name};
use crate::port::def::PortDef;
use crate::port::families::PortHandles;
use crate::port::map::PortMap;
use crate::port::ship::{Berth, HullView, berth_of, hull_view};
use crate::port::tv::{self, TvLease, TvLeaseStatus};
use crate::port_tick::port_error;
use crate::store::TacsimStore;
use crate::surface::address::{self, cell_on_site, pockets_in_reach};
use crate::surface::chart::{
    self, ChartKey, ChartRecord, SitePeek, body_admits, heard_line_due, repeat_refused, scan_adds,
    sound_adds, write_scan, write_sound,
};
use crate::surface::excursion::{
    Act, ActKind, ActParams, ActResult, CapRefused, Charge, DUMP_S, Excursion, ExcursionCounts,
    ExcursionId, LAUNCH_S, MAX_DUMP_GOODS, Outcome, RECOVER_S, ReadSpan, RefusedCode, RigKey,
    RigRecord, cap_refusal, cells_apart, charge_at, charge_for_act, needs_battery, ramp_pose,
    reserve_check, reserved_total, settle_free, settle_spent_out_of_bay, site_cell,
};
use crate::surface::reading::Footing;

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
/// **The start's `act` words** (doc 20 cut SM.4a, D23): the offer's Choice and the plans' keys,
/// sorted.
pub const ACT_WIRE_WORDS: [&str; 6] = ["dump", "launch", "recover", "scan", "sound", "sound_quiet"];

/// **The start's `act` parameter** (doc 20 cut SM.4a; the brief's reviewer finding F6, D23):
/// `launch`, `recover`, `dump`, `scan`, `sound` and `sound_quiet` — the kind and the sounding's
/// quiet flag. Both the start's `validate_invocation` and its prepare read it, never
/// `ActKind::from_word`.
#[must_use]
pub(crate) fn act_of_word(word: &str) -> Option<(ActKind, bool)> {
    match word {
        "sound_quiet" => Some((ActKind::Sound, true)),
        "launch" | "recover" | "dump" | "scan" | "sound" => {
            ActKind::from_word(word).map(|kind| (kind, false))
        }
        _ => None,
    }
}

/// The inverse of [`act_of_word`]: a quiet sounding is `sound_quiet`; the quiet flag means
/// nothing for any other kind.
#[must_use]
pub(crate) fn wire_word(kind: ActKind, quiet: bool) -> &'static str {
    match (kind, quiet) {
        (ActKind::Sound, true) => "sound_quiet",
        (kind, _) => kind.word(),
    }
}

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
    /// The site's port entry definition (`+tv` status reads its site block's tenure).
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
    /// `LEARN_NOTHING` (doc 20 cut SM.4a; §5, D4).
    NothingNew,
    /// `CHART_FULL` (doc 20 cut SM.4a; §5's caps, D1, D3).
    ChartFull {
        /// The body's display name.
        body: String,
    },
    /// `GROUND_COLD` (doc 20 cut SM.4a; lead ruling 6): the site's ground is not cached yet.
    GroundCold {
        /// The site's name.
        site: String,
        /// The site's port entry (the command asks for its warm).
        entry: CanonicalId,
    },
    /// `INSTRUMENTS_DARK` (doc 20 cut SM.4a; D16, D25).
    InstrumentsDark,
    /// `NO_FOOTING` (doc 20 cut SM.4a; §5): the rig stands where the mast reads nothing.
    NoFooting {
        /// The slot.
        n: u8,
        /// The terrain's word.
        terrain: String,
    },
}

/// The offer-wire words and the start's messages, in one table (the prepare's
/// `port_error` needs a `&'static str`).
const REFUSAL_CODES: [(&str, &str); 24] = [
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
    ("nothing_new", "you learn nothing new here"),
    ("chart_full", "your chart of that body is full"),
    ("ground_cold", "the site's ground is still being mapped"),
    ("instruments_dark", "the instruments cannot read the ground"),
    ("no_footing", "the rig stands where the mast reads nothing"),
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
            Refusal::NothingNew => "nothing_new",
            Refusal::ChartFull { .. } => "chart_full",
            Refusal::GroundCold { .. } => "ground_cold",
            Refusal::InstrumentsDark => "instruments_dark",
            Refusal::NoFooting { .. } => "no_footing",
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
            Refusal::NothingNew => tv::LEARN_NOTHING.to_owned(),
            Refusal::ChartFull { body } => tv::CHART_FULL.replace("{body}", body),
            Refusal::GroundCold { site, .. } => tv::GROUND_COLD.replace("{site}", site),
            Refusal::InstrumentsDark => tv::INSTRUMENTS_DARK.to_owned(),
            Refusal::NoFooting { n, terrain } => tv::NO_FOOTING
                .replace("{n}", &n.to_string())
                .replace("{terrain}", terrain),
        }
    }
}

// ---- the world plans --------------------------------------------------------------------------

/// **One act, planned against the world** (the brief's 3.3): what the preview prints, what
/// the `go` submits and what the start's offer fixes.
#[derive(Clone, Debug)]
pub(crate) struct ActPlan {
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
    /// The rig stands in this hull's bay at plan time (decision 21; doc 20 cut SM.4a, D12): a
    /// launch sets it, a recover from the bay sets it, and for a scan or a sounding it means
    /// **the launch is implied**.
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
/// **A scan or a sounding from the bay prepends this** (doc 20 cut SM.4a's lead ruling 2: the
/// implied launch; SM.4b's drive will too): keep the refusal order stable.
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

/// Where a scan or a sounding is made from.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Stand {
    /// The site's port entry.
    site: CanonicalId,
    /// The stand cell, site-local.
    cell: (i64, i64),
    /// The launch is implied (the rig stands in the bay; it reads from the ramp's cell).
    from_bay: bool,
}

/// **Where a scan or a sounding stands, in the plan's order** (doc 20 cut SM.4a; the brief's
/// 3.4, D12, D13): the layer and the view (`view_of`); under way → `LAUNCH_UNDER_WAY`; then
/// the rig: in this hull's bay → **`launch_plan`, its whole order unchanged**, and the stand
/// is the ramp's cell (`from_bay`); otherwise the hull's site berth (not a site →
/// `LAUNCH_NOT_SITE`), the actor aboard (else `TV_NOT_ABOARD`), and the rig `Direct` on the
/// hull's site domain inside the site (else `RECOVER_ELSEWHERE`) — **no lease-state gate**, as
/// a recover's and a dump's (decision 11).
fn stand_of(w: &World, actor: ObjId, rig: &RigAt) -> Result<Stand, Refusal> {
    let (view, name) = view_of(w, rig)?;
    if matches!(berth_of(w, rig.hull), Berth::UnderWay) {
        return Err(Refusal::UnderWay { name });
    }
    if where_is(w, rig.tv) == (Where::Bay { hull: rig.hull }) {
        let launch = launch_plan(w, actor, rig)?;
        let berth = site_berth(w, &view)?;
        return Ok(Stand {
            site: launch.site,
            cell: site_cell(berth.ramp.x_cm, berth.ramp.y_cm),
            from_bay: true,
        });
    }
    let berth = site_berth(w, &view)?;
    if !crate::port::voyage::aboard(w, rig.hull, actor) {
        return Err(Refusal::NotAboard { name });
    }
    match where_is(w, rig.tv) {
        Where::Ground {
            domain, x_cm, y_cm, ..
        } if domain == berth.domain && cell_on_site(site_cell(x_cm, y_cm)) => Ok(Stand {
            site: berth.entry,
            cell: site_cell(x_cm, y_cm),
            from_bay: false,
        }),
        Where::Bay { .. } | Where::Ground { .. } | Where::Elsewhere => {
            Err(Refusal::Elsewhere { n: rig.slot, name })
        }
    }
}

/// **The scan's world-side gate, in order** (doc 20 cut SM.4a; §5's R1, the brief's 3.4): the
/// stand (`stand_of`, the implied launch's whole gate first); the site's ground resolved
/// (`chart::site_context`, else `INSTRUMENTS_DARK`); the composite through the **non-building
/// peek**, shared by every rig of the call (`peeks`; cold → `GROUND_COLD`, dark →
/// `INSTRUMENTS_DARK`); the footing at the stand (none → `NO_FOOTING`); the fields the mast
/// sees (`chart::seen_fields`). `LAUNCH_S` more when it launches; the scanner's `scan_s` and
/// `scan_cu`; no trip home yet (`home_cu` 0 until SM.4b's drive supplies it — lead ruling 1).
///
/// # Errors
///
/// The first [`Refusal`] in that order.
pub(crate) fn scan_plan(
    w: &World,
    actor: ObjId,
    rig: &RigAt,
    scanner: &ScannerDef,
    peeks: &mut BTreeMap<CanonicalId, SitePeek>,
) -> Result<ActPlan, Refusal> {
    let stand = stand_of(w, actor, rig)?;
    let site = chart::site_context(w, &stand.site).map_err(|_| Refusal::InstrumentsDark)?;
    let ground = match chart::peek_site(w, &stand.site, peeks) {
        SitePeek::Warm(ground) => ground,
        SitePeek::Cold => {
            return Err(Refusal::GroundCold {
                site: site.name.clone(),
                entry: stand.site.clone(),
            });
        }
        SitePeek::Dark => return Err(Refusal::InstrumentsDark),
    };
    let surface = chart::ground_surface(&ground).ok_or(Refusal::InstrumentsDark)?;
    let footing = chart::footing_at(surface, stand.cell).map_err(|terrain| Refusal::NoFooting {
        n: rig.slot,
        terrain: terrain.label().replace('_', " "),
    })?;
    let (fields, hidden) = chart::seen_fields(surface, stand.cell, scanner);
    if fields.is_empty() {
        return Err(Refusal::InstrumentsDark);
    }
    let launch = if stand.from_bay { LAUNCH_S } else { 0 };
    Ok(ActPlan {
        site: stand.site,
        secs: launch.saturating_add(scanner.scan_s),
        cost_cu: scanner.scan_cu,
        // Lead ruling 1: SM.3b's 0 until SM.4b's drive supplies the trip home.
        home_cu: 0,
        params: ActParams::Scan {
            from_bay: stand.from_bay,
            body: site.spec.body.clone(),
            cell: [stand.cell.0, stand.cell.1],
            footing,
            fields,
            hidden,
        },
        from_bay: stand.from_bay,
    })
}

/// **The sounding's world-side gate, in order** (doc 20 cut SM.4a; §5's R2): the stand
/// (`stand_of`), the site's ground resolved (`chart::site_context`, else `INSTRUMENTS_DARK`:
/// an act that could render nothing never spends charge, D16). It needs no composite. The
/// reach is the scanner's `sound_radius_cells`, or `quiet_radius_cells` when quiet, at the
/// same `sound_s` and `sound_cu`; `LAUNCH_S` more when it launches; `home_cu` 0 (lead ruling 1).
///
/// # Errors
///
/// The first [`Refusal`] in that order.
pub(crate) fn sound_plan(
    w: &World,
    actor: ObjId,
    rig: &RigAt,
    scanner: &ScannerDef,
    quiet: bool,
) -> Result<ActPlan, Refusal> {
    let stand = stand_of(w, actor, rig)?;
    let site = chart::site_context(w, &stand.site).map_err(|_| Refusal::InstrumentsDark)?;
    let launch = if stand.from_bay { LAUNCH_S } else { 0 };
    let radius = if quiet {
        scanner.quiet_radius_cells
    } else {
        scanner.sound_radius_cells
    };
    Ok(ActPlan {
        site: stand.site,
        secs: launch.saturating_add(scanner.sound_s),
        cost_cu: scanner.sound_cu,
        // Lead ruling 1: SM.3b's 0 until SM.4b's drive supplies the trip home.
        home_cu: 0,
        params: ActParams::Sound {
            from_bay: stand.from_bay,
            body: site.spec.body.clone(),
            cell: [stand.cell.0, stand.cell.1],
            radius,
            quiet,
        },
        from_bay: stand.from_bay,
    })
}

/// **Which scanner a rig carries** (doc 20 cut SM.4a; `rig_battery_id`'s rule): the lessor
/// pool's tools whose row in the one tools table is a scanner.
#[must_use]
pub(crate) fn rig_scanner_id(w: &World, rig_tv: ObjId) -> Option<CanonicalId> {
    crate::port::vehicle::rig_row(w, rig_tv)?;
    let tools = w.module_state::<PortHandles>()?.surface_tools.clone()?;
    let port = crate::port::coop::programme_port(w)?;
    let pool = port.def.vehicles.as_deref()?;
    pool.tools
        .iter()
        .find(|id| tools.get(*id).and_then(ToolDef::scanner).is_some())
        .cloned()
}

/// **A scanner's figures** by its id, from a tools table (`ToolDef::scanner`).
#[must_use]
pub(crate) fn scanner_of<'t>(
    tools: &'t BTreeMap<CanonicalId, ToolDef>,
    id: &CanonicalId,
) -> Option<&'t ScannerDef> {
    tools.get(id).and_then(ToolDef::scanner)
}

/// **The plan of a reading act's stand and reach**, for the chart rules: the stand cell, the
/// fields a scan reads with its footing, or the pockets a sounding reads.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Reach {
    /// A scan.
    Scan {
        stand: [i64; 2],
        fields: Vec<u8>,
        footing: Footing,
    },
    /// A sounding.
    Sound {
        stand: [i64; 2],
        pockets: Vec<(i64, i64)>,
    },
}

impl Reach {
    /// A scan's or a sounding's reach from its frozen parameters; `None` for every other act.
    fn of(params: &ActParams) -> Option<Reach> {
        match params {
            ActParams::Scan {
                cell,
                fields,
                footing,
                ..
            } => Some(Reach::Scan {
                stand: *cell,
                fields: fields.clone(),
                footing: *footing,
            }),
            ActParams::Sound { cell, radius, .. } => Some(Reach::Sound {
                stand: *cell,
                pockets: pockets_in_reach((cell[0], cell[1]), i64::from(*radius)),
            }),
            ActParams::None {} | ActParams::Dump { .. } => None,
        }
    }

    /// **"You learn nothing new here."** (D4) against the operator's row for the site.
    fn refused(&self, row: Option<&ChartRecord>) -> bool {
        match self {
            Reach::Scan {
                stand,
                fields,
                footing,
            } => repeat_refused(row, false, *stand, scan_adds(row, fields, *footing)),
            Reach::Sound { stand, pockets } => {
                repeat_refused(row, true, *stand, sound_adds(row, pockets))
            }
        }
    }

    /// The units the act names, for the body caps: field indexes, block letters.
    fn units(&self) -> (Vec<u8>, Vec<String>) {
        match self {
            Reach::Scan { fields, .. } => (fields.clone(), Vec::new()),
            Reach::Sound { pockets, .. } => {
                let blocks: std::collections::BTreeSet<String> = pockets
                    .iter()
                    .filter_map(|(px, py)| address::block_letter(*px, *py))
                    .map(|letter| letter.to_string())
                    .collect();
                (Vec::new(), blocks.into_iter().collect())
            }
        }
    }
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
        ActKind::Scan => "scanning".to_owned(),
        ActKind::Sound => "sounding".to_owned(),
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
/// **Doc 20 cut SM.4a**: a scan or a sounding (`quiet` for `sound quiet`) plans through
/// `scan_plan`/`sound_plan` — the site's composite peeked once for the call, a cold one
/// answering `GROUND_COLD` and asking for its warm (RAM only, the `RigPointer` precedent) —
/// then, when it implies the launch, the launch's open-excursion refusal and its caps; then the
/// operator's chart: "You learn nothing new here." (the same-cell rule, D4) and the body caps
/// (`CHART_FULL`); then the reserve from `charge_for_act` (the bay credited no further than the
/// launch half, D22). No licence and no tenure is read (lead ruling 9).
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line.
#[allow(clippy::too_many_lines)] // one ordered refusal list, then the preview or the go
pub(crate) fn act_cmd(
    ctx: &mut Ctx<'_>,
    rig: &RigAt,
    kind: ActKind,
    quiet: bool,
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
    let name = name_of(ctx.w, rig.hull);
    let dark = || CmdError::msg(tv::TV_DARK.replace("{name}", &name));
    // 1. The world plan (the start's offer runs the same function). A reading act needs the
    // rig's scanner; a scan peeks the site's composite once for the whole call.
    let scanner = handles.surface_tools.as_ref().and_then(|tools| {
        rig_scanner_id(ctx.w, rig.tv).and_then(|id| scanner_of(tools, &id).copied())
    });
    let mut peeks: BTreeMap<CanonicalId, SitePeek> = BTreeMap::new();
    let planned = match kind {
        ActKind::Launch => launch_plan(ctx.w, actor, rig),
        ActKind::Recover => recover_plan(ctx.w, actor, rig),
        ActKind::Dump => dump_plan(ctx.w, actor, rig),
        ActKind::Scan | ActKind::Sound => {
            let Some(scanner) = scanner.as_ref() else {
                tracing::warn!(tv = rig.tv.0, "tacsim: a rig carries no readable scanner");
                return Err(dark());
            };
            if kind == ActKind::Scan {
                scan_plan(ctx.w, actor, rig, scanner, &mut peeks)
            } else {
                sound_plan(ctx.w, actor, rig, scanner, quiet)
            }
        }
    };
    let plan = match planned {
        Ok(plan) => plan,
        Err(refusal) => {
            // A cold site is asked for: the surface tick warms it, never this path.
            if let Refusal::GroundCold { entry, .. } = &refusal {
                crate::surface::want_site_ground(ctx.w, entry);
            }
            return Err(CmdError::msg(refusal.line()));
        }
    };
    let w: &World = ctx.w;
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
    // The launch's half: a launch, or a scan or a sounding that implies one (lead ruling 2).
    let launches = kind == ActKind::Launch || (kind.reads_ground() && plan.from_bay);
    if launches && record.excursion.is_some() {
        return Err(CmdError::msg(Refusal::Open { n: rig.slot }.line()));
    }
    if kind == ActKind::Recover && plan.from_bay && record.excursion.is_none() {
        return Err(CmdError::msg(Refusal::InBay { n: rig.slot }.line()));
    }
    if launches {
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
    // 4. The chart (doc 20 cut SM.4a): the same-cell rule, then the body caps.
    if let Some(reach) = Reach::of(&plan.params) {
        let read = |site: &CanonicalId| -> Result<Option<ChartRecord>, CmdError> {
            ctx.action_state(
                &handles.families.charts,
                &ChartKey {
                    principal: actor,
                    site: site.clone(),
                },
            )
            .map(|found| found.cloned())
            .map_err(|error| {
                tracing::warn!(tv = rig.tv.0, %error, "tacsim: a chart row does not read");
                dark()
            })
        };
        let row = read(&plan.site)?;
        if reach.refused(row.as_ref()) {
            return Err(CmdError::msg(Refusal::NothingNew.line()));
        }
        let body = match &plan.params {
            ActParams::Scan { body, .. } | ActParams::Sound { body, .. } => body.clone(),
            ActParams::None {} | ActParams::Dump { .. } => return Err(dark()),
        };
        let mut rows = Vec::new();
        for site in chart::body_sites(w, &body) {
            if let Some(found) = read(&site)? {
                rows.push(found);
            }
        }
        let (fields, blocks) = reach.units();
        if !body_admits(&rows, &plan.site, &fields, &blocks) {
            return Err(CmdError::msg(
                Refusal::ChartFull {
                    body: crate::port::site::body_name(&body),
                }
                .line(),
            ));
        }
    }
    let now_ic = w.world_ic_now().at;
    // `(now %, after %)`, `None` with no battery (a zero spend: the rule passes it unread). An
    // implied launch spends from the charge the bay holds when the launch half ends (D22).
    let charge = match &battery {
        Some(battery) => {
            let shown = charge_at(&record.charge, now_ic, battery);
            let from = charge_for_act(
                &record.charge,
                now_ic,
                kind.reads_ground() && plan.from_bay,
                battery,
            );
            match reserve_check(
                from,
                reserved_total(&record),
                plan.cost_cu,
                plan.home_cu,
                battery,
            ) {
                Ok(after) => Some((percent(shown, battery), percent(after.after_home, battery))),
                Err(short) => {
                    let after = u32::try_from(short.after_home.max(0)).unwrap_or(0);
                    return Err(CmdError::msg(
                        Refusal::Reserve {
                            n: rig.slot,
                            now: percent(shown, battery),
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
            ActKind::Dump => tv::DUMP_PREVIEW
                .replace("{n}", &n)
                .replace("{mark}", &rig.mark)
                .replace("{load}", &load)
                .replace("{site}", &site),
            ActKind::Scan | ActKind::Sound => {
                // A scan or a sounding spends, so `charge` is `Some` here.
                let Some((now, after)) = charge else {
                    return Err(dark());
                };
                let Some(scanner) = scanner.as_ref() else {
                    return Err(dark());
                };
                reading_preview(
                    w,
                    &plan,
                    scanner,
                    &mut peeks,
                    PreviewWords {
                        n: &n,
                        mark: &rig.mark,
                        ramp: &ramp,
                        site: &site,
                        real: &real,
                        now,
                        after,
                    },
                )
                .ok_or_else(dark)?
            }
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
            "act": wire_word(kind, quiet),
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
        ActKind::Dump => tv::DUMP_FILED.replace("{n}", &n),
        ActKind::Scan | ActKind::Sound => {
            let template = match (kind, plan.from_bay) {
                (ActKind::Scan, false) => tv::SCAN_FILED,
                (ActKind::Scan, true) => tv::SCAN_FILED_LAUNCH,
                (_, false) => tv::SOUND_FILED,
                (_, true) => tv::SOUND_FILED_LAUNCH,
            };
            template
                .replace("{n}", &n)
                .replace("{ramp}", &ramp)
                .replace("{when}", &when)
        }
    };
    crate::port::coop::say(ctx, line);
    Ok(())
}

/// The words every reading preview shares.
struct PreviewWords<'a> {
    n: &'a str,
    mark: &'a str,
    ramp: &'a str,
    site: &'a str,
    real: &'a str,
    now: u32,
    after: u32,
}

/// **A scan's or a sounding's preview** (doc 20 cut SM.4a; the brief's 3.8): `SCAN_PREVIEW` —
/// the fields the mast sees around the stand's field, the footing and what better footing
/// would buy (`chart::better_footing` within the scan's reach), the hidden count — or
/// `SOUND_PREVIEW` — the pockets in reach around the stand's pocket and who hears it; both name
/// the implied launch's two legs. `None` when a name does not form (never expected).
fn reading_preview(
    w: &World,
    plan: &ActPlan,
    scanner: &ScannerDef,
    peeks: &mut BTreeMap<CanonicalId, SitePeek>,
    words: PreviewWords<'_>,
) -> Option<String> {
    let launch = if plan.from_bay {
        tv::IMPLIED_LAUNCH
            .replace("{ramp}", words.ramp)
            .replace("{site}", words.site)
    } else {
        String::new()
    };
    let own_secs = if plan.from_bay {
        plan.secs.saturating_sub(LAUNCH_S)
    } else {
        plan.secs
    };
    let time = if plan.from_bay {
        tv::TWO_LEGS
            .replace("{down}", &LAUNCH_S.to_string())
            .replace("{secs}", &own_secs.to_string())
    } else {
        format!("{own_secs} IC s")
    };
    let head = |template: &str| {
        template
            .replace("{n}", words.n)
            .replace("{mark}", words.mark)
            .replace("{launch}", &launch)
            .replace("{time}", &time)
            .replace("{real}", words.real)
            .replace("{now}", &words.now.to_string())
            .replace("{after}", &words.after.to_string())
    };
    match &plan.params {
        ActParams::Scan {
            cell,
            footing,
            fields,
            hidden,
            ..
        } => {
            let stand = (cell[0], cell[1]);
            let field =
                address::field_of_cell(stand).and_then(|(fx, fy)| address::field_name(fx, fy))?;
            let radius = i64::from(scanner.scan_radius_cells);
            let better = match chart::peek_site(w, &plan.site, peeks) {
                SitePeek::Warm(ground) => chart::ground_surface(&ground)
                    .and_then(|surface| chart::better_footing(surface, stand, *footing, radius)),
                SitePeek::Cold | SitePeek::Dark => None,
            };
            let reach_m = radius.saturating_mul(10).to_string();
            let footing_words = match (footing, better) {
                (Footing::Rock, _) => tv::FOOTING_ROCK.to_owned(),
                (Footing::Regolith, Some((_, m, dir))) => tv::FOOTING_REGOLITH
                    .replace("{m}", &m.to_string())
                    .replace("{dir}", dir.abbrev()),
                (Footing::Regolith, None) => tv::FOOTING_REGOLITH_NONE.replace("{r}", &reach_m),
                (Footing::Ice, Some((sharper, m, dir))) => tv::FOOTING_ICE
                    .replace("{better}", sharper.name())
                    .replace("{m}", &m.to_string())
                    .replace("{dir}", dir.abbrev()),
                (Footing::Ice, None) => tv::FOOTING_ICE_NONE.replace("{r}", &reach_m),
            };
            let hidden_words = if hidden.is_empty() {
                String::new()
            } else {
                tv::HIDDEN_FIELDS.replace("{k}", &hidden.len().to_string())
            };
            Some(
                head(tv::SCAN_PREVIEW)
                    .replace("{count}", &fields.len().to_string())
                    .replace("{field}", &field)
                    .replace("{footing}", &footing_words)
                    .replace("{hidden}", &hidden_words),
            )
        }
        ActParams::Sound {
            cell,
            radius,
            quiet,
            ..
        } => {
            let stand = (cell[0], cell[1]);
            let (px, py) = address::pocket_of_cell(stand)?;
            let addr = address::pocket_address(px, py)?;
            let count = pockets_in_reach(stand, i64::from(*radius)).len();
            let heard = if *quiet {
                tv::HEARD_BY_NOBODY.to_owned()
            } else {
                tv::HEARD_ACROSS.replace("{site}", words.site)
            };
            Some(
                head(tv::SOUND_PREVIEW)
                    .replace("{count}", &count.to_string())
                    .replace("{addr}", &addr)
                    .replace("{heard}", &heard)
                    .replace("{quiet}", if *quiet { " quiet" } else { "" }),
            )
        }
        ActParams::None {} | ActParams::Dump { .. } => None,
    }
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
    /// **Doc 20 cut SM.4a**: a scan's or a sounding's body's site entries (at most
    /// `MAX_CHART_SITES_PER_BODY`), whose chart rows the prepare reads for the body caps.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    body_sites: Vec<CanonicalId>,
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
                body_sites: Vec::new(),
            },
            Err(refusal) => FixedPlan::refused(refusal.word()),
        }
    }
}

/// One rig as the start's offer fixes it: its plans keyed by the start's `act` word
/// ([`ACT_WIRE_WORDS`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixedRig {
    slot: u8,
    hull: ObjId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    battery_id: Option<CanonicalId>,
    plans: BTreeMap<String, FixedPlan>,
}

/// The world as the finish's offer reads it at landing (decision 5).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "where", rename_all = "snake_case", deny_unknown_fields)]
enum Facts {
    /// In a programme hull's bay.
    Bay { hull: ObjId },
    /// On a domain: the site whose domain it is (if any) and the cells from that site's
    /// ramp (through the hull of the rig's live record), and — doc 20 cut SM.4a — the rig's
    /// site cell on a site's domain (a scan's or a sounding's frozen stand is compared with it).
    Ground {
        domain: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        site: Option<CanonicalId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cells: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cell: Option<[i64; 2]>,
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
            let cell = site.as_ref().map(|_| {
                let (x, y) = site_cell(x_cm, y_cm);
                [x, y]
            });
            Facts::Ground {
                domain: domain.0,
                site,
                cells,
                cell,
            }
        }
        Where::Elsewhere => Facts::Elsewhere,
    }
}

// ---- the providers' shared plumbing -------------------------------------------------------

/// The offer's guard set and presentation (the burn's shape).
pub(crate) fn offer_draft(
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
pub(crate) fn parameter(
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
pub(crate) fn self_query(
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

/// The `act` parameter through [`act_of_word`] (D23): the kind and the sounding's quiet flag.
fn act_param(parameters: &Map<String, Value>) -> Result<(ActKind, bool), ActionError> {
    parameters
        .get("act")
        .and_then(Value::as_str)
        .and_then(act_of_word)
        .ok_or_else(|| {
            invalid("a surface act names launch, recover, dump, scan, sound or sound_quiet")
        })
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

/// **`tacsim_surface_act`** — every act's start (the brief's 3.4; doc 20 cut SM.4a adds the scan
/// and the soundings, and retires SM.3b's test-world probe).
pub(crate) struct SurfaceActProvider {
    pub action: DefinitionRef,
    pub outcome: DefinitionRef,
    pub excursions: StateFamilyHandle<RigKey, RigRecord>,
    pub charts: StateFamilyHandle<ChartKey, ChartRecord>,
    pub inventory: InventoryFamilies,
    pub tools: Arc<BTreeMap<CanonicalId, ToolDef>>,
}

/// **The start's fixed plan of a reading act** (doc 20 cut SM.4a): its plan, with the body's
/// site entries for the body caps; a refused one when the rig has no readable battery or
/// scanner (a scan or a sounding spends).
fn reading_fixed(
    w: &World,
    plan: Option<Result<ActPlan, Refusal>>,
    has_battery: bool,
) -> FixedPlan {
    let (Some(plan), true) = (plan, has_battery) else {
        return FixedPlan::refused("tv_dark");
    };
    let body = plan.as_ref().ok().and_then(|plan| match &plan.params {
        ActParams::Scan { body, .. } | ActParams::Sound { body, .. } => Some(body.clone()),
        ActParams::None {} | ActParams::Dump { .. } => None,
    });
    let mut fixed = FixedPlan::of(plan);
    if let Some(body) = body {
        fixed.body_sites = chart::body_sites(w, &body);
    }
    fixed
}

impl ActionProvider for SurfaceActProvider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        let parameters = parameters_of(invocation)?;
        tv_of(parameters)?;
        ordinal_of(parameters)?;
        act_param(parameters)?;
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
        // Doc 20 cut SM.4a (the brief's reviewer finding F5): each site's composite is peeked
        // once for the whole projection and shared by every rig's scan plan.
        let mut peeks: BTreeMap<CanonicalId, SitePeek> = BTreeMap::new();
        let mut fixed_rigs: BTreeMap<String, FixedRig> = BTreeMap::new();
        for rig in &rigs {
            let battery_id = rig_battery_id(w, rig.tv);
            let scanner = rig_scanner_id(w, rig.tv).and_then(|id| scanner_of(&self.tools, &id));
            let mut plans: BTreeMap<String, FixedPlan> = BTreeMap::new();
            // A launch, a scan and a sounding need the battery (`needs_battery`: a launch
            // always, a reading act spends); a recover and a dump never do, so they plan whether
            // or not its row resolves (§4.1; the fixer round's item 3).
            let has_battery = battery_id.is_some();
            plans.insert(
                wire_word(ActKind::Launch, false).to_owned(),
                if has_battery {
                    FixedPlan::of(launch_plan(w, actor, rig))
                } else {
                    FixedPlan::refused("tv_dark")
                },
            );
            plans.insert(
                wire_word(ActKind::Recover, false).to_owned(),
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
            plans.insert(wire_word(ActKind::Dump, false).to_owned(), dump);
            let scan = scanner.map(|scanner| scan_plan(w, actor, rig, scanner, &mut peeks));
            plans.insert(
                wire_word(ActKind::Scan, false).to_owned(),
                reading_fixed(w, scan, has_battery),
            );
            for quiet in [false, true] {
                let sound = scanner.map(|scanner| sound_plan(w, actor, rig, scanner, quiet));
                plans.insert(
                    wire_word(ActKind::Sound, quiet).to_owned(),
                    reading_fixed(w, sound, has_battery),
                );
            }
            fixed_rigs.insert(
                rig.tv.0.to_string(),
                FixedRig {
                    slot: rig.slot,
                    hull: rig.hull,
                    battery_id,
                    plans,
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
        let parameters = vec![
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
                    choices: ACT_WIRE_WORDS.iter().map(|w| (*w).to_owned()).collect(),
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
                "tacsim-surface-act",
                "Work a rig",
                "Send a rig down the ramp, scan or sound the ground, bring it in, or empty its \
                 hopper onto the spoil.",
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
        let (kind, quiet) = act_param(parameters)?;
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
        let plan = rig
            .plans
            .get(wire_word(kind, quiet))
            .cloned()
            .ok_or_else(|| invalid("that act is not planned"))?;
        if !plan.ok {
            return Err(port_error(
                ActionErrorCode::ProviderRejected,
                code_of_word(plan.code.as_deref().unwrap_or("tv_dark")),
            ));
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
        // 4. The launch's open excursion and the caps — a launch's, and a scan's or a
        // sounding's that implies one (doc 20 cut SM.4a, lead ruling 2); a recover from the bay
        // closes one.
        let launches = kind == ActKind::Launch || (kind.reads_ground() && plan.from_bay);
        if launches {
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
                        CapRefused::Operator => "you have as many rigs out as one pilot may run",
                        CapRefused::Site => "that site has as many rigs out as it holds",
                        CapRefused::World => "the Registry has as many rigs out as it allows",
                    },
                ));
            }
        }
        if kind == ActKind::Recover && plan.from_bay && record.excursion.is_none() {
            return Err(port_error(
                ActionErrorCode::ProviderRejected,
                "that rig is in the bay",
            ));
        }
        let mut params = plan
            .params
            .clone()
            .ok_or_else(|| invalid("an act's plan names its parameters"))?;
        // 5. The chart (doc 20 cut SM.4a; lead ruling 3, D4, D3): the same-cell rule from the
        // fixed plan's frozen stand, then the body caps over the actor's rows on the body. The
        // chart is not written by the start.
        if let Some(reach) = Reach::of(&params) {
            if plan.body_sites.len() > chart::MAX_CHART_SITES_PER_BODY {
                return Err(invalid(
                    "a reading act's offer names a bounded set of sites",
                ));
            }
            let row = read_chart(txn, &self.charts, actor, &site)?;
            if reach.refused(row.as_ref()) {
                return Err(port_error(
                    ActionErrorCode::ProviderRejected,
                    code_of_word(Refusal::NothingNew.word()),
                ));
            }
            let mut rows = Vec::new();
            for other in &plan.body_sites {
                if let Some(found) = read_chart(txn, &self.charts, actor, other)? {
                    rows.push(found);
                }
            }
            if let Some(row) = row
                && !plan.body_sites.contains(&site)
            {
                rows.push(row);
            }
            let (fields, blocks) = reach.units();
            if !body_admits(&rows, &site, &fields, &blocks) {
                return Err(port_error(
                    ActionErrorCode::ProviderRejected,
                    code_of_word(
                        Refusal::ChartFull {
                            body: String::new(),
                        }
                        .word(),
                    ),
                ));
            }
        }
        // 6. The reserve rule (§4.1; lead ruling 3): here and nowhere else, from the charge the
        // act will spend from (an implied launch: the bay credited no further than the launch
        // half, D22). With no battery the act spends nothing (`needs_battery` refused every
        // spend above), which the rule always passes.
        if let Some(battery) = battery
            && reserve_check(
                charge_for_act(
                    &record.charge,
                    now,
                    kind.reads_ground() && plan.from_bay,
                    battery,
                ),
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
        // 7. A dump freezes what the hopper holds now: each pool's **available** quantity
        // (its balance less every live hold), the one figure the finish compares and the
        // inventory's `transfer` can move — freezing the balance would refuse every dump of
        // a pool with a hold on it as `HopperChanged` (review finding tests/T4).
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
        // 8. The write: the act, its due instant and the charge it reserves; a launch (or the
        // launch a reading act implies) opens the excursion. The charge itself is not written
        // (lazy).
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
        if launches {
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
                "act": wire_word(kind, quiet),
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

/// A principal's chart row for a site, cloned; `None` when absent. A row under another key
/// fails closed (the families rule: readers re-check the key).
fn read_chart(
    txn: &mut ActionTxn<'_>,
    family: &StateFamilyHandle<ChartKey, ChartRecord>,
    principal: ObjId,
    site: &CanonicalId,
) -> Result<Option<ChartRecord>, ActionError> {
    let key = ChartKey {
        principal,
        site: site.clone(),
    };
    let found = txn
        .get(family, &key)
        .map_err(|_| port_error(ActionErrorCode::Integrity, "the charts are unavailable"))?
        .cloned();
    if let Some(row) = &found
        && row.key() != key
    {
        return Err(port_error(
            ActionErrorCode::Integrity,
            "a chart row disagrees with its own key",
        ));
    }
    Ok(found)
}

// ---- tacsim_surface_done: the finish ------------------------------------------------------------

/// **`tacsim_surface_done`** — every act's finish (the brief's 3.4).
pub(crate) struct SurfaceDoneProvider {
    pub action: DefinitionRef,
    pub outcome: DefinitionRef,
    pub excursions: StateFamilyHandle<RigKey, RigRecord>,
    pub charts: StateFamilyHandle<ChartKey, ChartRecord>,
    pub inventory: InventoryFamilies,
    pub tools: Arc<BTreeMap<CanonicalId, ToolDef>>,
}

/// **An implied launch's settlement** (doc 20 cut SM.4a; the brief's reviewer finding F1,
/// D22; §4.1): the bay credits charge up to the end of the launch half (`started_ic +
/// LAUNCH_S`) and no further; the act's cost then comes off out of the bay, stamped at the
/// act's `due_ic`. SM.3b's [`spent`] — which would credit the bay for the whole act — is never
/// called on a scan or a sounding from the bay.
fn spent_after_launch(
    charge: &Charge,
    act: &Act,
    battery: &BatteryDef,
) -> Result<Charge, ActionError> {
    let launched_at = act.started_ic.saturating_add(i64::from(LAUNCH_S));
    let value = charge_at(charge, launched_at, battery)
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
        in_bay: false,
    })
}

/// **An implied launch refused `Moved`** (the SM.4a fixer round's finding F1; §4.1, D22): the
/// rig left the bay (a staff move) and its excursion stays open, so its charge leaves the bay
/// as [`spent_after_launch`]'s would — the bay's credit up to `started_ic + LAUNCH_S` and no
/// further — with nothing spent (the act did not run), stamped at the act's `due_ic`, out of
/// the bay. Without the battery's row no credit is read at all (the stored value stands): a
/// rig never gains charge it cannot be shown to have. A charge already out of the bay is
/// unchanged.
fn left_after_launch(charge: &Charge, act: &Act, battery: Option<&BatteryDef>) -> Charge {
    if !charge.in_bay {
        return *charge;
    }
    let launched_at = act.started_ic.saturating_add(i64::from(LAUNCH_S));
    Charge {
        value_cu: battery.map_or(charge.value_cu, |battery| {
            charge_at(charge, launched_at, battery)
        }),
        at_ic: act.due_ic,
        in_bay: false,
    }
}

/// **What a reading act's finish wrote to the chart**: its span on the outcome.
fn write_reading(
    txn: &mut ActionTxn<'_>,
    charts: &StateFamilyHandle<ChartKey, ChartRecord>,
    act: &Act,
) -> Result<ReadSpan, ActionError> {
    let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
    let body = match &act.params {
        ActParams::Scan { body, .. } | ActParams::Sound { body, .. } => body.clone(),
        ActParams::None {} | ActParams::Dump { .. } => {
            return Err(integrity("a reading act carries its reach"));
        }
    };
    let existing = read_chart(txn, charts, act.operator, &act.site)?;
    let existed = existing.is_some();
    let mut row =
        existing.unwrap_or_else(|| ChartRecord::new(act.operator, act.site.clone(), body));
    let at = act.due_ic;
    let span = match &act.params {
        ActParams::Scan {
            cell,
            footing,
            fields,
            hidden,
            ..
        } => {
            write_scan(&mut row, *cell, fields, *footing, at);
            ReadSpan::Scan {
                cell: *cell,
                footing: *footing,
                fields: fields.clone(),
                hidden: hidden.clone(),
            }
        }
        ActParams::Sound {
            cell,
            radius,
            quiet,
            ..
        } => {
            let pockets = pockets_in_reach((cell[0], cell[1]), i64::from(*radius));
            write_sound(&mut row, *cell, &pockets, *quiet, at);
            // The heard budget (D7): one heard line a sounder an IC hour on this site; a quiet
            // sounding is never heard and never touches it.
            let heard = !*quiet && heard_line_due(row.heard_ic, at);
            if heard {
                row.heard_ic = Some(at);
            }
            ReadSpan::Sound {
                cell: *cell,
                radius: *radius,
                quiet: *quiet,
                heard,
            }
        }
        ActParams::None {} | ActParams::Dump { .. } => {
            return Err(integrity("a reading act carries its reach"));
        }
    };
    row.check()
        .map_err(|_| integrity("the chart row is out of bounds"))?;
    let key = row.key();
    let staged = if existed {
        txn.replace(charts, &key, row)
    } else {
        txn.insert(charts, &key, row)
    };
    staged.map_err(|_| integrity("the chart row did not stage"))?;
    Ok(span)
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
        // it a zero spend that does not leave the bay settles (`settle_free`), and — since cut
        // SM.4a's pre-commit round (item 2) — so does a spend that stays out of the bay (a scan
        // or a sounding from the ground: `settle_spent_out_of_bay`, which is what `spent` gives
        // out of the bay), so a recover and a ground reading land with the row unresolvable and
        // a launch (or an implied launch) fails closed (§4.1 "never stranded by its battery";
        // the fixer round's item 3). A dump never touches it.
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
                None if !charge.in_bay && !in_bay => {
                    settle_spent_out_of_bay(charge, act.reserved_cu, act.due_ic).ok_or_else(|| {
                        port_error(
                            ActionErrorCode::Integrity,
                            "the act spent more than the charge",
                        )
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
        let mut span: Option<ReadSpan> = None;
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
                    // Doc 20 cut SM.6 (lead ruling 5; SM.3b's round F3): ONE
                    // `transfer_many`, which stages the spoil's pool census once for every
                    // new pool, so a two-ore hopper dumps onto a fresh spoil in one bundle.
                    // A refusal here cannot be recorded in this bundle (a debit may be
                    // staged): the whole bundle is refused, nothing is written, and the
                    // tick's next finish records `SpoilGone` (`failed`).
                    let moves: Vec<(CanonicalId, u64)> = goods
                        .iter()
                        .filter(|(_, qty)| **qty > 0)
                        .map(|(good, qty)| (good.clone(), *qty))
                        .collect();
                    if !moves.is_empty() {
                        self.inventory
                            .transfer_many(txn, hopper, spoil, &moves)
                            .map_err(|error| {
                                tracing::warn!(
                                    tv = tv.0,
                                    act = ordinal,
                                    spoil = spoil_vault,
                                    goods = moves.len(),
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
            // Doc 20 cut SM.4a (D14, D22): a scan or a sounding is `Done` when the rig stands,
            // at landing, on the act's site at the frozen stand cell — the tick placed an implied
            // launch's rig at the ramp — and then the charge settles out of the bay and the
            // operator's chart is written in this bundle, stamped at the act's own `due_ic`. An
            // implied launch whose rig never left the bay is `NeverLeft` (the excursion closes,
            // the launch's rule); anything else is `Moved`. Neither refusal spends charge or
            // writes the chart.
            ActKind::Scan | ActKind::Sound => {
                let from_bay = act.params.launches();
                let stand = act.params.stand();
                let at_stand = matches!(
                    &rig.facts,
                    Facts::Ground { site: Some(site), cell: Some(cell), .. }
                        if *site == act.site && Some(*cell) == stand
                );
                if at_stand {
                    record.charge = if from_bay {
                        match battery {
                            Some(battery) => spent_after_launch(&record.charge, &act, battery)?,
                            None => {
                                return Err(port_error(
                                    ActionErrorCode::Integrity,
                                    "that rig cannot be read",
                                ));
                            }
                        }
                    } else {
                        settle(&record.charge, false)?
                    };
                    span = Some(write_reading(txn, &self.charts, &act)?);
                    ActResult::Done
                } else if from_bay && matches!(&rig.facts, Facts::Bay { hull } if *hull == act.hull)
                {
                    record.excursion = None;
                    ActResult::Refused {
                        code: RefusedCode::NeverLeft,
                    }
                } else {
                    // An implied launch whose rig left the bay but not for its stand (a staff
                    // move before the due instant): the excursion the start opened stays open
                    // (the rig is out), so the charge leaves the bay too — credited to the end
                    // of the launch half and no further, nothing spent (the SM.4a fixer round's
                    // finding F1; §4.1: out of the bay charge only falls by acts). A rig found
                    // in some bay keeps its charge as it is.
                    if from_bay && !matches!(&rig.facts, Facts::Bay { .. }) {
                        record.charge = left_after_launch(&record.charge, &act, battery);
                    }
                    ActResult::Refused {
                        code: RefusedCode::Moved,
                    }
                }
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
            span,
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
            Refusal::NothingNew,
            Refusal::ChartFull {
                body: String::new(),
            },
            Refusal::GroundCold {
                site: String::new(),
                entry: CanonicalId::new("ahuna_port").unwrap_or_else(|_| panic!("an id")),
            },
            Refusal::InstrumentsDark,
            Refusal::NoFooting {
                n: 1,
                terrain: String::new(),
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
        assert_eq!(Refusal::NothingNew.line(), "You learn nothing new here.");
        assert_eq!(Refusal::NothingNew.code(), "you learn nothing new here");
    }

    /// **The start's `act` words parse** (doc 20 cut SM.4a, D23; the brief's reviewer finding
    /// F6): every wire word round-trips through `act_of_word`/`wire_word`; `sound_quiet` is a
    /// quiet sounding; `probe` (the retired test kind) and `survey` (the verb's alias only) do
    /// not parse; a sounding's quiet flag is spelled, every other kind's ignored.
    #[test]
    fn sm4a_the_wire_words_parse() {
        for word in ACT_WIRE_WORDS {
            let (kind, quiet) = act_of_word(word).unwrap_or_else(|| panic!("{word}"));
            assert_eq!(wire_word(kind, quiet), word);
        }
        assert_eq!(act_of_word("sound_quiet"), Some((ActKind::Sound, true)));
        assert_eq!(act_of_word("sound"), Some((ActKind::Sound, false)));
        assert_eq!(act_of_word("scan"), Some((ActKind::Scan, false)));
        for bad in ["probe", "survey", "quiet", "", "Scan"] {
            assert_eq!(act_of_word(bad), None, "{bad:?}");
        }
        assert_eq!(wire_word(ActKind::Scan, true), "scan");
        let mut sorted = ACT_WIRE_WORDS;
        sorted.sort_unstable();
        assert_eq!(sorted, ACT_WIRE_WORDS, "the Choice is sorted");
        // The start's parameter gate refuses the retired probe with the message naming the six.
        let parameters = |act: &str| {
            json!({ "tv": "900", "act": act, "ordinal": 1 })
                .as_object()
                .cloned()
                .unwrap_or_default()
        };
        let refused = act_param(&parameters("probe")).expect_err("probe is retired");
        assert!(
            refused
                .to_string()
                .contains("a surface act names launch, recover, dump, scan, sound or sound_quiet"),
            "{refused}"
        );
        assert_eq!(
            act_param(&parameters("sound_quiet")).ok(),
            Some((ActKind::Sound, true))
        );
    }

    /// **An implied launch settles at the launch half, never at the due instant** (doc 20 cut
    /// SM.4a, F1, D22): with a battery's figures and a charge under capacity in the bay,
    /// `spent_after_launch` leaves `charge_at(c, started + 300) − cost` out of the bay stamped
    /// at the due instant — strictly less than `charge_at(c, due) − cost`, which SM.3b's
    /// `spent` would credit — and equal to `charge_for_act` at the start less the cost.
    #[test]
    fn sm4a_spent_after_launch_credits_the_bay_only_for_the_launch_half() {
        let battery = crate::definitions::tools::tools_table()
            .unwrap_or_else(|e| panic!("{e}"))
            .values()
            .find_map(ToolDef::battery)
            .copied()
            .unwrap_or_else(|| panic!("a battery row"));
        let charge = Charge {
            value_cu: 20_000,
            at_ic: 1_000,
            in_bay: true,
        };
        let act = Act {
            ordinal: 1,
            kind: ActKind::Scan,
            operator: ObjId(42),
            hull: ObjId(500),
            slot: 1,
            site: CanonicalId::new("ahuna_port").unwrap_or_else(|_| panic!("an id")),
            started_ic: 1_000,
            due_ic: 1_900,
            reserved_cu: 1_000,
            params: ActParams::None {},
        };
        let settled =
            spent_after_launch(&charge, &act, &battery).unwrap_or_else(|e| panic!("{e:?}"));
        let launched = charge_at(&charge, 1_300, &battery);
        assert_eq!(
            settled,
            Charge {
                value_cu: launched - 1_000,
                at_ic: 1_900,
                in_bay: false
            }
        );
        assert!(settled.value_cu < charge_at(&charge, 1_900, &battery) - 1_000);
        assert_eq!(
            settled.value_cu,
            charge_for_act(&charge, act.started_ic, true, &battery) - act.reserved_cu
        );
        let ruinous = Act {
            reserved_cu: launched + 1,
            ..act
        };
        assert!(spent_after_launch(&charge, &ruinous, &battery).is_err());
    }

    /// **An implied launch refused `Moved` leaves the bay at the launch half, spending nothing**
    /// (the SM.4a fixer round's finding F1): the charge is `charge_at(c, started + 300)` out of
    /// the bay at the due instant — never the bay's credit for the whole act, never less the
    /// act's cost; without the battery's row the stored value stands (no credit), still out of
    /// the bay; a charge already out of the bay is unchanged.
    #[test]
    fn sm4a_a_moved_implied_launch_leaves_the_bay_at_the_launch_half() {
        let battery = crate::definitions::tools::tools_table()
            .unwrap_or_else(|e| panic!("{e}"))
            .values()
            .find_map(ToolDef::battery)
            .copied()
            .unwrap_or_else(|| panic!("a battery row"));
        let charge = Charge {
            value_cu: 20_000,
            at_ic: 1_000,
            in_bay: true,
        };
        let act = Act {
            ordinal: 1,
            kind: ActKind::Sound,
            operator: ObjId(42),
            hull: ObjId(500),
            slot: 1,
            site: CanonicalId::new("ahuna_port").unwrap_or_else(|_| panic!("an id")),
            started_ic: 1_000,
            due_ic: 2_500,
            reserved_cu: 2_000,
            params: ActParams::None {},
        };
        let left = left_after_launch(&charge, &act, Some(&battery));
        assert_eq!(
            left,
            Charge {
                value_cu: charge_at(&charge, 1_300, &battery),
                at_ic: 2_500,
                in_bay: false
            }
        );
        assert!(
            left.value_cu > charge.value_cu,
            "the launch half's credit shows"
        );
        assert!(left.value_cu < charge_at(&charge, 2_500, &battery));
        assert_eq!(
            left_after_launch(&charge, &act, None),
            Charge {
                value_cu: 20_000,
                at_ic: 2_500,
                in_bay: false
            }
        );
        let out = Charge {
            in_bay: false,
            ..charge
        };
        assert_eq!(left_after_launch(&out, &act, Some(&battery)), out);
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
