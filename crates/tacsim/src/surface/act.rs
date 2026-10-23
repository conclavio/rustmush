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
//! **Doc 20 cut SM.4b** adds the **drive** on the same two actions (`act` word `drive`): the
//! command (`drive_cmd`) runs the cheap gates first, then ONE A* (`drive_plan`, through
//! `crate::surface::drive`) and the reserve with the real trip home, writes the drive's order to
//! a RAM slot (`ActOrders`, SM.4b's `DriveOrders`: the legal-action query carries no
//! parameters, so a typed target cannot reach the offer, and a player-supplied leg cannot be
//! trusted) and submits; the start's offer fixes that order after re-running the stand gate, and
//! the prepare binds it to the invocation's ordinal. A **recover** from farther than beside the
//! ramp carries its leg home in the same one act (`recover_plan`), checked by `leg_affordable` —
//! never the 5 % floor — and the scan's and the sounding's reserve now carry the real trip home
//! from their stand (`home_here`). No act reads a licence or a tenure (lead ruling 9).
//!
//! **Doc 20 cut SM.7** adds **mining** on the same two actions (`act` words `mine` and
//! `mine_stop`; no third action, SM.3b's decision 25): `mine_cmd` plans the run from the
//! operator's own reading (`mine_plan`: the stand, the lease, the ground, **the licence and the
//! tenure** — mining is the act that reads rights, §8 — the record, the pocket's locks, the
//! reading and the family, the truth server-side, the fit, the body caps), writes the order to
//! `ActOrders` and submits; the start's offer re-plans and requires it equal; the prepare writes
//! the run into the record's **cycle slot** (`RigRecord::run`) and locks the pocket's cursor rows
//! in one bundle. Each cycle's finish is `tacsim_surface_done` with a `cycle` parameter (keyed
//! `tacsim.surface/{tv}/{ordinal}/cycle/{k}`): the family's ore moved by
//! `InventoryFamilies::transfer` from the block vault to the hopper, the cursor's `drawn_t`
//! raised by the same tonnes, the charge spent, the operator's core written to the chart, and the
//! run re-armed or closed — one bundle. While a run drills only a scan or a sounding may start.
//!
//! **Doc 20 cut SM.8** adds the **unload** on the same two actions (`act` word `unload`; no
//! third action): a `Done` recover with ore in the hopper writes a durable queued unload on the
//! rig's record in its own bundle, which the surface tick starts as the operator (`how: queued`,
//! keyed `tacsim.surface/{tv}/{ordinal}/queued`); `unload_cmd` (`+tv/<n> unload [<t>] [go]`)
//! files one by hand (`how: whole` or `part` with `qty`, after writing an `ActOrders` entry). The
//! start's offer fixes both possible sources and the prepare runs the one `how` names,
//! re-checking the hopper and the hold's unreserved room in the transaction; the finish moves the
//! hopper's goods into the hold by ONE `InventoryFamilies::transfer_many_marked` (a marker a
//! good). No act of the unload reads a licence, a tenure, a lease or the battery. Every start
//! clears a queued unload.
//!
//! **Doc 20 cut SM.11** adds the fill plan on the same two actions (no third; the brief's lead
//! rulings 1–4, decisions D10 and D17): `plan_cmd` (`+tv/<n> plan <list> [family=<good>] [go]`)
//! compiles the list (`crate::surface::plan::compile`, the start's offer the same function),
//! writes an `ActOrders` entry and submits `act: plan_file`; `plan_show_cmd` reads it back;
//! `plan_clear_cmd` ends a live plan (`plan_clear`) or strikes an ended one (`plan_strike`) —
//! two writes on two keys. Every step of a plan is one of the existing acts, submitted by the
//! surface tick as the author with the server-authored start parameters `by: plan`, `plan` and
//! `step`, which the offer fixes from the durable record (`crate::surface::plan::decide`) — never
//! a RAM order — and the prepare holds the start to; a plan's end is `act: plan_end`. The idle
//! return is a recover with `by: idle`. A live plan holds its rig from every hand start
//! (`plan_live`). The finish copies who set an act going onto its outcome, writes a plan's
//! readings `via: Plan` and counts the plan's tally.
//!
//! Neither provider's prepare reads a clock: `now` is the offer's IC instant. The start's offer
//! never reads `World::now`; since cut SM.8 the finish's offer fixes the real second (`wall`) as
//! well as the IC `clock` — the market lot offer's precedent — for the hopper's marker ring
//! (`InventoryFamilies::transfer_many_marked` stamps its ring entries with it), so the finish's
//! fixed parameters are not a pure function of IC time.

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

use crate::definitions::tools::{BatteryDef, DrillDef, ScannerDef, ToolDef};
use crate::port::capacity::Load;
use crate::port::coop::{name_of, port_name};
use crate::port::def::PortDef;
use crate::port::families::PortHandles;
use crate::port::map::PortMap;
use crate::port::ship::{Berth, HullView, berth_of, hull_view};
use crate::port::site_map::SurfaceReads;
use crate::port::tv::{self, TvLease, TvLeaseStatus};
use crate::port_tick::port_error;
use crate::store::TacsimStore;
use crate::surface::address::{self, cell_on_site, pockets_in_reach};
use crate::surface::chart::{
    self, ChartKey, ChartRecord, SitePeek, body_admits, heard_line_due, repeat_refused, scan_adds,
    sound_adds, write_scan, write_sound,
};
use crate::surface::cursor::{PocketCursor, PocketLock};
use crate::surface::drive::{self, DriveRates, Leg, LegRefused, Target};
use crate::surface::excursion::{
    Act, ActKind, ActParams, ActResult, CapRefused, Charge, DUMP_S, Excursion, ExcursionCounts,
    ExcursionId, LAUNCH_S, MAX_DUMP_GOODS, Outcome, RECOVER_S, ReadSpan, RefusedCode, RigKey,
    RigRecord, cap_refusal, cells_apart, charge_at, charge_for_act, leg_affordable, needs_battery,
    ramp_pose, reserve_check, reserved_total, settle_free, settle_spent_out_of_bay, site_cell,
};
use crate::surface::excursion::{DarkCause, LicenceEnd, LockRow, MAX_LOCK_ROWS, Mined, Run};
use crate::surface::excursion::{Queued, UNLOAD_S, UnloadHow};
use crate::surface::geology::{OreFamily, Word, by_family};
use crate::surface::mine::{
    Bound, MineChoice, MinePlan, RightsVerdict, TenureRead, blind_room_t, class_yield,
    cursor_after, cursor_locked, cursor_released, cycle_secs, cycle_yield, fit, lock_live,
    pocket_yields, run_end,
};
use crate::surface::plan::{
    ActBy, FillPlan, FillState, MAX_PLAN_NOTES, MAX_PLAN_STEPS, PLAN_BOUND_IC_SECS, PlanEnd,
    PlanNote, PlanNoteKind, PlanStep, PlanTally, PlanUpdate,
};
use crate::surface::reading::Footing;
use crate::surface::unload::{Bound as ShortBound, Short};
use crate::surface::{HomePeek, Peeks};

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
/// sorted. Doc 20 cut SM.7 adds `mine` and `mine_stop`; doc 20 cut SM.8 `unload`; doc 20 cut
/// SM.11 the plan's four controls `plan_clear`, `plan_end`, `plan_file` and `plan_strike` (none an
/// act: each writes the plan and never the act slot, `mine_stop`'s precedent).
pub const ACT_WIRE_WORDS: [&str; 14] = [
    "drive",
    "dump",
    "launch",
    "mine",
    "mine_stop",
    "plan_clear",
    "plan_end",
    "plan_file",
    "plan_strike",
    "recover",
    "scan",
    "sound",
    "sound_quiet",
    "unload",
];

/// **The start's `act` parameter** (doc 20 cut SM.4a; the brief's reviewer finding F6, D23):
/// `launch`, `recover`, `dump`, `scan`, `sound` and `sound_quiet` — the kind and the sounding's
/// quiet flag — and since doc 20 cut SM.4b `drive`, since SM.7 `mine`, since SM.8 `unload`.
/// `mine_stop` is no act ([`start_word`]).
#[must_use]
pub(crate) fn act_of_word(word: &str) -> Option<(ActKind, bool)> {
    match word {
        "sound_quiet" => Some((ActKind::Sound, true)),
        "launch" | "recover" | "dump" | "scan" | "sound" | "drive" | "mine" | "unload" => {
            ActKind::from_word(word).map(|kind| (kind, false))
        }
        _ => None,
    }
}

/// **What a start's `act` word asks** (doc 20 cut SM.7; the brief's 3.4): an act (its kind and a
/// sounding's quiet flag), or `mine_stop` — a control on a committed run, no act.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StartWord {
    /// An act.
    Act {
        /// What it is.
        kind: ActKind,
        /// A quiet sounding.
        quiet: bool,
    },
    /// `+tv/mine stop`.
    MineStop,
    /// **Doc 20 cut SM.11**: `+tv/plan … go` — the filing.
    PlanFile,
    /// `+tv/plan clear` on a live plan.
    PlanClear,
    /// `+tv/plan clear` on an ended plan: it is struck from the record.
    PlanStrike,
    /// The surface tick's end of a plan.
    PlanEnd,
}

/// **The start's word** (doc 20 cut SM.7): [`act_of_word`]'s acts, `mine_stop` and (doc 20 cut
/// SM.11) the plan's four controls. Both the start's `validate_invocation` and its prepare read
/// it.
#[must_use]
pub(crate) fn start_word(word: &str) -> Option<StartWord> {
    match word {
        "mine_stop" => return Some(StartWord::MineStop),
        "plan_file" => return Some(StartWord::PlanFile),
        "plan_clear" => return Some(StartWord::PlanClear),
        "plan_strike" => return Some(StartWord::PlanStrike),
        "plan_end" => return Some(StartWord::PlanEnd),
        _ => {}
    }
    act_of_word(word).map(|(kind, quiet)| StartWord::Act { kind, quiet })
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

/// **A mining cycle's finish key** (doc 20 cut SM.7; the brief's decision R2: the receipt
/// `(deposit, pocket, k)` as the replay guard, over `(tv, run ordinal, k)` — the run froze its
/// deposit and pocket, and a second run of the same pocket restarts `k`):
/// `tacsim.surface/{tv}/{ordinal}/cycle/{k}`.
#[must_use]
pub fn cycle_key(tv: ObjId, ordinal: u32, k: u8) -> Option<IdempotencyKeyText> {
    IdempotencyKeyText::parse_canonical(format!("{SURFACE_KEY_PREFIX}{}/{ordinal}/cycle/{k}", tv.0))
        .ok()
}

/// **`+tv/mine stop`'s key** (doc 20 cut SM.7): `tacsim.surface/{tv}/{ordinal}/stop`, the run's
/// ordinal — one stop a run.
#[must_use]
pub fn stop_key(tv: ObjId, ordinal: u32) -> Option<IdempotencyKeyText> {
    IdempotencyKeyText::parse_canonical(format!("{SURFACE_KEY_PREFIX}{}/{ordinal}/stop", tv.0)).ok()
}

/// **A queued unload's start key** (doc 20 cut SM.8; the brief's decision R15):
/// `tacsim.surface/{tv}/{ordinal}/queued` — the surface tick's own request, so it never meets a
/// hand `+tv/unload go`'s under [`act_key`] (the two race as distinct requests and the prepare's
/// ordinal check settles them).
#[must_use]
pub fn queued_key(tv: ObjId, ordinal: u32) -> Option<IdempotencyKeyText> {
    IdempotencyKeyText::parse_canonical(format!("{SURFACE_KEY_PREFIX}{}/{ordinal}/queued", tv.0))
        .ok()
}

/// **A plan's filing key** (doc 20 cut SM.11; the brief's 3.6): `tacsim.surface/{tv}/plan/{no}/file`
/// — one filing a plan number.
#[must_use]
pub fn plan_file_key(tv: ObjId, no: u32) -> Option<IdempotencyKeyText> {
    IdempotencyKeyText::parse_canonical(format!("{SURFACE_KEY_PREFIX}{}/plan/{no}/file", tv.0)).ok()
}

/// **A live plan's clear key** (the brief's D17; the reviewer's F1):
/// `tacsim.surface/{tv}/plan/{no}/clear/{seq}` — while the plan is live its `seq` moves with every
/// plan start, so the same `(no, seq)` always carries the same parameters: the clear names no act
/// of the record ([`PLAN_CLEAR_ORDINAL`], the lead's pre-commit round, item 2).
#[must_use]
pub fn plan_clear_key(tv: ObjId, no: u32, seq: u32) -> Option<IdempotencyKeyText> {
    IdempotencyKeyText::parse_canonical(format!(
        "{SURFACE_KEY_PREFIX}{}/plan/{no}/clear/{seq}",
        tv.0
    ))
    .ok()
}

/// **The `ordinal` a clear carries** (the lead's pre-commit round, item 2; the judge's
/// correctness/F6): the start's offer requires the parameter, but a clear names no act of the
/// record and its prepare never reads it — so it is this constant, never the record's ordinal,
/// which SM.8's queued unload start moves without moving the plan's `seq`. Two clears typed around
/// such a landing carry the same key and the same parameters: the second is the store's Replay of
/// the first, never its idempotency conflict.
pub const PLAN_CLEAR_ORDINAL: u32 = 1;

/// **An ended plan's strike key** (D17): `tacsim.surface/{tv}/plan/{no}/strike/{ordinal}` — hand
/// acts may move the ordinal under an ended plan, so a retry after one is a new key.
#[must_use]
pub fn plan_strike_key(tv: ObjId, no: u32, ordinal: u32) -> Option<IdempotencyKeyText> {
    IdempotencyKeyText::parse_canonical(format!(
        "{SURFACE_KEY_PREFIX}{}/plan/{no}/strike/{ordinal}",
        tv.0
    ))
    .ok()
}

/// **A plan's end key** (the brief's 3.8): `tacsim.surface/{tv}/plan/{no}/end` — one end a plan.
#[must_use]
pub fn plan_end_key(tv: ObjId, no: u32) -> Option<IdempotencyKeyText> {
    IdempotencyKeyText::parse_canonical(format!("{SURFACE_KEY_PREFIX}{}/plan/{no}/end", tv.0)).ok()
}

/// **A plan step's start key** (the brief's 3.6, D9): `tacsim.surface/{tv}/{ordinal}/plan/{step}/
/// {word}` — the word in the key, because a step whose submission did not land may be re-decided
/// as another move at the same ordinal (a drive becomes a recover), and a reused key with other
/// parameters would meet the store's idempotency conflict.
#[must_use]
pub fn plan_step_key(tv: ObjId, ordinal: u32, step: u16, word: &str) -> Option<IdempotencyKeyText> {
    IdempotencyKeyText::parse_canonical(format!(
        "{SURFACE_KEY_PREFIX}{}/{ordinal}/plan/{step}/{word}",
        tv.0
    ))
    .ok()
}

/// **The idle return's key** (lead ruling 6): `tacsim.surface/{tv}/{ordinal}/idle`.
#[must_use]
pub fn idle_key(tv: ObjId, ordinal: u32) -> Option<IdempotencyKeyText> {
    IdempotencyKeyText::parse_canonical(format!("{SURFACE_KEY_PREFIX}{}/{ordinal}/idle", tv.0)).ok()
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

/// **Every rig `actor` leases, at the helm or not** (doc 20 cut SM.11; the lead's pre-commit round,
/// item 3): [`operator_rigs`]' rows without its helm filter — the plan's author reaches their own
/// plan from anywhere aboard (`+tv/plan show|clear`), and the plan's `Bound` end is submitted as
/// its author wherever they stand.
pub(crate) fn leased_rigs(w: &World, actor: ObjId) -> Vec<RigAt> {
    let Some(store) = w.module_state::<TacsimStore>() else {
        return Vec::new();
    };
    let mut rigs: Vec<RigAt> = store
        .tv_leases_of(actor)
        .filter(|record| record.live())
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
    /// `DRIVE_OFF_SITE` (doc 20 cut SM.4b): the target is off the site.
    OffSite {
        /// The site's name.
        site: String,
    },
    /// `DRIVE_HERE` (doc 20 cut SM.4b): the rig stands on the target's cell.
    Here {
        /// The slot.
        n: u8,
    },
    /// `NO_PATH` (doc 20 cut SM.4b; §8): no way over the ground within the site's budget.
    NoPath {
        /// The slot.
        n: u8,
        /// The target in words.
        target: String,
    },
    /// `TOO_WINDING` (doc 20 cut SM.4b; R1): more than 32 waypoints even at 80 m.
    Winding {
        /// The slot.
        n: u8,
        /// The target in words.
        target: String,
        /// The waypoints the widest freeze kept.
        needed: usize,
    },
    /// `NO_WAY_HOME` (doc 20 cut SM.4b; R2): the end stand has no way back to the ramp.
    NoWayHome {
        /// The slot.
        n: u8,
        /// The target in words.
        target: String,
        /// The egress's label.
        ramp: String,
    },
    /// `DRIVE_UNPLANNED` (doc 20 cut SM.4b; the brief's decision P2): a start of a drive with no
    /// order of the actor's for the rig (a forged start, or an order lost to a restart).
    Unplanned,
    /// `DRIVE_MOVED` (doc 20 cut SM.4b): the rig no longer stands where its drive's leg begins.
    Moved,
    /// `RECOVER_SHORT` (doc 20 cut SM.4b; lead ruling 5, decision P6): the rig's charge cannot
    /// cover its own leg home.
    LegShort {
        /// The slot.
        n: u8,
        /// The charge now, whole percent (`?` when the battery cannot be read).
        now: String,
        /// The leg's charge, whole percent rounded up (`?` likewise).
        need: String,
        /// The egress's label.
        ramp: String,
    },
    /// `RECOVER_DARK` (doc 20 cut SM.4b; R3): the way home's terrain cannot be mapped this boot.
    RecoverDark {
        /// The slot.
        n: u8,
        /// The egress's label.
        ramp: String,
    },
    /// `MINE_IN_BAY` (doc 20 cut SM.7; the brief's decision R11): the drill works only a pocket it
    /// stands on — no implied launch.
    MineInBay {
        /// The slot.
        n: u8,
    },
    /// `MINE_RUNNING` (doc 20 cut SM.7; decision R1): a run holds the rig; only a scan or a
    /// sounding may start beside it.
    MineRunning {
        /// The slot.
        n: u8,
        /// The run's pocket.
        addr: String,
        /// The cycle in flight.
        k: u8,
        /// The run's cycles.
        cycles: u8,
        /// When the cycle in flight is due, in words.
        when: String,
    },
    /// `MINE_NO_FOOTING` (doc 20 cut SM.7): the rig stands where the drill bites nothing.
    MineNoFooting {
        /// The slot.
        n: u8,
        /// The terrain's word.
        terrain: String,
    },
    /// **The licence** (doc 20 cut SM.7; lead ruling 5, reviewer F21): `MINE_NO_LICENCE` for no
    /// licence at all, else `+licence`'s own line for the standing
    /// (`crate::port::licence::standing_line`), formatted when the plan is made.
    Licence {
        /// The line.
        line: String,
    },
    /// `MINE_CLAIMED` (doc 20 §5, verbatim; lead ruling 5).
    Claimed,
    /// `MINE_LOCKED` (doc 20 §6, verbatim; lead ruling 3, decision R4).
    Locked,
    /// `MINE_HOPPER_FULL`: no cycle fits the hopper.
    HopperFull {
        /// The slot.
        n: u8,
        /// The hopper's words.
        hopper: String,
    },
    /// `MINE_HOLD_FULL`: no cycle fits the hold's unreserved room.
    HoldFull {
        /// The hull's name.
        name: String,
    },
    /// `MINE_WORKED_OUT`: the pocket's tonnes of the family are drawn.
    WorkedOut {
        /// The pocket's address.
        addr: String,
        /// The ore's words.
        ore: String,
    },
    /// `MINE_NO_FIT` (§6's line, with the slot): a named count that does not fit.
    NoFit {
        /// The slot.
        n: u8,
        /// The count named.
        m: u8,
        /// The count that fits.
        fit: u8,
        /// What bound it.
        bound: crate::surface::mine::Bound,
        /// The family word as typed, with a leading space, or empty.
        fam: String,
    },
    /// `MINE_BLIND_ROOM` (lead ruling 2): the only room line for an unread pocket.
    BlindRoom {
        /// The blind room, whole tonnes.
        room: u32,
    },
    /// `MINE_BLIND_COUNT`: a count named on an unread pocket.
    BlindCount {
        /// The slot.
        n: u8,
        /// The family word as typed, with a leading space, or empty.
        fam: String,
    },
    /// `MINE_NO_ORE`: the operator's reading shows none of the family named.
    NoOre {
        /// The pocket's address.
        addr: String,
        /// The ore's words.
        ore: String,
    },
    /// `MINE_ALL_BARREN`: the operator's reading shows the pocket barren.
    AllBarren {
        /// The pocket's address.
        addr: String,
    },
    /// `MINE_FORFEIT` (doc 20 cut SM.7; the brief's decision R15): the rig's deposit is forfeit.
    MineForfeit {
        /// The slot.
        n: u8,
        /// Occator's counter.
        counter: String,
        /// Its port.
        port: String,
    },
    /// `MINE_ENDED` (decision R15): the rig's economy lease ended.
    MineEnded {
        /// The slot.
        n: u8,
        /// Occator's counter.
        counter: String,
        /// Its port.
        port: String,
    },
    /// `MINE_CHANGED` (the brief's 3.4): the start's re-plan differs from the order's — `go`
    /// commits exactly the preview or nothing.
    MineChanged,
    /// `UNLOAD_UNDER_WAY` (doc 20 cut SM.8).
    UnloadUnderWay {
        /// The hull's name.
        name: String,
    },
    /// `UNLOAD_NOT_SITE` (doc 20 cut SM.8; decision R5): an unload works only on a site's pad.
    UnloadNotSite {
        /// The slot.
        n: u8,
        /// The hull's name.
        name: String,
    },
    /// `UNLOAD_OUT` (doc 20 cut SM.8; decision R9): the rig stands on the site's ground.
    UnloadOut {
        /// The slot.
        n: u8,
        /// The site's name.
        site: String,
    },
    /// `UNLOAD_NO_ROOM` (doc 20 cut SM.8; decision R18): the hold takes nothing of it.
    UnloadNoRoom {
        /// The slot.
        n: u8,
        /// The hull's name.
        name: String,
        /// The hopper's load in words.
        load: String,
    },
    /// `UNLOAD_NO_FIT` (doc 20 cut SM.8; decisions R10, R18): a named tonnage past the fit, which
    /// is at least 1.
    UnloadNoFit {
        /// The slot.
        n: u8,
        /// The tonnes named.
        q: u64,
        /// The tonnes that fit.
        fit: u64,
        /// The good's label.
        ore: String,
    },
    /// `UNLOAD_NO_STOCK` (doc 20 cut SM.8; the lead's pre-commit round, item 4): a named tonnage
    /// past the hopper's own stock of the first good, which is at least 1 — the hold's room was
    /// not what bound it.
    UnloadNoStock {
        /// The slot.
        n: u8,
        /// The tonnes of the good the hopper holds.
        fit: u64,
        /// The good's label.
        ore: String,
    },
    /// `UNLOAD_CHANGED` (doc 20 cut SM.8; §7.1 "re-checked at prepare"): the hopper or the hold's
    /// room changed between the preview and the start.
    UnloadChanged {
        /// The slot.
        n: u8,
    },
    /// `UNLOAD_MARKERS` (doc 20 cut SM.11, the lead's pre-commit round, item 1): the queued
    /// unload's start, while the hopper's marker ring could not record the move — the queue
    /// stands and the surface tick starts it again under its attempt rule.
    UnloadMarkers {
        /// The slot.
        n: u8,
    },
    /// `PLAN_LIVE` (doc 20 cut SM.11; the brief's D8): a live plan holds its rig from every hand
    /// start.
    PlanLive {
        /// The slot.
        n: u8,
        /// The plan's number.
        no: u32,
    },
    /// `PLAN_ALREADY` (doc 20 cut SM.11): filing over a live plan.
    PlanFiled {
        /// The slot.
        n: u8,
        /// The live plan's number.
        no: u32,
    },
    /// `PLAN_CHANGED` (doc 20 cut SM.11): the start's re-compile differs from the preview.
    PlanChanged {
        /// The slot.
        n: u8,
    },
    /// `PLAN_WAITS` (doc 20 cut SM.11): the plan's next step cannot start now.
    PlanWaits {
        /// The slot.
        n: u8,
    },
    /// `PLAN_CHANGED`'s line (doc 20 cut SM.11): the plan moved on between the decision and the
    /// landing.
    PlanMoved {
        /// The slot.
        n: u8,
    },
    /// `PLAN_NONE` (doc 20 cut SM.11).
    PlanNone {
        /// The slot.
        n: u8,
    },
    /// `PLAN_ALREADY_ENDED` (doc 20 cut SM.11): a clear landing on a plan that ended by itself.
    PlanEnded {
        /// The slot.
        n: u8,
        /// The plan's number.
        no: u32,
    },
}

/// The offer-wire words and the start's messages, in one table (the prepare's
/// `port_error` needs a `&'static str`).
const REFUSAL_CODES: [(&str, &str); 64] = [
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
    ("off_site", "that point is off the site"),
    ("here", "the rig is there already"),
    ("no_path", "there is no way there over the ground"),
    ("winding", "that way winds too much to drive in one leg"),
    (
        "no_way_home",
        "the rig could not get back to the ramp from there",
    ),
    // Doc 20 cut SM.7 (reviewer F7): a mine order lost to a restart, or replaced by the rig's
    // newer drive order, must not be told about a drive.
    ("unplanned", "that act was not planned; preview it again"),
    ("moved", "that rig has moved since the drive was planned"),
    ("leg_short", "the rig's charge cannot cover the drive"),
    (
        "recover_dark",
        "the rig's way back cannot be mapped just now",
    ),
    // Doc 20 cut SM.7: mining's sixteen.
    ("mine_in_bay", "the rig is in the bay"),
    ("mine_running", "that rig is drilling"),
    (
        "mine_no_footing",
        "the rig stands where the drill bites nothing",
    ),
    ("unlicensed", "the operator holds no licence in force"),
    ("claimed", "that block is under claim"),
    ("locked", "another rig is working that pocket"),
    ("hopper_full", "the hopper has no room for another cycle"),
    (
        "hold_full",
        "the hold has no unreserved room for another cycle",
    ),
    ("worked_out", "that pocket is worked out"),
    ("no_fit", "those cycles do not fit"),
    (
        "blind_room",
        "an unread pocket needs room for the richest cycle",
    ),
    ("blind_count", "an unread pocket is cored one cycle"),
    ("no_ore", "the reading shows no ore to mine"),
    ("mine_forfeit", "the rig's deposit is forfeit"),
    ("mine_ended", "the rig's lease has ended"),
    (
        "mine_changed",
        "that run changed since its preview; preview it again",
    ),
    // Doc 20 cut SM.8: unloading's seven (the lead's pre-commit round added `unload_no_stock`).
    ("unload_under_way", "the hull is under way"),
    ("unload_not_site", "the hull is not on a site's pad"),
    ("unload_out", "the rig is not in the bay"),
    ("unload_no_room", "the hold has no room"),
    ("unload_no_fit", "that much does not fit the hold"),
    ("unload_no_stock", "the hopper holds less than that"),
    (
        "unload_changed",
        "the hopper or the hold's room changed; preview it again",
    ),
    // Doc 20 cut SM.11 (the lead's pre-commit round, item 1): the queued unload's start while the
    // hopper's unload log is full.
    (
        "unload_markers",
        "the rig's unload log is full; try again later",
    ),
    // Doc 20 cut SM.11: the fill plan's seven (prepare refusals, never a world plan's).
    ("plan_live", "that rig is working a plan"),
    ("plan_filed", "that rig is already working a plan"),
    (
        "plan_changed",
        "that plan changed since its preview; preview it again",
    ),
    ("plan_waits", "that rig's plan is waiting"),
    ("plan_moved", "the plan moved on"),
    ("plan_none", "that rig has no plan"),
    ("plan_ended", "that plan has already ended"),
];

/// **Every refusal word** (doc 20 cut SM.11; test support for the plan's refusal table, which
/// iterates the start's own list).
#[cfg(test)]
pub(crate) fn refusal_words() -> Vec<&'static str> {
    REFUSAL_CODES.iter().map(|(word, _)| *word).collect()
}

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
            Refusal::OffSite { .. } => "off_site",
            Refusal::Here { .. } => "here",
            Refusal::NoPath { .. } => "no_path",
            Refusal::Winding { .. } => "winding",
            Refusal::NoWayHome { .. } => "no_way_home",
            Refusal::Unplanned => "unplanned",
            Refusal::Moved => "moved",
            Refusal::LegShort { .. } => "leg_short",
            Refusal::RecoverDark { .. } => "recover_dark",
            Refusal::MineInBay { .. } => "mine_in_bay",
            Refusal::MineRunning { .. } => "mine_running",
            Refusal::MineNoFooting { .. } => "mine_no_footing",
            Refusal::Licence { .. } => "unlicensed",
            Refusal::Claimed => "claimed",
            Refusal::Locked => "locked",
            Refusal::HopperFull { .. } => "hopper_full",
            Refusal::HoldFull { .. } => "hold_full",
            Refusal::WorkedOut { .. } => "worked_out",
            Refusal::NoFit { .. } => "no_fit",
            Refusal::BlindRoom { .. } => "blind_room",
            Refusal::BlindCount { .. } => "blind_count",
            Refusal::NoOre { .. } | Refusal::AllBarren { .. } => "no_ore",
            Refusal::MineForfeit { .. } => "mine_forfeit",
            Refusal::MineEnded { .. } => "mine_ended",
            Refusal::MineChanged => "mine_changed",
            Refusal::UnloadUnderWay { .. } => "unload_under_way",
            Refusal::UnloadNotSite { .. } => "unload_not_site",
            Refusal::UnloadOut { .. } => "unload_out",
            Refusal::UnloadNoRoom { .. } => "unload_no_room",
            Refusal::UnloadNoFit { .. } => "unload_no_fit",
            Refusal::UnloadNoStock { .. } => "unload_no_stock",
            Refusal::UnloadChanged { .. } => "unload_changed",
            Refusal::UnloadMarkers { .. } => "unload_markers",
            Refusal::PlanLive { .. } => "plan_live",
            Refusal::PlanFiled { .. } => "plan_filed",
            Refusal::PlanChanged { .. } => "plan_changed",
            Refusal::PlanWaits { .. } => "plan_waits",
            Refusal::PlanMoved { .. } => "plan_moved",
            Refusal::PlanNone { .. } => "plan_none",
            Refusal::PlanEnded { .. } => "plan_ended",
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
            Refusal::OffSite { site } => tv::DRIVE_OFF_SITE.replace("{site}", site),
            Refusal::Here { n } => tv::DRIVE_HERE.replace("{n}", &n.to_string()),
            Refusal::NoPath { n, target } => tv::NO_PATH
                .replace("{n}", &n.to_string())
                .replace("{target}", target),
            Refusal::Winding { target, .. } => tv::TOO_WINDING
                .replace("{target}", target)
                .replace("{cap}", &crate::surface::MAX_LEG_WAYPOINTS.to_string()),
            Refusal::NoWayHome { n, target, ramp } => tv::NO_WAY_HOME
                .replace("{n}", &n.to_string())
                .replace("{target}", target)
                .replace("{ramp}", ramp),
            Refusal::Unplanned => tv::DRIVE_UNPLANNED.to_owned(),
            Refusal::Moved => tv::DRIVE_MOVED.to_owned(),
            Refusal::LegShort { n, now, need, ramp } => tv::RECOVER_SHORT
                .replace("{n}", &n.to_string())
                .replace("{now}", now)
                .replace("{need}", need)
                .replace("{ramp}", ramp),
            Refusal::RecoverDark { n, ramp } => tv::RECOVER_DARK
                .replace("{n}", &n.to_string())
                .replace("{ramp}", ramp),
            Refusal::MineInBay { n } => tv::MINE_IN_BAY.replace("{n}", &n.to_string()),
            Refusal::MineRunning {
                n,
                addr,
                k,
                cycles,
                when,
            } => tv::MINE_RUNNING
                .replace("{n}", &n.to_string())
                .replace("{addr}", addr)
                .replace("{k}", &k.to_string())
                .replace("{cycles}", &cycles.to_string())
                .replace("{when}", when),
            Refusal::MineNoFooting { n, terrain } => tv::MINE_NO_FOOTING
                .replace("{n}", &n.to_string())
                .replace("{terrain}", terrain),
            Refusal::Licence { line } => line.clone(),
            Refusal::Claimed => tv::MINE_CLAIMED.to_owned(),
            Refusal::Locked => tv::MINE_LOCKED.to_owned(),
            Refusal::HopperFull { n, hopper } => tv::MINE_HOPPER_FULL
                .replace("{n}", &n.to_string())
                .replace("{hopper}", hopper),
            Refusal::HoldFull { name } => tv::MINE_HOLD_FULL.replace("{name}", name),
            Refusal::WorkedOut { addr, ore } => tv::MINE_WORKED_OUT
                .replace("{addr}", addr)
                .replace("{ore}", ore),
            Refusal::NoFit {
                n,
                m,
                fit,
                bound,
                fam,
            } => {
                let ord = crate::surface::mine::ordinal_word(*fit);
                let why = match bound {
                    crate::surface::mine::Bound::Hopper => tv::FIT_HOPPER.replace("{ord}", ord),
                    crate::surface::mine::Bound::Hold => tv::FIT_HOLD.replace("{ord}", ord),
                    crate::surface::mine::Bound::WorkedOut => tv::FIT_POCKET.replace("{ord}", ord),
                    crate::surface::mine::Bound::Cap => tv::FIT_CAP.to_owned(),
                };
                tv::MINE_NO_FIT
                    .replace("{m}", &m.to_string())
                    .replace("{why}", &why)
                    .replace("{n}", &n.to_string())
                    .replace("{fam}", fam)
                    .replace("{fit}", &fit.to_string())
            }
            Refusal::BlindRoom { room } => tv::MINE_BLIND_ROOM.replace("{room}", &room.to_string()),
            Refusal::BlindCount { n, fam } => tv::MINE_BLIND_COUNT
                .replace("{n}", &n.to_string())
                .replace("{fam}", fam),
            Refusal::NoOre { addr, ore } => tv::MINE_NO_ORE
                .replace("{addr}", addr)
                .replace("{ore}", ore),
            Refusal::AllBarren { addr } => tv::MINE_ALL_BARREN.replace("{addr}", addr),
            Refusal::MineForfeit { n, counter, port } => tv::MINE_FORFEIT
                .replace("{n}", &n.to_string())
                .replace("{counter}", counter)
                .replace("{port}", port),
            Refusal::MineEnded { n, counter, port } => tv::MINE_ENDED
                .replace("{n}", &n.to_string())
                .replace("{counter}", counter)
                .replace("{port}", port),
            Refusal::MineChanged => tv::MINE_CHANGED.to_owned(),
            Refusal::UnloadUnderWay { name } => tv::UNLOAD_UNDER_WAY.replace("{name}", name),
            Refusal::UnloadNotSite { n, name } => tv::UNLOAD_NOT_SITE
                .replace("{name}", name)
                .replace("{n}", &n.to_string()),
            Refusal::UnloadOut { n, site } => tv::UNLOAD_OUT
                .replace("{n}", &n.to_string())
                .replace("{site}", site),
            Refusal::UnloadNoRoom { n, name, load } => tv::UNLOAD_NO_ROOM
                .replace("{name}", name)
                .replace("{n}", &n.to_string())
                .replace("{load}", load),
            Refusal::UnloadNoFit { n, q, fit, ore } => tv::UNLOAD_NO_FIT
                .replace("{q}", &q.to_string())
                .replace("{n}", &n.to_string())
                .replace("{fit}", &fit.to_string())
                .replace("{ore}", ore),
            Refusal::UnloadNoStock { n, fit, ore } => tv::UNLOAD_NO_STOCK
                .replace("{n}", &n.to_string())
                .replace("{fit}", &fit.to_string())
                .replace("{ore}", ore),
            Refusal::UnloadChanged { n } => tv::UNLOAD_CHANGED.replace("{n}", &n.to_string()),
            Refusal::UnloadMarkers { n } => tv::UNLOAD_MARKERS.replace("{n}", &n.to_string()),
            Refusal::PlanLive { n, no } => tv::PLAN_LIVE
                .replace("{n}", &n.to_string())
                .replace("{no}", &no.to_string()),
            Refusal::PlanFiled { n, no } => tv::PLAN_ALREADY
                .replace("{n}", &n.to_string())
                .replace("{no}", &no.to_string()),
            Refusal::PlanChanged { n } | Refusal::PlanMoved { n } => {
                tv::PLAN_CHANGED.replace("{n}", &n.to_string())
            }
            Refusal::PlanWaits { n } => tv::PLAN_WAITS
                .replace("{n}", &n.to_string())
                .replace("{why}", tv::WAIT_NEXT),
            Refusal::PlanNone { n } => tv::PLAN_NONE.replace("{n}", &n.to_string()),
            Refusal::PlanEnded { n, no } => tv::PLAN_ALREADY_ENDED
                .replace("{n}", &n.to_string())
                .replace("{no}", &no.to_string()),
        }
    }
}

/// **One sample of every refusal variant** (test support: `sm3b_every_refusal_word_has_a_message`
/// and the plan's refusal table iterate it).
#[cfg(test)]
pub(crate) fn refusal_samples() -> Vec<Refusal> {
    vec![
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
        Refusal::OffSite {
            site: String::new(),
        },
        Refusal::Here { n: 1 },
        Refusal::NoPath {
            n: 1,
            target: String::new(),
        },
        Refusal::Winding {
            n: 1,
            target: String::new(),
            needed: 33,
        },
        Refusal::NoWayHome {
            n: 1,
            target: String::new(),
            ramp: String::new(),
        },
        Refusal::Unplanned,
        Refusal::Moved,
        Refusal::LegShort {
            n: 1,
            now: "2".to_owned(),
            need: "3".to_owned(),
            ramp: String::new(),
        },
        Refusal::RecoverDark {
            n: 1,
            ramp: String::new(),
        },
        Refusal::MineInBay { n: 1 },
        Refusal::MineRunning {
            n: 1,
            addr: String::new(),
            k: 1,
            cycles: 4,
            when: String::new(),
        },
        Refusal::MineNoFooting {
            n: 1,
            terrain: String::new(),
        },
        Refusal::Licence {
            line: tv::MINE_NO_LICENCE.to_owned(),
        },
        Refusal::Claimed,
        Refusal::Locked,
        Refusal::HopperFull {
            n: 1,
            hopper: String::new(),
        },
        Refusal::HoldFull {
            name: String::new(),
        },
        Refusal::WorkedOut {
            addr: String::new(),
            ore: String::new(),
        },
        Refusal::NoFit {
            n: 1,
            m: 4,
            fit: 3,
            bound: Bound::Hopper,
            fam: String::new(),
        },
        Refusal::BlindRoom { room: 3 },
        Refusal::BlindCount {
            n: 1,
            fam: String::new(),
        },
        Refusal::NoOre {
            addr: String::new(),
            ore: String::new(),
        },
        Refusal::AllBarren {
            addr: String::new(),
        },
        Refusal::MineForfeit {
            n: 1,
            counter: String::new(),
            port: String::new(),
        },
        Refusal::MineEnded {
            n: 1,
            counter: String::new(),
            port: String::new(),
        },
        Refusal::MineChanged,
        Refusal::UnloadUnderWay {
            name: String::new(),
        },
        Refusal::UnloadNotSite {
            n: 1,
            name: String::new(),
        },
        Refusal::UnloadOut {
            n: 1,
            site: String::new(),
        },
        Refusal::UnloadNoRoom {
            n: 1,
            name: String::new(),
            load: String::new(),
        },
        Refusal::UnloadNoFit {
            n: 1,
            q: 9,
            fit: 3,
            ore: String::new(),
        },
        Refusal::UnloadNoStock {
            n: 1,
            fit: 3,
            ore: String::new(),
        },
        Refusal::UnloadChanged { n: 1 },
        Refusal::UnloadMarkers { n: 1 },
        Refusal::PlanLive { n: 1, no: 2 },
        Refusal::PlanFiled { n: 1, no: 2 },
        Refusal::PlanChanged { n: 1 },
        Refusal::PlanWaits { n: 1 },
        Refusal::PlanMoved { n: 1 },
        Refusal::PlanNone { n: 1 },
        Refusal::PlanEnded { n: 1, no: 2 },
    ]
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
    if let Err(bar) = lease_gate(w, &rig.record) {
        let (counter, port) = tv::counter_words(w);
        return Err(match bar {
            LeaseBar::Forfeit => Refusal::Forfeit {
                n: rig.slot,
                counter,
                port,
            },
            LeaseBar::Ended => Refusal::Ended {
                n: rig.slot,
                counter,
                port,
            },
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

/// **Why a rig's lease bars it** (doc 20 cut SM.7; the brief's 3.3 item 1, reviewer F11).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LeaseBar {
    /// The lease record is `Forfeit`.
    Forfeit,
    /// Its economy lease is not live.
    Ended,
}

/// **The lease gate** (SM.3a's operate gate R7, factored by doc 20 cut SM.7 so the launch and the
/// mining run read one rule — the brief's decision R15): the lease record `Forfeit`, then its
/// economy lease not live. **Recover, dump and drive never read it**: the way home is never
/// barred (§8, §19 item 3).
///
/// # Errors
///
/// The [`LeaseBar`], in that order.
pub(crate) fn lease_gate(w: &World, record: &TvLease) -> Result<(), LeaseBar> {
    if record.status == TvLeaseStatus::Forfeit {
        return Err(LeaseBar::Forfeit);
    }
    if !tv::rig_lease_live(w, record) {
        return Err(LeaseBar::Ended);
    }
    Ok(())
}

/// **The recover's world-side gate, in order** (the brief's 3.3; §7.1's placement half): the
/// layer, the view, under way, not at a site, not aboard as the launch; then the rig: in this
/// hull's bay → `Ok` with `from_bay` (the command and the prepare refuse `RECOVER_IN_BAY`
/// unless an excursion is still open, which this recover closes — decision 21); on another
/// domain, another hull's bay or elsewhere → `RECOVER_ELSEWHERE`; on the hull's site domain
/// beside the ramp (Chebyshev ≤ 1) → SM.3b's recover, `RECOVER_S`, no cost. No lease-state gate
/// (§8 / §19 item 3: rights never stop a rig's way home).
///
/// **Doc 20 cut SM.4b — farther off, the rig drives home first** (lead ruling 5; §3.6 of the
/// brief): the site's home field through the non-building peek (`peeks`; cold →
/// `GROUND_COLD`; it reads the keyless field when the site is dark, R3, so a recover never
/// refuses `INSTRUMENTS_DARK` — only a home whose terrain failed this boot reads `RECOVER_DARK`),
/// the descent from the rig's cell frozen by `drive::recover_leg` (never refused for its
/// waypoints), and ONE act of kind `Recover` whose params carry the leg to the beside-ramp cell
/// it ends on: `secs` the leg's then `RECOVER_S`, `cost_cu` the leg's charge, `home_cu` 0 (it IS
/// the way home). `rates` is the rig's (`rig_rates`): without the battery's row the leg costs
/// nothing (a rig is never stranded by its battery, §4.1); without the unit's speed, dark. A
/// cell with no way home at all → `RECOVER_FAR` as before (staff's remedy).
///
/// # Errors
///
/// The first [`Refusal`] in that order.
pub(crate) fn recover_plan(
    w: &World,
    actor: ObjId,
    rig: &RigAt,
    peeks: &mut Peeks,
    rates: Option<DriveRates>,
) -> Result<ActPlan, Refusal> {
    let (view, name) = view_of(w, rig)?;
    if matches!(berth_of(w, rig.hull), Berth::UnderWay) {
        return Err(Refusal::UnderWay { name });
    }
    let berth = site_berth(w, &view)?;
    if !crate::port::voyage::aboard(w, rig.hull, actor) {
        return Err(Refusal::NotAboard { name });
    }
    let ramp_cell = site_cell(berth.ramp.x_cm, berth.ramp.y_cm);
    let (x_cm, y_cm) = match where_is(w, rig.tv) {
        Where::Bay { hull } if hull == rig.hull => {
            return Ok(ActPlan {
                site: berth.entry,
                secs: RECOVER_S,
                cost_cu: 0,
                home_cu: 0,
                params: ActParams::None {},
                from_bay: true,
            });
        }
        Where::Ground {
            domain, x_cm, y_cm, ..
        } if domain == berth.domain => (x_cm, y_cm),
        Where::Bay { .. } | Where::Ground { .. } | Where::Elsewhere => {
            return Err(Refusal::Elsewhere { n: rig.slot, name });
        }
    };
    let cells = cells_apart(site_cell(x_cm, y_cm), ramp_cell);
    if cells <= 1 {
        return Ok(ActPlan {
            site: berth.entry,
            secs: RECOVER_S,
            cost_cu: 0,
            home_cu: 0,
            params: ActParams::None {},
            from_bay: false,
        });
    }
    let far = || Refusal::Far {
        n: rig.slot,
        m: cells.saturating_mul(10),
        ramp: berth.ramp_label.clone(),
    };
    let field = match crate::surface::peek_home(w, &berth.entry, ramp_cell, peeks) {
        HomePeek::Warm(field) => field,
        HomePeek::Cold => {
            return Err(Refusal::GroundCold {
                site: berth.name.clone(),
                entry: berth.entry.clone(),
            });
        }
        HomePeek::Dark => {
            return Err(Refusal::RecoverDark {
                n: rig.slot,
                ramp: berth.ramp_label.clone(),
            });
        }
    };
    let Some(rates) = rates else {
        return Err(Refusal::Dark { name });
    };
    // Lead ruling 5: a cell with no way home is SM.3b's `RECOVER_FAR` (staff's remedy).
    let leg = drive::recover_leg(&field, (x_cm, y_cm), &rates).ok_or_else(far)?;
    let to = leg
        .cells
        .last()
        .map(|(x, y)| [i64::from(*x), i64::from(*y)])
        .ok_or_else(far)?;
    Ok(ActPlan {
        site: berth.entry,
        secs: leg.secs.saturating_add(RECOVER_S),
        cost_cu: leg.cu,
        home_cu: 0,
        params: ActParams::Drive {
            from_bay: false,
            target: format!("the {}", berth.ramp_label),
            to,
            waypoints: leg.waypoints,
            secs: leg.secs,
            cost: leg.cost,
            metres: leg.metres,
            heading_mdeg: leg.heading_mdeg,
        },
        from_bay: false,
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

/// **The unload's world-side gate, in order** (doc 20 cut SM.8; §7.1, §8; the brief's 3.5,
/// decisions R5, R6, R9 and R14), cheap first: the layer and the view (dark or held →
/// `TV_DARK`); under way → `UNLOAD_UNDER_WAY`; not on a site's pad → `UNLOAD_NOT_SITE` (an
/// unload is a surface act, the dump's rule — R5); **aboard, for the hand unload only** (`queued`
/// false: `TV_NOT_ABOARD`; a queued unload is the recover's second half, committed aboard by its
/// `go` — R6); the rig in this hull's bay (on the site's ground → `UNLOAD_OUT`; anywhere else →
/// `RECOVER_ELSEWHERE`); the rig's hopper vault and the hull's hold vault (either missing →
/// `TV_DARK`, logged). `UNLOAD_S`, no cost, no trip home; `params` names the two vaults, the
/// goods empty (the command and the offer split the hopper; the prepare re-checks). **No
/// licence, no tenure, no lease, no battery is read** (§8 rights per act; §19 item 3; §4.1).
///
/// # Errors
///
/// The first [`Refusal`] in that order.
pub(crate) fn unload_plan(
    w: &World,
    actor: ObjId,
    rig: &RigAt,
    queued: bool,
) -> Result<ActPlan, Refusal> {
    let (view, name) = view_of(w, rig)?;
    if matches!(berth_of(w, rig.hull), Berth::UnderWay) {
        return Err(Refusal::UnloadUnderWay { name });
    }
    let berth = site_berth(w, &view).map_err(|refusal| match refusal {
        Refusal::NotSite { name } => Refusal::UnloadNotSite { n: rig.slot, name },
        Refusal::UnderWay { name } => Refusal::UnloadUnderWay { name },
        other => other,
    })?;
    if !queued && !crate::port::voyage::aboard(w, rig.hull, actor) {
        return Err(Refusal::NotAboard { name });
    }
    match where_is(w, rig.tv) {
        Where::Bay { hull } if hull == rig.hull => {}
        Where::Ground { domain, .. } if domain == berth.domain => {
            return Err(Refusal::UnloadOut {
                n: rig.slot,
                site: berth.name,
            });
        }
        Where::Bay { .. } | Where::Ground { .. } | Where::Elsewhere => {
            return Err(Refusal::Elsewhere { n: rig.slot, name });
        }
    }
    let Some(hopper) = crate::port::vehicle::hopper_of(w, rig.tv) else {
        tracing::warn!(tv = rig.tv.0, "tacsim: an unload found no hopper vault");
        return Err(Refusal::Dark { name });
    };
    let Some(hold) = crate::port::ship::hold_vault(w, &view) else {
        tracing::warn!(hull = rig.hull.0, "tacsim: an unload found no hold vault");
        return Err(Refusal::Dark { name });
    };
    Ok(ActPlan {
        site: berth.entry,
        secs: UNLOAD_S,
        cost_cu: 0,
        home_cu: 0,
        params: ActParams::Unload {
            hopper: hopper.0,
            hold: hold.0,
            goods: BTreeMap::new(),
            how: if queued {
                UnloadHow::Queued
            } else {
                UnloadHow::Whole
            },
            excursion: None,
        },
        from_bay: false,
    })
}

/// **The hold's room an unload weighs against** (doc 20 cut SM.8; lead ruling 3, decision R16).
#[derive(Clone, Debug)]
pub(crate) struct UnloadRoom {
    /// The hull's hold plan (`crate::port::market::hold_plan`).
    pub plan: crate::port::market::HoldPlan,
    /// The plan's goods table (`crate::port::market::plan_goods`).
    pub goods: &'static crate::definitions::GoodsTable,
    /// The hold's ore compartment.
    pub compartment: crate::port::capacity::Compartment,
    /// The hold's unreserved room, per measure, with the unloading rig's own hopper left out.
    pub room: Load,
    /// The hull's other rigs (their hoppers and runs are the reservation).
    pub siblings: Vec<ObjId>,
}

/// **The hold's room for `tv`'s unload** (doc 20 cut SM.8; the brief's 3.5, lead ruling 3,
/// decisions R4 and R16) — one function, the command, the start's offer and the recover's preview
/// all call it: the hull's [`crate::port::market::hold_plan`], its goods table
/// ([`crate::port::market::plan_goods`]; a store table loaded and unequal to it is a dark plan,
/// logged, never a preview the prepare then refuses), the hold's ONE ore compartment
/// (`capacity::compartment_for` over the plan's compartments for the ore stowage tag; a hull
/// whose ores route to none or several is dark), and
/// `crate::port::ship::hold_unreserved_with` over that table with **the unloading rig left out**
/// (its hopper is the source; a rig in the bay has no run) — the hull's rigs as the departure's
/// bay gate collects them (`rigs_in_bay` ∪ `live_on_hull`), less `tv`. With one ore compartment
/// the summed room `hold_unreserved_with` computes is that compartment's. `None` when any of it
/// will not read.
pub(crate) fn unload_room(
    w: &World,
    view: &HullView<'_>,
    tv: ObjId,
    read_pool: &dyn Fn(EndpointRef, &CanonicalId) -> Result<u64, crate::port_tick::PoolReadError>,
    read_rig: &dyn Fn(ObjId) -> Result<Option<RigRecord>, String>,
) -> Option<UnloadRoom> {
    let plan = crate::port::market::hold_plan(view)?;
    let goods = crate::port::market::plan_goods(&plan)?;
    if let Some(stored) = w.module_state::<TacsimStore>().and_then(TacsimStore::goods)
        && stored != goods
    {
        tracing::error!(
            hull = view.hull().0,
            "tacsim: the store's goods table differs from the hold plan's; the unload reads dark"
        );
        return None;
    }
    let ore = CanonicalId::new("metal_ore").ok()?;
    let compartment = crate::port::capacity::compartment_for(
        goods,
        &crate::port::market::plan_compartments(&plan),
        &ore,
    )?;
    let mut siblings: Vec<ObjId> = crate::port::vehicle::rigs_in_bay(w, view)
        .iter()
        .map(|rig| rig.tv)
        .collect();
    for record in tv::live_on_hull(w, view.hull()) {
        if !siblings.contains(&record.tv) {
            siblings.push(record.tv);
        }
    }
    siblings.retain(|other| *other != tv);
    siblings.sort_unstable();
    let room =
        crate::port::ship::hold_unreserved_with(goods, w, view, &siblings, read_pool, read_rig)?;
    Some(UnloadRoom {
        plan,
        goods,
        compartment,
        room,
        siblings,
    })
}

/// Where a scan, a sounding or (doc 20 cut SM.4b) a drive is made from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Stand {
    /// The site's port entry.
    pub site: CanonicalId,
    /// The stand cell, site-local.
    pub cell: (i64, i64),
    /// The launch is implied (the rig stands in the bay; it reads from the ramp's cell).
    pub from_bay: bool,
    /// **The exact start pose, centimetres** (doc 20 cut SM.4b): the ramp pose from the bay,
    /// else the rig's own `Direct` pose — a drive's leg starts here.
    pub start_cm: (i64, i64),
    /// The hull's ramp cell (doc 20 cut SM.4b: the home field's key).
    pub ramp_cell: (i64, i64),
    /// The egress's label (`stern ramp`).
    pub ramp_label: String,
    /// The site's name.
    pub site_name: String,
}

/// **Where a scan or a sounding stands, in the plan's order** (doc 20 cut SM.4a; the brief's
/// 3.4, D12, D13): the layer and the view (`view_of`); under way → `LAUNCH_UNDER_WAY`; then
/// the rig: in this hull's bay → **`launch_plan`, its whole order unchanged**, and the stand
/// is the ramp's cell (`from_bay`); otherwise the hull's site berth (not a site →
/// `LAUNCH_NOT_SITE`), the actor aboard (else `TV_NOT_ABOARD`), and the rig `Direct` on the
/// hull's site domain inside the site (else `RECOVER_ELSEWHERE`) — **no lease-state gate**, as
/// a recover's and a dump's (decision 11). **Doc 20 cut SM.4b**: a drive stands the same way
/// (the implied launch's whole gate from the bay), so the command, the start's offer and the
/// drive's order all read this one rule.
pub(crate) fn stand_of(w: &World, actor: ObjId, rig: &RigAt) -> Result<Stand, Refusal> {
    let (view, name) = view_of(w, rig)?;
    if matches!(berth_of(w, rig.hull), Berth::UnderWay) {
        return Err(Refusal::UnderWay { name });
    }
    if where_is(w, rig.tv) == (Where::Bay { hull: rig.hull }) {
        let launch = launch_plan(w, actor, rig)?;
        let berth = site_berth(w, &view)?;
        let ramp_cell = site_cell(berth.ramp.x_cm, berth.ramp.y_cm);
        return Ok(Stand {
            site: launch.site,
            cell: ramp_cell,
            from_bay: true,
            start_cm: (berth.ramp.x_cm, berth.ramp.y_cm),
            ramp_cell,
            ramp_label: berth.ramp_label.clone(),
            site_name: berth.name.clone(),
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
            site: berth.entry.clone(),
            cell: site_cell(x_cm, y_cm),
            from_bay: false,
            start_cm: (x_cm, y_cm),
            ramp_cell: site_cell(berth.ramp.x_cm, berth.ramp.y_cm),
            ramp_label: berth.ramp_label.clone(),
            site_name: berth.name.clone(),
        }),
        Where::Bay { .. } | Where::Ground { .. } | Where::Elsewhere => {
            Err(Refusal::Elsewhere { n: rig.slot, name })
        }
    }
}

/// **The charge home from a stand a reading act occupies** (doc 20 cut SM.4b; lead ruling 4,
/// §3.8 of the brief): 0 from the bay (the stand is the ramp's cell); else the home field's figure
/// through the non-building peek (cold → `GROUND_COLD`, dark → `INSTRUMENTS_DARK`) and
/// `drive::home_cu_here` — the conservative bound where the stand has no way home, since there is
/// nothing left to prevent there (R2).
fn home_here(
    w: &World,
    stand: &Stand,
    rates: Option<&DriveRates>,
    name: &str,
    peeks: &mut Peeks,
) -> Result<u32, Refusal> {
    // From the bay, or from beside the ramp (Chebyshev ≤ 1: the recover's own rule), the way
    // home is the climb alone — no field is read, so a rig at the ramp scans and sounds with
    // the home field still cold.
    if stand.from_bay || cells_apart(stand.cell, stand.ramp_cell) <= 1 {
        return Ok(0);
    }
    let field = match crate::surface::peek_home(w, &stand.site, stand.ramp_cell, peeks) {
        HomePeek::Warm(field) => field,
        HomePeek::Cold => {
            return Err(Refusal::GroundCold {
                site: stand.site_name.clone(),
                entry: stand.site.clone(),
            });
        }
        HomePeek::Dark => return Err(Refusal::InstrumentsDark),
    };
    let rates = rates.ok_or_else(|| Refusal::Dark {
        name: name.to_owned(),
    })?;
    Ok(drive::home_cu_here(
        &field,
        stand.cell,
        rates,
        mush_atlas::physics::LocomotionTable::shipped(),
    ))
}

/// **A rig's driving figures** (doc 20 cut SM.4b; lead ruling 3): the rig unit's tracked
/// `max_speed_cms` (its live unit record's, else the programme pool's unit) and the battery's
/// `drive_cu_per_nominal_cell` — **0 without the battery's row**, so a recover's leg home never
/// needs it (§4.1; a drive needs the battery anyway). `None` when the unit has no tracked mode or
/// a zero speed (`TV_DARK`).
#[must_use]
pub(crate) fn rig_rates(w: &World, tv: ObjId, battery: Option<&BatteryDef>) -> Option<DriveRates> {
    let store = w.module_state::<TacsimStore>()?;
    let unit = crate::port::hull::live_unit_id(w, tv).or_else(|| tv::pool_unit(w))?;
    let speed = store
        .definitions()
        .specs
        .get(&unit)?
        .mobility
        .tracked
        .as_ref()?
        .max_speed_cms;
    if speed == 0 {
        return None;
    }
    Some(DriveRates {
        max_speed_cms: speed,
        cu_per_nominal_cell: battery.map_or(0, |battery| battery.drive_cu_per_nominal_cell),
    })
}

/// **The drive's world plan** (doc 20 cut SM.4b; lead rulings 2 and 4, R1–R4; §3.5 of the brief),
/// called only after the command's cheap gates: the site's ground resolved (dark →
/// `INSTRUMENTS_DARK`); the composite through the non-building peek (cold → `GROUND_COLD`, dark →
/// `INSTRUMENTS_DARK`); the home field at the hull's ramp cell (likewise); the target's cell (off
/// the site → `DRIVE_OFF_SITE`); **ONE A*** from the stand's exact start pose
/// (`drive::drive_leg`: the rig's cell → `DRIVE_HERE`, no way → `NO_PATH`, past 32 waypoints even
/// at 80 m → `TOO_WINDING`); then the charge home from the end stand (`drive::home_cu_to`: no way
/// home at all → `NO_WAY_HOME`, R2). The plan: `LAUNCH_S` more from the bay, the leg's seconds,
/// its charge, the real trip home, the leg in the params.
///
/// # Errors
///
/// The first [`Refusal`] in that order.
pub(crate) fn drive_plan(
    w: &World,
    rig: &RigAt,
    stand: &Stand,
    target: &Target,
    rates: &DriveRates,
    peeks: &mut Peeks,
) -> Result<(ActPlan, Leg), Refusal> {
    chart::site_context(w, &stand.site).map_err(|_| Refusal::InstrumentsDark)?;
    let cold = || Refusal::GroundCold {
        site: stand.site_name.clone(),
        entry: stand.site.clone(),
    };
    let ground = match chart::peek_site(w, &stand.site, peeks) {
        SitePeek::Warm(ground) => ground,
        SitePeek::Cold => return Err(cold()),
        SitePeek::Dark => return Err(Refusal::InstrumentsDark),
    };
    let field = match crate::surface::peek_home(w, &stand.site, stand.ramp_cell, peeks) {
        HomePeek::Warm(field) => field,
        HomePeek::Cold => return Err(cold()),
        HomePeek::Dark => return Err(Refusal::InstrumentsDark),
    };
    let surface = chart::ground_surface(&ground).ok_or(Refusal::InstrumentsDark)?;
    let to = drive::target_cell(target, stand.cell).ok_or_else(|| Refusal::OffSite {
        site: stand.site_name.clone(),
    })?;
    let words = drive::target_words(target);
    let table = mush_atlas::physics::LocomotionTable::shipped();
    let leg =
        drive::drive_leg(surface, table, stand.start_cm, to, rates).map_err(|why| match why {
            LegRefused::Here => Refusal::Here { n: rig.slot },
            LegRefused::NoPath | LegRefused::Overflow => Refusal::NoPath {
                n: rig.slot,
                target: words.clone(),
            },
            LegRefused::Winding { needed } => Refusal::Winding {
                n: rig.slot,
                target: words.clone(),
                needed,
            },
        })?;
    let home_cu = drive::home_cu_to(&field, to, rates).ok_or_else(|| Refusal::NoWayHome {
        n: rig.slot,
        target: words.clone(),
        ramp: stand.ramp_label.clone(),
    })?;
    let launch = if stand.from_bay { LAUNCH_S } else { 0 };
    let plan = ActPlan {
        site: stand.site.clone(),
        secs: launch.saturating_add(leg.secs),
        cost_cu: leg.cu,
        home_cu,
        params: ActParams::Drive {
            from_bay: stand.from_bay,
            target: words,
            to: [to.0, to.1],
            waypoints: leg.waypoints.clone(),
            secs: leg.secs,
            cost: leg.cost,
            metres: leg.metres,
            heading_mdeg: leg.heading_mdeg,
        },
        from_bay: stand.from_bay,
    };
    Ok((plan, leg))
}

/// **The scan's world-side gate, in order** (doc 20 cut SM.4a; §5's R1, the brief's 3.4): the
/// stand (`stand_of`, the implied launch's whole gate first); the site's ground resolved
/// (`chart::site_context`, else `INSTRUMENTS_DARK`); the composite through the **non-building
/// peek**, shared by every rig of the call (`peeks`; cold → `GROUND_COLD`, dark →
/// `INSTRUMENTS_DARK`); the footing at the stand (none → `NO_FOOTING`); the fields the mast
/// sees (`chart::seen_fields`). `LAUNCH_S` more when it launches; the scanner's `scan_s` and
/// `scan_cu`; **since doc 20 cut SM.4b the real trip home** from the stand (`home_here`: 0 from
/// the bay, else the home field's figure — cold → `GROUND_COLD`, dark → `INSTRUMENTS_DARK` — and
/// without the rig's rates `TV_DARK`).
///
/// # Errors
///
/// The first [`Refusal`] in that order.
pub(crate) fn scan_plan(
    w: &World,
    actor: ObjId,
    rig: &RigAt,
    scanner: &ScannerDef,
    peeks: &mut Peeks,
    rates: Option<&DriveRates>,
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
    // Doc 20 cut SM.4b (lead ruling 4): the real trip home from the stand.
    let home_cu = home_here(w, &stand, rates, &name_of(w, rig.hull), peeks)?;
    let launch = if stand.from_bay { LAUNCH_S } else { 0 };
    Ok(ActPlan {
        site: stand.site,
        secs: launch.saturating_add(scanner.scan_s),
        cost_cu: scanner.scan_cu,
        home_cu,
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
/// same `sound_s` and `sound_cu`; `LAUNCH_S` more when it launches; **since doc 20 cut SM.4b
/// the real trip home** from the stand (`home_here`, as the scan's).
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
    peeks: &mut Peeks,
    rates: Option<&DriveRates>,
) -> Result<ActPlan, Refusal> {
    let stand = stand_of(w, actor, rig)?;
    let site = chart::site_context(w, &stand.site).map_err(|_| Refusal::InstrumentsDark)?;
    let home_cu = home_here(w, &stand, rates, &name_of(w, rig.hull), peeks)?;
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
        home_cu,
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
            ActParams::None {}
            | ActParams::Dump { .. }
            | ActParams::Drive { .. }
            | ActParams::Unload { .. } => None,
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

/// **What an act in flight is doing** (`RIG_BUSY`'s and `ACT_LINE`'s `{doing}`). Doc 20 cut
/// SM.4b: a drive is `driving to {target}`, a recover with a leg `driving to the {ramp}`.
#[must_use]
pub(crate) fn doing_words(kind: ActKind, params: &ActParams, ramp: &str) -> String {
    match (kind, params) {
        (ActKind::Drive, ActParams::Drive { target, .. }) => format!("driving to {target}"),
        (ActKind::Recover, ActParams::Drive { .. }) => format!("driving to the {ramp}"),
        (ActKind::Drive, _) => "driving".to_owned(),
        (ActKind::Launch, _) => format!("going down the {ramp}"),
        (ActKind::Recover, _) => format!("climbing the {ramp}"),
        (ActKind::Dump, _) => "dumping its hopper".to_owned(),
        (ActKind::Scan, _) => "scanning".to_owned(),
        (ActKind::Sound, _) => "sounding".to_owned(),
        // Doc 20 cut SM.7: a run never sits in the act slot; the word is the status's.
        (ActKind::Mine, _) => "drilling".to_owned(),
        // Doc 20 cut SM.8.
        (ActKind::Unload, _) => "unloading its hopper into the hold".to_owned(),
    }
}

/// **`{doing}` for a run in flight** (doc 20 cut SM.7; the SM.7 lead's pre-commit round, item 2):
/// `drilling C117` — [`doing_words`]' word for the cycle slot, which holds a run rather than an
/// act, with the pocket it works. The departure's `TV_BUSY` and the lease return's `RIG_BUSY`
/// read it, each followed by `crate::port::tv::RUN_STOP_REMEDY`.
#[must_use]
pub(crate) fn drilling_words(pocket: &str) -> String {
    format!("drilling {pocket}")
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

/// `{load}`: `"{q} t of {good label}"` per good, joined as a list ([`and_list`]: `a`, `a and b`,
/// `a, b and c`). The SM.8 lead's pre-commit round, item 5: goods v3's labels carry commas
/// ("ice ore, run-of-mine"), so a plain `", "` join read two ores as four words.
#[must_use]
pub(crate) fn load_words(w: &World, goods: &BTreeMap<CanonicalId, u64>) -> String {
    let table = w.module_state::<TacsimStore>().and_then(TacsimStore::goods);
    let words: Vec<String> = goods
        .iter()
        .map(|(good, qty)| {
            let label = table
                .and_then(|table| table.good(good))
                .map_or_else(|| good.to_string(), |def| def.label.clone());
            format!("{qty} t of {label}")
        })
        .collect();
    and_list(&words)
}

/// `a`, `a and b`, `a, b and c` — the goods a load line names (the cockpit's `join_names` shape);
/// empty for none.
fn and_list(words: &[String]) -> String {
    match words {
        [] => String::new(),
        [one] => one.clone(),
        [head @ .., last] => format!("{} and {last}", head.join(", ")),
    }
}

/// **The rig's battery row** (finding F5: the one tools table), whether or not the act needs it.
fn rig_battery(w: &World, handles: &PortHandles, rig: &RigAt) -> Option<BatteryDef> {
    handles
        .surface_tools
        .as_ref()
        .and_then(|tools| rig_battery_id(w, rig.tv).and_then(|id| battery_of(tools, &id).cloned()))
}

/// **A cold site or home asked for** (doc 20 cut SM.4a; since SM.4b the home field too): the
/// surface tick warms them, never this path (RAM only, the `RigPointer` precedent).
fn want_cold(ctx: &mut Ctx<'_>, rig: &RigAt, refusal: &Refusal) {
    if let Refusal::GroundCold { entry, .. } = refusal {
        crate::surface::want_site_ground(ctx.w, entry);
        if let Some((site, ramp)) = crate::surface::hull_home(ctx.w, rig.hull) {
            crate::surface::want_home(ctx.w, &site, ramp);
        }
    }
}

/// **A live plan holds its rig** (doc 20 cut SM.11; the brief's 3.5, D8): `PLAN_LIVE` when the
/// rig's record holds a plan live for the actor — every hand start's command asks it first, before
/// the record's busy refusal, and says nothing else. A record that will not read answers nothing
/// here (the command's own read darkens it).
fn plan_holds(ctx: &Ctx<'_>, rig: &RigAt) -> Result<(), CmdError> {
    let Some(handles) = ctx.w.module_state::<PortHandles>() else {
        return Ok(());
    };
    let Ok(Some(record)) = ctx.action_state(&handles.families.excursions, &RigKey(rig.tv)) else {
        return Ok(());
    };
    match record.live_plan_for(ctx.actor) {
        Some(plan) => Err(CmdError::msg(
            Refusal::PlanLive {
                n: rig.slot,
                no: plan.no,
            }
            .line(),
        )),
        None => Ok(()),
    }
}

/// **The record's busy refusal** (`RIG_BUSY`): an act in flight.
fn busy_refusal(w: &World, rig: &RigAt, record: &RigRecord) -> Option<Refusal> {
    let act = record.act.as_ref()?;
    let ramp = ramp_label(w, rig.hull);
    Some(Refusal::Busy {
        n: rig.slot,
        doing: doing_words(act.kind, &act.params, &ramp),
        when: tv::due_words(w, act.due_ic),
    })
}

/// **The record's run refusal** (doc 20 cut SM.7; decision R1): a run in the cycle slot holds the
/// rig to its pocket — `MINE_RUNNING`, naming the pocket, the cycle and its due.
pub(crate) fn run_refusal(w: &World, rig: &RigAt, record: &RigRecord) -> Option<Refusal> {
    let run = record.run.as_ref()?;
    Some(Refusal::MineRunning {
        n: rig.slot,
        addr: address::pocket_address(run.pocket[0], run.pocket[1]).unwrap_or_default(),
        k: run.k,
        cycles: run.cycles,
        when: tv::due_words(w, run.due_ic),
    })
}

/// **The launch half's gates** (a launch, or a scan, a sounding or a drive that implies one;
/// lead ruling 2): an excursion still open (`LAUNCH_OPEN`), then the caps (§8; lead ruling 8),
/// counted over the world's rig records.
fn launch_half(
    ctx: &Ctx<'_>,
    handles: &PortHandles,
    rig: &RigAt,
    record: &RigRecord,
    site: &CanonicalId,
    dark: &dyn Fn() -> CmdError,
) -> Result<(), CmdError> {
    if record.excursion.is_some() {
        return Err(CmdError::msg(Refusal::Open { n: rig.slot }.line()));
    }
    let w: &World = ctx.w;
    let mut counts = ExcursionCounts::default();
    for tv in world_rigs(w) {
        let found = ctx
            .action_state(&handles.families.excursions, &RigKey(tv))
            .map_err(|_| dark())?;
        if let Some(excursion) = found.and_then(|record| record.excursion.as_ref()) {
            counts.world = counts.world.saturating_add(1);
            if &excursion.site == site {
                counts.site = counts.site.saturating_add(1);
            }
            if excursion.operator == ctx.actor {
                counts.operator = counts.operator.saturating_add(1);
            }
        }
    }
    if let Some(cap) = cap_refusal(&counts) {
        return Err(CmdError::msg(
            Refusal::Cap {
                cap,
                site: site_name(w, site),
            }
            .line(),
        ));
    }
    Ok(())
}

/// **The reserve rule's words** (§4.1; lead ruling 3, enforced again in the start's prepare):
/// `(now %, after the act %, after the trip home %)` from the charge the act spends from (an
/// implied launch: the bay credited no further than the launch half, D22), or the `RESERVE_LOW`
/// line.
fn reserve_words(
    record: &RigRecord,
    now_ic: i64,
    launches: bool,
    (cost_cu, home_cu): (u32, u32),
    battery: &BatteryDef,
    n: u8,
) -> Result<(u32, u32, u32), CmdError> {
    let shown = charge_at(&record.charge, now_ic, battery);
    let from = charge_for_act(&record.charge, now_ic, launches, battery);
    match reserve_check(from, reserved_total(record), cost_cu, home_cu, battery) {
        Ok(after) => Ok((
            percent(shown, battery),
            percent(after.after_act, battery),
            percent(after.after_home, battery),
        )),
        Err(short) => {
            let after = u32::try_from(short.after_home.max(0)).unwrap_or(0);
            Err(CmdError::msg(
                Refusal::Reserve {
                    n,
                    now: percent(shown, battery),
                    after: percent(after, battery),
                    floor: percent(short.floor, battery),
                }
                .line(),
            ))
        }
    }
}

/// Whole percent of a battery, rounded up (a leg's need: never under what it costs).
fn percent_up(cu: u32, battery: &BatteryDef) -> u32 {
    if battery.capacity_cu == 0 {
        return 0;
    }
    u32::try_from((u64::from(cu).saturating_mul(100)).div_ceil(u64::from(battery.capacity_cu)))
        .unwrap_or(u32::MAX)
}

/// **A recover's leg home, affordable or `RECOVER_SHORT`** (doc 20 cut SM.4b; lead ruling 5,
/// decision P6): `leg_affordable` on the record's own value — never the 5 % floor, never the
/// battery's row; the battery reads only the words (`?` without it). **SM.11: its idle
/// return-and-park must treat this refusal as "parked where it stands".**
pub(crate) fn leg_home(
    record: &RigRecord,
    plan: &ActPlan,
    battery: Option<&BatteryDef>,
    rig: &RigAt,
    ramp: &str,
) -> Result<(), Refusal> {
    if leg_affordable(&record.charge, reserved_total(record), plan.cost_cu) {
        return Ok(());
    }
    let words = |cu: u32, up: bool| {
        battery.map_or_else(
            || "?".to_owned(),
            |battery| {
                if up {
                    percent_up(cu, battery).to_string()
                } else {
                    percent(cu, battery).to_string()
                }
            },
        )
    };
    Err(Refusal::LegShort {
        n: rig.slot,
        now: words(record.charge.value_cu, false),
        need: words(plan.cost_cu, true),
        ramp: ramp.to_owned(),
    })
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
/// **Doc 20 cut SM.4b**: a scan's and a sounding's reserve carries the real trip home from the
/// stand (`home_here`); a recover from farther than beside the ramp drives home first — one act
/// whose leg is checked by `leg_affordable` (`RECOVER_SHORT`), never by the 5 % floor, and
/// previewed `RECOVER_DRIVE_PREVIEW`. A drive is `drive_cmd`'s.
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
    // Doc 20 cut SM.11 (D8): a live plan holds its rig.
    plan_holds(ctx, rig)?;
    let name = name_of(ctx.w, rig.hull);
    let dark = || CmdError::msg(tv::TV_DARK.replace("{name}", &name));
    // 1. The world plan (the start's offer runs the same function). A reading act needs the
    // rig's scanner; the site's composite and its home field are peeked once for the whole call.
    let scanner = handles.surface_tools.as_ref().and_then(|tools| {
        rig_scanner_id(ctx.w, rig.tv).and_then(|id| scanner_of(tools, &id).copied())
    });
    let battery = rig_battery(ctx.w, &handles, rig);
    let rates = rig_rates(ctx.w, rig.tv, battery.as_ref());
    let mut peeks = Peeks::default();
    let planned = match kind {
        ActKind::Launch => launch_plan(ctx.w, actor, rig),
        ActKind::Recover => recover_plan(ctx.w, actor, rig, &mut peeks, rates),
        ActKind::Dump => dump_plan(ctx.w, actor, rig),
        ActKind::Scan | ActKind::Sound => {
            let Some(scanner) = scanner.as_ref() else {
                tracing::warn!(tv = rig.tv.0, "tacsim: a rig carries no readable scanner");
                return Err(dark());
            };
            if kind == ActKind::Scan {
                scan_plan(ctx.w, actor, rig, scanner, &mut peeks, rates.as_ref())
            } else {
                sound_plan(
                    ctx.w,
                    actor,
                    rig,
                    scanner,
                    quiet,
                    &mut peeks,
                    rates.as_ref(),
                )
            }
        }
        // A drive names its target: `drive_cmd`; a run is `mine_cmd`'s.
        ActKind::Drive | ActKind::Mine | ActKind::Unload => return Err(dark()),
    };
    let plan = match planned {
        Ok(plan) => plan,
        Err(refusal) => {
            // A cold site or home is asked for: the surface tick warms it, never this path.
            want_cold(ctx, rig, &refusal);
            return Err(CmdError::msg(refusal.line()));
        }
    };
    let w: &World = ctx.w;
    // 2. The battery, read only where the act needs it — a spend or a launch
    // (`needs_battery`); a recover or a dump never does, so a rig whose battery row stops
    // resolving still comes home (§4.1; the fixer round's item 3; since doc 20 cut SM.4b a
    // recover with a leg too).
    if battery.is_none() && needs_battery(kind, plan.cost_cu, plan.home_cu) {
        return Err(dark());
    }
    // 3. The typed record, and every refusal of the prepare it can read, in its order.
    let record = record_of(ctx, &handles, rig.tv, battery.as_ref()).map_err(|()| dark())?;
    let ramp = ramp_label(w, rig.hull);
    if let Some(busy) = busy_refusal(w, rig, &record) {
        return Err(CmdError::msg(busy.line()));
    }
    // Doc 20 cut SM.7 (decision R1): while a run drills, only a scan or a sounding may start.
    if !matches!(kind, ActKind::Scan | ActKind::Sound)
        && let Some(running) = run_refusal(w, rig, &record)
    {
        return Err(CmdError::msg(running.line()));
    }
    // The launch's half: a launch, or a scan or a sounding that implies one (lead ruling 2).
    let launches = kind == ActKind::Launch || plan.params.launches();
    if kind == ActKind::Recover && plan.from_bay && record.excursion.is_none() {
        return Err(CmdError::msg(Refusal::InBay { n: rig.slot }.line()));
    }
    if launches {
        launch_half(ctx, &handles, rig, &record, &plan.site, &dark)?;
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
            ActParams::None {}
            | ActParams::Dump { .. }
            | ActParams::Drive { .. }
            | ActParams::Unload { .. } => {
                return Err(dark());
            }
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
    // implied launch spends from the charge the bay holds when the launch half ends (D22). Doc
    // 20 cut SM.4b: a recover's leg home is checked by `leg_affordable`, never the floor.
    let homeward = kind == ActKind::Recover && plan.params.leg().is_some();
    let charge = if homeward {
        leg_home(&record, &plan, battery.as_ref(), rig, &ramp)
            .map_err(|refusal| CmdError::msg(refusal.line()))?;
        battery.as_ref().map(|battery| {
            let shown = charge_at(&record.charge, now_ic, battery);
            (
                percent(shown, battery),
                percent(shown.saturating_sub(plan.cost_cu), battery),
            )
        })
    } else {
        match &battery {
            Some(battery) => {
                let (now, _, after) = reserve_words(
                    &record,
                    now_ic,
                    plan.params.launches(),
                    (plan.cost_cu, plan.home_cu),
                    battery,
                    rig.slot,
                )?;
                Some((now, after))
            }
            None => None,
        }
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
                let (preview, unread) = match &plan.params {
                    ActParams::Drive { .. } => {
                        (tv::RECOVER_DRIVE_PREVIEW, tv::RECOVER_DRIVE_PREVIEW_UNREAD)
                    }
                    _ => (tv::RECOVER_PREVIEW, tv::RECOVER_PREVIEW_UNREAD),
                };
                let (metres, drive_s) = match &plan.params {
                    ActParams::Drive { metres, secs, .. } => (*metres, *secs),
                    _ => (0, 0),
                };
                match charge {
                    Some((now, after)) => preview
                        .replace("{now}", &now.to_string())
                        .replace("{after}", &after.to_string()),
                    None => unread.to_owned(),
                }
                .replace("{n}", &n)
                .replace("{mark}", &rig.mark)
                .replace("{ramp}", &ramp)
                .replace("{name}", &name)
                .replace("{secs}", &plan.secs.to_string())
                .replace("{m}", &metres.to_string())
                .replace("{drive}", &drive_s.to_string())
                .replace("{climb}", &RECOVER_S.to_string())
                .replace("{real}", &real)
                .replace("{hopper}", &hopper)
                // Doc 20 cut SM.8 (lead ruling 1): the unload the recover's `Done` will queue.
                .replace("{then}", &recover_then(ctx, rig))
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
            ActKind::Drive | ActKind::Mine | ActKind::Unload => return Err(dark()),
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
        ActKind::Recover if homeward => tv::RECOVER_DRIVE_FILED
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
        ActKind::Drive | ActKind::Mine | ActKind::Unload => return Err(dark()),
    };
    crate::port::coop::say(ctx, line);
    Ok(())
}

// ---- doc 20 cut SM.8: `+tv/unload` and the recover's unload clause -----------------------------

/// **An unload's split against its room** (doc 20 cut SM.8; the brief's 3.4): the whole unload
/// (`qty` `None`) or a part one, through [`crate::surface::unload`]; `Err(())` when the capacity
/// function refuses the question (a good the plan's table does not weigh — never expected).
fn unload_split(
    room: &UnloadRoom,
    hopper: &BTreeMap<CanonicalId, u64>,
    qty: Option<u64>,
) -> Result<Result<crate::surface::unload::Split, Short>, ()> {
    match qty {
        None => crate::surface::unload::whole(room.goods, &room.compartment, hopper, &room.room)
            .map(Ok)
            .map_err(|_| ()),
        Some(qty) => {
            crate::surface::unload::part(room.goods, &room.compartment, hopper, &room.room, qty)
                .map_err(|_| ())
        }
    }
}

/// **The hold's unreserved mass after `moves`**, whole tonnes floored (the previews' `{room}`).
fn room_after_t(room: &UnloadRoom, moves: &BTreeMap<CanonicalId, u64>) -> Option<u64> {
    let mut after = room.room.clone();
    for (good, qty) in moves {
        crate::port::capacity::take_room(room.goods, &room.compartment, &mut after, good, *qty)
            .ok()?;
    }
    let mass = CanonicalId::new(crate::definitions::logistics::MASS_KG).ok()?;
    Some(after.get(&mass).copied().unwrap_or(0) / 1_000)
}

/// **A recover's `{then}`** (doc 20 §1 beat 9; cut SM.8 lead rulings 1 and 3): empty for an empty
/// hopper; else the unload the recover's `Done` will queue, split against the hold's room now —
/// `RECOVER_THEN` (all of it, and the room after), `RECOVER_THEN_PART` (what fits and what
/// stays) or `RECOVER_THEN_NONE` (none of it: the dump named). A hopper or a room that will not
/// read leaves it empty, logged: a recover is never refused for it (the way home is never barred).
fn recover_then(ctx: &Ctx<'_>, rig: &RigAt) -> String {
    let w: &World = ctx.w;
    let hopper = match hopper_contents(ctx, rig.tv) {
        Ok(hopper) => hopper,
        Err(error) => {
            tracing::warn!(tv = rig.tv.0, %error, "tacsim: a recover's preview could not read the hopper");
            return String::new();
        }
    };
    if hopper.is_empty() {
        return String::new();
    }
    let Some(view) = hull_view(w, rig.hull) else {
        return String::new();
    };
    let read_pool = |endpoint: EndpointRef, good: &CanonicalId| {
        crate::port_tick::read_pool(ctx, endpoint, good)
    };
    let read_rig = |tv: ObjId| SurfaceReads::rig(ctx, tv);
    let Some(room) = unload_room(w, &view, rig.tv, &read_pool, &read_rig) else {
        tracing::warn!(
            tv = rig.tv.0,
            "tacsim: a recover's preview could not read the hold's room"
        );
        return String::new();
    };
    let Ok(Ok(split)) = unload_split(&room, &hopper, None) else {
        return String::new();
    };
    if split.moves.is_empty() {
        tv::RECOVER_THEN_NONE
            .replace("{stays}", &load_words(w, &split.stays))
            .replace("{n}", &rig.slot.to_string())
    } else if split.stays.is_empty() {
        tv::RECOVER_THEN
            .replace("{load}", &load_words(w, &split.moves))
            .replace(
                "{room}",
                &room_after_t(&room, &split.moves).unwrap_or(0).to_string(),
            )
    } else {
        tv::RECOVER_THEN_PART
            .replace("{load}", &load_words(w, &split.moves))
            .replace("{stays}", &load_words(w, &split.stays))
    }
}

/// **`+tv/<n> unload [<t>] [go]`** (doc 20 §1 beat 9, §7.1, §10; cut SM.8, the brief's 3.5; lead
/// rulings 2 and 3, decisions R4, R10, R18): [`unload_plan`] (`queued` false: the hand unload
/// reads aboard); the typed record (unreadable → `TV_DARK`); an act in flight → `RIG_BUSY`; a
/// run → `MINE_RUNNING`; the hopper's available goods (unreadable → `TV_DARK`; empty →
/// `DUMP_EMPTY`); the hold's room ([`unload_room`]; `None` → `TV_DARK`, logged); then the split:
/// bare, every good up to the room (none fits → `UNLOAD_NO_ROOM`); `<t>`, that much of the first
/// good, past the fit `UNLOAD_NO_FIT` naming it — a fit of 0 is `UNLOAD_NO_ROOM`, so the suggested
/// command always parses. Bare, the preview (`UNLOAD_PREVIEW`), nothing written; with `go`, the
/// order written ([`ActOrders`]: `ActOrder::Unload`), then ONE `tacsim_surface_act` with `act:
/// unload` and `how` (`whole`, or `part` with `qty`) keyed `tacsim.surface/{tv}/{ordinal}/act`,
/// and `UNLOAD_FILED`. No licence, tenure, lease or battery is read.
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line.
#[allow(clippy::too_many_lines)] // one ordered refusal list, then the preview or the go
pub(crate) fn unload_cmd(
    ctx: &mut Ctx<'_>,
    rig: &RigAt,
    qty: Option<u64>,
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
    // Doc 20 cut SM.11 (D8): a live plan holds its rig.
    plan_holds(ctx, rig)?;
    let name = name_of(ctx.w, rig.hull);
    let dark = || CmdError::msg(tv::TV_DARK.replace("{name}", &name));
    // 1. The world plan (the start's offer runs the same function).
    let plan =
        unload_plan(ctx.w, actor, rig, false).map_err(|refusal| CmdError::msg(refusal.line()))?;
    // 2. The record: busy, then a run.
    let battery = rig_battery(ctx.w, &handles, rig);
    let record = record_of(ctx, &handles, rig.tv, battery.as_ref()).map_err(|()| dark())?;
    if let Some(busy) = busy_refusal(ctx.w, rig, &record) {
        return Err(CmdError::msg(busy.line()));
    }
    if let Some(running) = run_refusal(ctx.w, rig, &record) {
        return Err(CmdError::msg(running.line()));
    }
    // 3. The hopper, then the hold's room.
    let hopper = hopper_contents(ctx, rig.tv).map_err(|error| {
        tracing::warn!(tv = rig.tv.0, %error, "tacsim: an unload could not read the hopper");
        dark()
    })?;
    if hopper.is_empty() {
        return Err(CmdError::msg(Refusal::Empty { n: rig.slot }.line()));
    }
    let w: &World = ctx.w;
    let view = hull_view(w, rig.hull).ok_or_else(dark)?;
    let read_pool = |endpoint: EndpointRef, good: &CanonicalId| {
        crate::port_tick::read_pool(ctx, endpoint, good)
    };
    let read_rig = |tv: ObjId| SurfaceReads::rig(&*ctx, tv);
    let Some(room) = unload_room(w, &view, rig.tv, &read_pool, &read_rig) else {
        tracing::warn!(
            tv = rig.tv.0,
            "tacsim: an unload could not read the hold's room"
        );
        return Err(dark());
    };
    // 4. The split.
    let no_room = || {
        CmdError::msg(
            Refusal::UnloadNoRoom {
                n: rig.slot,
                name: name.clone(),
                load: load_words(w, &hopper),
            }
            .line(),
        )
    };
    let split = match unload_split(&room, &hopper, qty).map_err(|()| dark())? {
        Ok(split) if split.moves.is_empty() => return Err(no_room()),
        Ok(split) => split,
        Err(Short { fit: 0, .. }) => return Err(no_room()),
        Err(Short { fit, bound }) => {
            let first = hopper.keys().next().cloned().ok_or_else(dark)?;
            // SM.7's `{ore}` (`ore_words`: `metal ore`), the mining lines' own word.
            let ore = tv::ore_words(first.as_str());
            // The lead's pre-commit round, item 4: the line names the term that bound the fit
            // — the hopper's own stock, or the hold's room.
            let refusal = match bound {
                ShortBound::Stock => Refusal::UnloadNoStock {
                    n: rig.slot,
                    fit,
                    ore,
                },
                ShortBound::Room => Refusal::UnloadNoFit {
                    n: rig.slot,
                    q: qty.unwrap_or(0),
                    fit,
                    ore,
                },
            };
            return Err(CmdError::msg(refusal.line()));
        }
    };
    let n = rig.slot.to_string();
    let now_ic = w.world_ic_now().at;
    if !go {
        let (kg, cap_kg) = tv::hopper_load_of(ctx, rig.tv).map_err(|error| {
            tracing::warn!(tv = rig.tv.0, %error, "tacsim: an unload could not weigh the hopper");
            dark()
        })?;
        let mut moved_kg: u64 = 0;
        for (good, qty) in &split.moves {
            let per = room
                .goods
                .good(good)
                .and_then(crate::definitions::GoodDef::mass_kg_per_unit)
                .ok_or_else(dark)?;
            moved_kg = moved_kg.saturating_add(qty.saturating_mul(per));
        }
        let real =
            match crate::port::voyage::real_until(w, now_ic.saturating_add(i64::from(plan.secs))) {
                Some(real) => format!(", about {}", tv::span_words(real)),
                None => String::new(),
            };
        let stays = if qty.is_none() && !split.stays.is_empty() {
            tv::UNLOAD_STAYS.replace("{stays}", &load_words(w, &split.stays))
        } else {
            String::new()
        };
        let line = tv::UNLOAD_PREVIEW
            .replace("{n}", &n)
            .replace("{mark}", &rig.mark)
            .replace("{load}", &load_words(w, &split.moves))
            .replace("{name}", &name)
            .replace("{secs}", &plan.secs.to_string())
            .replace("{real}", &real)
            .replace("{hop0}", &(kg / 1_000).to_string())
            .replace("{hop1}", &(kg.saturating_sub(moved_kg) / 1_000).to_string())
            .replace("{cap}", &(cap_kg / 1_000).to_string())
            .replace(
                "{room}",
                &room_after_t(&room, &split.moves)
                    .ok_or_else(dark)?
                    .to_string(),
            )
            .replace("{stays}", &stays)
            .replace("{arg}", &qty.map_or_else(String::new, |q| format!(" {q}")));
        crate::port::coop::say(ctx, line);
        return Ok(());
    }
    // `go`: the order, then ONE submission as the actor (the drive's `to` precedent: `how` and
    // `qty` bind the start to the order it was planned from — decision R15).
    let Some(ordinal) = record.ordinal.checked_add(1) else {
        tracing::error!(tv = rig.tv.0, "tacsim: a rig's act ordinal is exhausted");
        return Err(dark());
    };
    let Some(key) = act_key(rig.tv, ordinal) else {
        tracing::error!(tv = rig.tv.0, "tacsim: a rig act's key did not form");
        return Err(dark());
    };
    let how = if qty.is_some() {
        UnloadHow::Part
    } else {
        UnloadHow::Whole
    };
    let when = tv::due_words(w, now_ic.saturating_add(i64::from(plan.secs)));
    put_order(
        ctx,
        rig.tv,
        ActOrder::Unload(UnloadOrder {
            actor,
            ordinal,
            goods: split.moves,
            how,
            qty,
        }),
    );
    let parameters = match qty {
        Some(qty) => json!({
            "tv": rig.tv.0.to_string(),
            "act": wire_word(ActKind::Unload, false),
            "ordinal": ordinal,
            "how": how.word(),
            "qty": qty,
        }),
        None => json!({
            "tv": rig.tv.0.to_string(),
            "act": wire_word(ActKind::Unload, false),
            "ordinal": ordinal,
            "how": how.word(),
        }),
    };
    ctx.submit_authoritative(action, PerceivedRef::SelfActor, Vec::new(), parameters, key);
    crate::port::coop::say(
        ctx,
        tv::UNLOAD_FILED.replace("{n}", &n).replace("{when}", &when),
    );
    Ok(())
}

// ---- doc 20 cut SM.4b: the drive's order and `+tv/drive` --------------------------------------

/// **One planned drive, waiting for its start** (doc 20 cut SM.4b; the brief's decision P2): the
/// command plans the drive, writes this order for the rig and submits; the start's offer fixes
/// it (after re-running the same stand gate) and the prepare binds it to the invocation's
/// ordinal. A player-typed target cannot reach the offer (the legal-action query carries no
/// parameters) and a player-supplied leg cannot be trusted (the prepare has no composite), so the
/// plan the prepare trusts is server-fixed this way. RAM only: a restart loses an unlanded order,
/// and its start then refuses "not planned; preview it again".
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DriveOrder {
    /// Who planned it.
    pub actor: ObjId,
    /// The ordinal its start carries.
    pub ordinal: u32,
    /// The site entry.
    pub site: CanonicalId,
    /// The act's IC seconds (`LAUNCH_S` more from the bay).
    pub secs: u32,
    /// The leg's charge.
    pub cost_cu: u32,
    /// The charge home from the end stand.
    pub home_cu: u32,
    /// The frozen leg ([`ActParams::Drive`]).
    pub params: ActParams,
    /// The launch is implied.
    pub from_bay: bool,
}

/// **One planned mining run, waiting for its start** (doc 20 cut SM.7; the brief's decision R10,
/// SM.4b's P2 precedent): the command plans the run and writes this order; the start's offer
/// re-plans with the same choice and requires the plan equal (`mine_changed` otherwise), so `go`
/// commits exactly the preview or nothing. RAM only: a restart loses an unlanded order ("not
/// planned; preview it again").
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MineOrder {
    /// Who planned it.
    pub actor: ObjId,
    /// The ordinal its start carries.
    pub ordinal: u32,
    /// The site entry.
    pub site: CanonicalId,
    /// What was asked.
    pub choice: crate::surface::mine::MineChoice,
    /// The plan previewed.
    pub plan: crate::surface::mine::MinePlan,
}

/// **One planned hand unload, waiting for its start** (doc 20 cut SM.8; the brief's decisions R4
/// and R15, SM.7's `MineOrder` precedent): the command splits the hopper against the hold's room
/// and writes this order; the start's offer fixes it — only for the ordinal it was planned for
/// (RAM orders are never removed on landing) — and the prepare holds the start's `how` and `qty`
/// to it, then re-checks the hopper and the room in the transaction. RAM only: a restart loses an
/// unlanded order ("that act was not planned; preview it again").
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UnloadOrder {
    /// Who planned it.
    pub actor: ObjId,
    /// The ordinal its start carries.
    pub ordinal: u32,
    /// What moves (the preview's split).
    pub goods: BTreeMap<CanonicalId, u64>,
    /// `Whole` or `Part`.
    pub how: UnloadHow,
    /// The tonnes named (`Part` only).
    pub qty: Option<u64>,
}

/// **One planned filing, waiting for its start** (doc 20 cut SM.11; the brief's 3.5 and D10, the
/// `MineOrder` precedent): the command compiles the list and writes this order; the start's offer
/// — only for the record's ordinal and next plan number (the reviewer's F11) — re-compiles over the
/// engine and requires it equal (`plan_changed` otherwise), so `go` files exactly the preview or
/// nothing. RAM only: a restart loses an unlanded filing ("that act was not planned"). The plan's
/// own steps never read a RAM order (lead ruling 1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PlanOrder {
    /// Who filed it.
    pub actor: ObjId,
    /// The record's ordinal when it was planned (a filing writes no act, so no ordinal of its own).
    pub ordinal: u32,
    /// The plan's number: the record's `plans + 1`.
    pub no: u32,
    /// The list as typed.
    pub list: String,
    /// `family=` as typed.
    pub family: Option<OreFamily>,
    /// The compiled plan previewed.
    pub compiled: crate::surface::plan::Compiled,
}

/// **One rig's order** (doc 20 cut SM.7): a drive's or a mining run's — and since doc 20 cut SM.8
/// a hand unload's, since SM.11 a plan's filing; the newer of a rig's orders replaces the older
/// (one slot).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ActOrder {
    /// `+tv/drive … go`.
    Drive(DriveOrder),
    /// `+tv/mine … go` (boxed: the plan is large beside a drive's order).
    Mine(Box<MineOrder>),
    /// `+tv/unload … go` (doc 20 cut SM.8).
    Unload(UnloadOrder),
    /// `+tv/plan … go` (doc 20 cut SM.11; boxed beside the others).
    Plan(Box<PlanOrder>),
}

/// **The act orders** (doc 20 cut SM.4b's `DriveOrders`, generalised by cut SM.7 to every act
/// whose plan the legal-action query cannot carry — the brief's decision R10; the `RigPointer`
/// precedent): tacsim's own RAM module slot, registered by `crate::surface::tick::register`, keyed
/// by the rig, bounded at `crate::port::tv::MAX_POINTERS` (the smallest key dropped past it). A
/// convenience for the offer, never a fact: it decides nothing the prepare does not re-check.
#[derive(Clone, Debug, Default)]
pub(crate) struct ActOrders(BTreeMap<ObjId, ActOrder>);

/// Write `order` for `tv`.
fn put_order(ctx: &mut Ctx<'_>, tv: ObjId, order: ActOrder) {
    if let Some(orders) = ctx.w.module_state_mut::<ActOrders>() {
        orders.0.insert(tv, order);
        while orders.0.len() > tv::MAX_POINTERS {
            if orders.0.pop_first().is_none() {
                break;
            }
        }
    }
}

/// **A drive's end cell as one word** (the review's correctness/F5): `y × 256 + x` for the
/// `to` of an [`ActParams::Drive`], `0..=65,535` — the start's `to` parameter, which the prepare
/// holds to the order the offer fixed. `None` for any other params or a cell off the site.
fn order_to_word(params: &ActParams) -> Option<u32> {
    let ActParams::Drive { to, .. } = params else {
        return None;
    };
    let side = i64::from(crate::surface::ground::SITE_CELLS);
    let ([x, y], range) = (*to, 0..side);
    if !range.contains(&x) || !range.contains(&y) {
        return None;
    }
    u32::try_from(y.checked_mul(side)?.checked_add(x)?).ok()
}

/// The drive order planned for `tv`, if its slot holds one.
fn order_of(w: &World, tv: ObjId) -> Option<DriveOrder> {
    match w
        .module_state::<ActOrders>()
        .and_then(|orders| orders.0.get(&tv))
    {
        Some(ActOrder::Drive(order)) => Some(order.clone()),
        Some(ActOrder::Mine(_) | ActOrder::Unload(_) | ActOrder::Plan(_)) | None => None,
    }
}

/// The hand unload order planned for `tv`, if its slot holds one (doc 20 cut SM.8).
fn unload_order_of(w: &World, tv: ObjId) -> Option<UnloadOrder> {
    match w
        .module_state::<ActOrders>()
        .and_then(|orders| orders.0.get(&tv))
    {
        Some(ActOrder::Unload(order)) => Some(order.clone()),
        Some(ActOrder::Drive(_) | ActOrder::Mine(_) | ActOrder::Plan(_)) | None => None,
    }
}

/// **Landed mining orders leave the slot** (doc 20 cut SM.7; the SM.7 lead's pre-commit round,
/// item 5): every mining order whose ordinal is not its rig's next (`record.ordinal + 1`; 1 for a
/// rig with no record in `records`) — its start landed, or another act overtook it — is dropped,
/// so no later offer re-plans it. The surface tick calls it with a **complete** scan of the
/// family (a capped scan prunes nothing). A drive's order is kept as SM.4b left it: its stale
/// start answers "that rig's orders moved on" at the prepare (`tests/driving.rs` D9). **Doc 20
/// cut SM.8**: a hand unload's order is pruned the same way. The number of orders dropped.
pub(crate) fn prune_mine_orders(w: &mut World, records: &[RigRecord]) -> usize {
    let next: BTreeMap<ObjId, u32> = records
        .iter()
        .map(|record| (record.tv, record.ordinal.saturating_add(1)))
        .collect();
    // Doc 20 cut SM.11 (the reviewer's F11): a filing leaves once it landed (`plans ≥ no`) or the
    // rig's orders moved on (`ordinal` changed); a rig with no record keeps a first filing at 0.
    let filed: BTreeMap<ObjId, (u32, u32)> = records
        .iter()
        .map(|record| (record.tv, (record.ordinal, record.plans)))
        .collect();
    let Some(orders) = w.module_state_mut::<ActOrders>() else {
        return 0;
    };
    let before = orders.0.len();
    orders.0.retain(|tv, order| match order {
        ActOrder::Drive(_) => true,
        ActOrder::Mine(mine) => next.get(tv).copied().unwrap_or(1) == mine.ordinal,
        ActOrder::Unload(unload) => next.get(tv).copied().unwrap_or(1) == unload.ordinal,
        ActOrder::Plan(plan) => {
            let (ordinal, plans) = filed.get(tv).copied().unwrap_or((0, 0));
            plans < plan.no && ordinal == plan.ordinal
        }
    });
    before.saturating_sub(orders.0.len())
}

/// **Test hook** (doc 20 cut SM.11; the brief's P5): the plan number of the filing `tv`'s RAM
/// slot holds, if any — so a test can prove a landed filing leaves the slot. Reads only.
#[doc(hidden)]
#[must_use]
pub fn plan_order_for_tests(w: &World, tv: ObjId) -> Option<u32> {
    plan_order_of(w, tv).map(|order| order.no)
}

/// **Test hook** (doc 20 cut SM.11; the brief's P5 and P8): how many times the start's offer
/// compiled a filing, the interpreter ran and it planned a drive leg, this boot — so a test can
/// prove a stale filing compiles nothing and a capped plan plans no leg. Reads only.
#[doc(hidden)]
#[must_use]
pub fn plan_counts_for_tests(w: &World) -> (u64, u64, u64) {
    use std::sync::atomic::Ordering;
    w.module_state::<crate::surface::plan::PlanCounters>()
        .map_or((0, 0, 0), |counters| {
            (
                counters.0.compiles.load(Ordering::Relaxed),
                counters.0.decides.load(Ordering::Relaxed),
                counters.0.legs.load(Ordering::Relaxed),
            )
        })
}

/// The filing planned for `tv`, if its slot holds one (doc 20 cut SM.11).
fn plan_order_of(w: &World, tv: ObjId) -> Option<PlanOrder> {
    match w
        .module_state::<ActOrders>()
        .and_then(|orders| orders.0.get(&tv))
    {
        Some(ActOrder::Plan(order)) => Some(PlanOrder::clone(order)),
        Some(ActOrder::Drive(_) | ActOrder::Mine(_) | ActOrder::Unload(_)) | None => None,
    }
}

/// **Test hook** (doc 20 cut SM.7; the SM.7 lead's pre-commit round, item 5): the ordinal of the
/// mining order `tv`'s RAM slot holds, if any — so a test can prove a landed order leaves the
/// slot. Reads only; nothing in the shipped lane calls it.
#[doc(hidden)]
#[must_use]
pub fn mine_order_for_tests(w: &World, tv: ObjId) -> Option<u32> {
    mine_order_of(w, tv).map(|order| order.ordinal)
}

/// **Test hook** (doc 20 cut SM.8; the fix round's tests/T4): the ordinal of the hand unload
/// order `tv`'s RAM slot holds, if any — so a test can pin the offer's ordinal filter on a stale
/// order still in the slot, and the prune that drops it. Reads only; nothing in the shipped lane
/// calls it.
#[doc(hidden)]
#[must_use]
pub fn unload_order_for_tests(w: &World, tv: ObjId) -> Option<u32> {
    unload_order_of(w, tv).map(|order| order.ordinal)
}

/// The mining order planned for `tv`, if its slot holds one.
fn mine_order_of(w: &World, tv: ObjId) -> Option<MineOrder> {
    match w
        .module_state::<ActOrders>()
        .and_then(|orders| orders.0.get(&tv))
    {
        Some(ActOrder::Mine(order)) => Some(MineOrder::clone(order)),
        Some(ActOrder::Drive(_) | ActOrder::Unload(_) | ActOrder::Plan(_)) | None => None,
    }
}

/// **`+tv/<n> drive <target> [go]`** (doc 20 §1 beat 7, §8, §10; cut SM.4b, the brief's 3.5;
/// lead rulings 1–4 and 9, R4): **the cheap gates first** — the target parsed (`DRIVE_USAGE`);
/// the stand (`stand_of`: from the bay the implied launch's whole gate; else on the site's
/// ground); the battery and the rig's rates (`TV_DARK`); the record (busy → `RIG_BUSY`; the launch
/// half's open excursion and the caps) — **then** the plan (`drive_plan`: the peeks, a cold site or
/// home wanted and refused `GROUND_COLD`; the target; ONE A*; no way home); then the reserve with
/// the real trip home (`RESERVE_LOW`); then the preview (`DRIVE_PREVIEW`) or, with `go`, the order
/// written ([`ActOrders`]) and ONE `tacsim_surface_act` submission with `act: drive` keyed
/// `tacsim.surface/{tv}/{ordinal}/act`. No licence and no tenure is read (lead ruling 9): a rig
/// may drive onto any block.
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line.
#[allow(clippy::too_many_lines)] // one ordered refusal list, then the preview or the go
pub(crate) fn drive_cmd(
    ctx: &mut Ctx<'_>,
    rig: &RigAt,
    words: &str,
    go: bool,
) -> Result<(), CmdError> {
    let n = rig.slot.to_string();
    let split: Vec<&str> = words.split_whitespace().collect();
    let Some(target) = drive::parse_target(&split) else {
        return Err(CmdError::msg(tv::DRIVE_USAGE.replace("{n}", &n)));
    };
    // Doc 20 cut SM.11 (D8): a live plan holds its rig.
    plan_holds(ctx, rig)?;
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
    // 1. The stand: from the bay the implied launch's whole gate, else on the site's ground.
    let stand = stand_of(ctx.w, actor, rig).map_err(|refusal| CmdError::msg(refusal.line()))?;
    // 2. The battery and the rates: a drive spends.
    let Some(battery) = rig_battery(ctx.w, &handles, rig) else {
        return Err(dark());
    };
    let Some(rates) = rig_rates(ctx.w, rig.tv, Some(&battery)) else {
        return Err(dark());
    };
    // 3. The record: busy, the launch half's open excursion and its caps.
    let record = record_of(ctx, &handles, rig.tv, Some(&battery)).map_err(|()| dark())?;
    if let Some(busy) = busy_refusal(ctx.w, rig, &record) {
        return Err(CmdError::msg(busy.line()));
    }
    // Doc 20 cut SM.7 (§6 "the next drive's preview" is legal during a run; decision R1): a run
    // holds the rig to its pocket, so the preview prints and `go` refuses.
    if go && let Some(running) = run_refusal(ctx.w, rig, &record) {
        return Err(CmdError::msg(running.line()));
    }
    if stand.from_bay {
        launch_half(ctx, &handles, rig, &record, &stand.site, &dark)?;
    }
    // 4. The plan: the peeks, the target, ONE A*, the way home (R4: never before the gates).
    let mut peeks = Peeks::default();
    let (plan, leg) = match drive_plan(ctx.w, rig, &stand, &target, &rates, &mut peeks) {
        Ok(planned) => planned,
        Err(refusal) => {
            want_cold(ctx, rig, &refusal);
            return Err(CmdError::msg(refusal.line()));
        }
    };
    let w: &World = ctx.w;
    // 5. The reserve, with the real trip home.
    let now_ic = w.world_ic_now().at;
    let (now, after, home) = reserve_words(
        &record,
        now_ic,
        stand.from_bay,
        (plan.cost_cu, plan.home_cu),
        &battery,
        rig.slot,
    )?;
    let target_text = drive::target_words(&target);
    let ramp = stand.ramp_label.clone();
    if !go {
        let launch = if stand.from_bay {
            tv::IMPLIED_LAUNCH
                .replace("{ramp}", &ramp)
                .replace("{site}", &stand.site_name)
        } else {
            String::new()
        };
        let time = if stand.from_bay {
            tv::TWO_LEGS
                .replace("{down}", &LAUNCH_S.to_string())
                .replace("{secs}", &leg.secs.to_string())
        } else {
            format!("{} IC s", leg.secs)
        };
        let real =
            match crate::port::voyage::real_until(w, now_ic.saturating_add(i64::from(plan.secs))) {
                Some(real) => format!(", about {}", tv::span_words(real)),
                None => String::new(),
            };
        let line = tv::DRIVE_PREVIEW
            .replace("{n}", &n)
            .replace("{mark}", &rig.mark)
            .replace("{launch}", &launch)
            .replace("{m}", &leg.metres.to_string())
            .replace("{target}", &target_text)
            .replace("{time}", &time)
            .replace("{real}", &real)
            .replace("{now}", &now.to_string())
            .replace("{after}", &after.to_string())
            .replace("{home}", &home.to_string())
            .replace("{arg}", &split.join(" "));
        crate::port::coop::say(ctx, line);
        return Ok(());
    }
    // `go`: the order, then ONE submission as the actor.
    let Some(ordinal) = record.ordinal.checked_add(1) else {
        tracing::error!(tv = rig.tv.0, "tacsim: a rig's act ordinal is exhausted");
        return Err(dark());
    };
    let Some(key) = act_key(rig.tv, ordinal) else {
        tracing::error!(tv = rig.tv.0, "tacsim: a rig act's key did not form");
        return Err(dark());
    };
    let due = now_ic.saturating_add(i64::from(plan.secs));
    let when = tv::due_words(w, due);
    put_order(
        ctx,
        rig.tv,
        ActOrder::Drive(DriveOrder {
            actor,
            ordinal,
            site: plan.site.clone(),
            secs: plan.secs,
            cost_cu: plan.cost_cu,
            home_cu: plan.home_cu,
            params: plan.params.clone(),
            from_bay: plan.from_bay,
        }),
    );
    // The review's correctness/F5: the start names the end cell this command planned, and the
    // prepare holds it to the order the offer fixed — so a second `go` before the first lands
    // cannot start the first's start on the second's target (the first then refuses "that rig's
    // orders moved on", and the told line never names a drive the world does not run).
    let Some(to_word) = order_to_word(&plan.params) else {
        tracing::error!(tv = rig.tv.0, "tacsim: a drive's end cell did not form");
        return Err(dark());
    };
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        Vec::new(),
        json!({
            "tv": rig.tv.0.to_string(),
            "act": wire_word(ActKind::Drive, false),
            "ordinal": ordinal,
            "to": to_word,
        }),
        key,
    );
    let line = if stand.from_bay {
        tv::DRIVE_FILED_LAUNCH
    } else {
        tv::DRIVE_FILED
    }
    .replace("{n}", &n)
    .replace("{ramp}", &ramp)
    .replace("{target}", &target_text)
    .replace("{when}", &when);
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
    peeks: &mut Peeks,
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
        ActParams::None {}
        | ActParams::Dump { .. }
        | ActParams::Drive { .. }
        | ActParams::Unload { .. } => None,
    }
}

// ---- doc 20 cut SM.7: mining ---------------------------------------------------------------------

/// **Which drill a rig carries** (doc 20 cut SM.7; `rig_scanner_id`'s rule): the lessor pool's
/// tools whose row in the one tools table is a drill.
#[must_use]
pub(crate) fn rig_drill_id(w: &World, rig_tv: ObjId) -> Option<CanonicalId> {
    crate::port::vehicle::rig_row(w, rig_tv)?;
    let tools = w.module_state::<PortHandles>()?.surface_tools.clone()?;
    let port = crate::port::coop::programme_port(w)?;
    let pool = port.def.vehicles.as_deref()?;
    pool.tools
        .iter()
        .find(|id| tools.get(*id).and_then(ToolDef::drill).is_some())
        .cloned()
}

/// **A drill's figures** by its id, from a tools table (`ToolDef::drill`).
#[must_use]
pub(crate) fn drill_of<'t>(
    tools: &'t BTreeMap<CanonicalId, ToolDef>,
    id: &CanonicalId,
) -> Option<&'t DrillDef> {
    tools.get(id).and_then(ToolDef::drill)
}

/// **A family's word on the verb** (`+tv/mine ice`): the first word `geology::family_of_word`
/// reads for it.
#[must_use]
pub(crate) fn family_word(family: OreFamily) -> &'static str {
    match family {
        OreFamily::Metals => "metal",
        OreFamily::Volatiles => "ice",
        OreFamily::CarbonFeedstock => "carbon",
        OreFamily::StrategicElements => "strategic",
    }
}

/// **The block's authored tenure, as mining reads it** (the brief's 3.3 item 5, reviewer F5):
/// `port::site::block_tenure`'s rule exactly — `(Commons, _)` commons; `(Held, Some(_))` held;
/// `(Open, _)` and `(Held, None)` open (a holder-less held block reads open, as the status line
/// and `tenure_parts` count it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Authored {
    /// Commons: no claim is read.
    Commons,
    /// Open ground.
    Open,
    /// An org's (until SM.10 seeds its claims).
    Held,
}

/// The authored word of `letter` on `site`.
#[must_use]
pub(crate) fn authored_of(site: &crate::port::def::SiteDef, letter: char) -> Authored {
    use crate::port::def::Tenure;
    let block = site.block(letter);
    match (block.tenure, block.holder) {
        (Tenure::Commons, _) => Authored::Commons,
        (Tenure::Held, Some(_)) => Authored::Held,
        (Tenure::Open, _) | (Tenure::Held, None) => Authored::Open,
    }
}

/// **Why tenure bars a block** (doc 20 cut SM.7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TenureBar {
    /// `MINE_CLAIMED`.
    Claimed,
    /// The claims or the issuer cannot be read: fail closed. Which read failed (the SM.7 lead's
    /// pre-commit round, item 3: a run's landing pages it and ends `RunEnd::Dark`).
    Dark(DarkCause),
}

/// **A live claim** (the brief's decision R20, reviewer F5): not lapsed, and a term not before
/// `now`; an uncomparable term (another clock generation, K4a) counts as **live** — a claim we
/// cannot date bars the block (fail closed). A claim past its term that nothing has lapsed is not
/// live (SM.10 lapses such claims; before it a staff-filed claim can outlive its term).
#[must_use]
pub(crate) fn claim_live(claim: &mush_industry::ClaimRecord, now: &mush_core::ClockStamp) -> bool {
    !claim.is_lapsed() && !matches!(claim.term().checked_cmp(now), Ok(std::cmp::Ordering::Less))
}

/// **The tenure verdict, pure** (doc 20 §6 "tenure allows it"; the brief's 3.3 item 5, decision
/// R5): commons → Ok with no claim read; then the **live** claims of the site's issuer over the
/// block's opened deposits (`claims`, filtered by the caller to the issuer): none live → an
/// authored held block refuses `Claimed` (R5 — since doc 20 cut SM.10 the fallback for a held
/// block the site clerk has not seeded yet: a seeded one carries the org's live claims, F15),
/// open ground is Ok; some live → Ok iff `authorizes` one of them — **whatever its
/// contest state** (a suspended or seized claim authorizes nobody, so it bars its holder too:
/// industry's rule), else `Claimed`. Block-level: it answers the same whatever family a run names.
///
/// # Errors
///
/// [`TenureBar::Claimed`].
pub(crate) fn tenure_verdict(
    authored: Authored,
    claims: &[mush_industry::ClaimRecord],
    now: &mush_core::ClockStamp,
    authorizes: &dyn Fn(&mush_industry::ClaimRecord) -> bool,
) -> Result<(), TenureBar> {
    if authored == Authored::Commons {
        return Ok(());
    }
    let live: Vec<&mush_industry::ClaimRecord> = claims
        .iter()
        .filter(|claim| claim_live(claim, now))
        .collect();
    if live.is_empty() {
        return if authored == Authored::Held {
            Err(TenureBar::Claimed)
        } else {
            Ok(())
        };
    }
    if live.iter().any(|claim| authorizes(claim)) {
        Ok(())
    } else {
        Err(TenureBar::Claimed)
    }
}

/// **The tenure of a block, for `actor`** (doc 20 §6; the brief's 3.3 item 5 and decision R5):
/// the block's authored word read directly ([`authored_of`]; never `port::site::block_tenure`'s
/// display string); commons answers at once; else the claims over every opened deposit of the
/// block (the four families' receipt-derived ids that `reads.deposit` finds) whose issuer is the
/// site's issuer principal (`port::site::org_principal`), judged by [`tenure_verdict`] with
/// `mush_industry::extraction_authorized` over that claim's own deposit (`remaining` its initial
/// reserve: the cursor, not the pool, sets the tonnes, §3.4) and industry's licence instrument.
/// **Doc 20 cut SM.10**: then the block's public record (`tacsim.claims`): a live claim held by
/// neither the actor nor someone the actor `acts_for` refuses `Claimed` — the one answer for a
/// block with no opened deposit. **Read only**: no industry family is written (SM.6's D10
/// tripwire).
///
/// # Errors
///
/// [`TenureBar`]: claimed, or the issuer, a deposit or a claim cannot be read.
pub(crate) fn mine_tenure(
    w: &World,
    reads: &dyn SurfaceReads,
    actor: ObjId,
    (entry, site): (&CanonicalId, &crate::port::def::SiteDef),
    letter: char,
    now: &mush_core::ClockStamp,
) -> Result<(), TenureBar> {
    let authored = authored_of(site, letter);
    if authored == Authored::Commons {
        return Ok(());
    }
    let Some(issuer) = crate::port::site::org_principal(w, &site.issuer) else {
        tracing::error!(
            site = %entry,
            issuer = %site.issuer,
            "tacsim: a mining site's issuer does not resolve; its blocks refuse mining (fail closed)"
        );
        return Err(TenureBar::Dark(DarkCause::Issuer));
    };
    let instrument = w
        .module_state::<mush_industry::IndustryStore>()
        .and_then(|store| store.licence().cloned());
    let mut claims: Vec<mush_industry::ClaimRecord> = Vec::new();
    let mut bodies: BTreeMap<mush_industry::ClaimId, mush_industry::OreDeposit> = BTreeMap::new();
    for family in OreFamily::ALL {
        let Ok(id) = mush_industry::derive_system_deposit_id(&crate::surface::opening::receipt(
            entry, letter, family,
        )) else {
            continue;
        };
        let Some(body) = reads
            .deposit(id)
            .map_err(|_| TenureBar::Dark(DarkCause::Deposit))?
        else {
            continue;
        };
        for claim in reads
            .claims(id)
            .map_err(|_| TenureBar::Dark(DarkCause::Claims))?
        {
            if claim.issuer() == &issuer {
                bodies.insert(claim.id(), body.clone());
                claims.push(claim);
            }
        }
    }
    let operator = mush_org::PrincipalId::Object(actor);
    tenure_verdict(authored, &claims, now, &|claim| {
        bodies.get(&claim.id()).is_some_and(|body| {
            mush_industry::extraction_authorized(
                w,
                actor,
                &operator,
                body,
                body.initial_reserve(),
                std::slice::from_ref(claim),
                instrument.as_ref(),
                now,
            )
            .is_ok()
        })
    })?;
    // Doc 20 cut SM.10 (the brief's 3.8 item 21, reviewer F4; a stated, narrow touch of SM.7's
    // gate): the block's public record too. On an ore-bearing block the two always agree (both
    // are written in one bundle); on a block with no opened deposit the record alone answers, so
    // that block refuses a stranger exactly as an ore block does. A live claim held by neither
    // the actor nor someone the actor acts for bars it; a past-term one is open ground (R20).
    let record = reads
        .block_tenure(&crate::surface::claims::BlockKey {
            site: entry.clone(),
            letter,
        })
        .map_err(|_| TenureBar::Dark(DarkCause::Claims))?;
    match record.and_then(|record| record.claim) {
        Some(claim)
            if claim.live(now.at)
                && claim.holder != operator
                && !mush_economy::acts_for(w, actor, &claim.holder) =>
        {
            Err(TenureBar::Claimed)
        }
        _ => Ok(()),
    }
}

/// The live lease's bar as a mining refusal.
fn mine_lease_refusal(w: &World, rig: &RigAt, bar: LeaseBar) -> Refusal {
    let (counter, port) = tv::counter_words(w);
    match bar {
        LeaseBar::Forfeit => Refusal::MineForfeit {
            n: rig.slot,
            counter,
            port,
        },
        LeaseBar::Ended => Refusal::MineEnded {
            n: rig.slot,
            counter,
            port,
        },
    }
}

/// **The licence, as mining reads it** (lead ruling 5; reviewer F21): in force or in its grace →
/// its lapse; no licence → `MINE_NO_LICENCE`; every other standing → `+licence`'s own line
/// (`crate::port::licence::standing_line`).
fn mine_licence(w: &World, actor: ObjId, now_ic: i64) -> Result<i64, Refusal> {
    use crate::port::licence::LicenceStanding;
    match crate::port::licence::licence_standing(w, actor, now_ic) {
        LicenceStanding::InForce { lapses_ic, .. } | LicenceStanding::Grace { lapses_ic, .. } => {
            Ok(lapses_ic)
        }
        LicenceStanding::Unlicensed => Err(Refusal::Licence {
            line: tv::MINE_NO_LICENCE.to_owned(),
        }),
        other => Err(Refusal::Licence {
            line: crate::port::licence::standing_line(w, actor, &other)
                .unwrap_or_else(|| tv::MINE_NO_LICENCE.to_owned()),
        }),
    }
}

/// **The licence's verdict for the next cycle** (the brief's 3.7): `Ok(lapses_ic)` in force or
/// in its grace, else the standing. A `Lapsed { at_ic }` read at a late landing is still judged
/// against the next cycle's start, never against now (`RightsVerdict::stops_next`, R9).
pub(crate) fn licence_verdict(w: &World, actor: ObjId, now_ic: i64) -> Result<i64, LicenceEnd> {
    use crate::port::licence::LicenceStanding;
    match crate::port::licence::licence_standing(w, actor, now_ic) {
        LicenceStanding::InForce { lapses_ic, .. } | LicenceStanding::Grace { lapses_ic, .. } => {
            Ok(lapses_ic)
        }
        LicenceStanding::Lapsed { at_ic } => Err(LicenceEnd::Lapsed { at_ic }),
        LicenceStanding::Unlicensed => Err(LicenceEnd::Unlicensed),
        LicenceStanding::Suspended => Err(LicenceEnd::Suspended),
        LicenceStanding::Other => Err(LicenceEnd::Other),
        LicenceStanding::Unreadable => Err(LicenceEnd::Unreadable),
    }
}

/// **A stored licence end as `+licence`'s line** (the tell's; reviewer F1: formatted at the tell,
/// never stored): `MINE_NO_LICENCE` for no licence, else `standing_line`'s.
#[must_use]
pub(crate) fn licence_end_line(w: &World, operator: ObjId, why: LicenceEnd) -> String {
    use crate::port::licence::LicenceStanding;
    let standing = match why {
        LicenceEnd::Lapsed { at_ic } => LicenceStanding::Lapsed { at_ic },
        LicenceEnd::Unlicensed => return tv::MINE_NO_LICENCE.to_owned(),
        LicenceEnd::Suspended => LicenceStanding::Suspended,
        LicenceEnd::Other => LicenceStanding::Other,
        LicenceEnd::Unreadable => LicenceStanding::Unreadable,
    };
    crate::port::licence::standing_line(w, operator, &standing)
        .unwrap_or_else(|| tv::MINE_NO_LICENCE.to_owned())
}

/// **The site the actor stands aboard** (doc 20 cut SM.10, D11: a contest is filed from aboard
/// the hull settled on the site): the rig's hull's view (`view_of`), its site berth
/// (`site_berth`: under way, not a site, dark) and the actor aboard it (`TV_NOT_ABOARD`); the
/// site's entry and name.
///
/// # Errors
///
/// The [`Refusal`] the launch's own gates answer.
pub(crate) fn aboard_site(
    w: &World,
    actor: ObjId,
    rig: &RigAt,
) -> Result<(CanonicalId, String), Refusal> {
    let (view, name) = view_of(w, rig)?;
    let berth = site_berth(w, &view)?;
    if !crate::port::voyage::aboard(w, rig.hull, actor) {
        return Err(Refusal::NotAboard { name });
    }
    Ok((berth.entry.clone(), berth.name.clone()))
}

/// The lane site definition of `entry` (cloned), when the latched lane authors one.
pub(crate) fn site_def_of(w: &World, entry: &CanonicalId) -> Option<crate::port::def::SiteDef> {
    let store = w.module_state::<TacsimStore>()?;
    let (_, ports) = crate::port::readiness::report_ports(store, w);
    ports
        .into_iter()
        .find(|(id, _)| id == entry)
        .and_then(|(_, def)| def.site.clone())
}

/// **One measure-wise room less `units` of a good** (saturating), for the preview's hold room
/// after the run.
fn room_less(room: &Load, per_unit: &[(CanonicalId, u64)], units: u64) -> Load {
    room.iter()
        .map(|(measure, left)| {
            let per = per_unit
                .iter()
                .find(|(m, _)| m == measure)
                .map_or(0, |(_, per)| *per);
            (
                measure.clone(),
                left.saturating_sub(units.saturating_mul(per)),
            )
        })
        .collect()
}

/// **The R1-richest family of the operator's own row of a field** (the brief's decision R6): each
/// family's reading ranked by its upper word then its lower (`Exact(w)` is `(w, w)`, a band
/// `(high, low)`), ties to `OreFamily::ALL`'s order; `None` when the row does not cover the field,
/// reads ice, or reads barren in every family.
fn richest_read(
    site: &chart::SiteContext,
    row: Option<&ChartRecord>,
    field: (i64, i64),
) -> Option<OreFamily> {
    use crate::surface::reading::FieldReading;
    let footing = chart::row_reads_field(row, field)?;
    let ground = chart::field_ground_at(site, field).ok()?;
    let readings = crate::surface::reading::read_r1(&site.mac, &site.body, &ground, footing);
    let mut best: Option<((u8, u8), OreFamily)> = None;
    for (family, reading) in OreFamily::ALL.into_iter().zip(readings) {
        let rank = match reading {
            FieldReading::Exact(word) => (word.code(), word.code()),
            FieldReading::Band(low, high) => (high.code(), low.code()),
            FieldReading::NoReturn => continue,
        };
        if best.is_none_or(|(kept, _)| rank > kept) {
            best = Some((rank, family));
        }
    }
    best.filter(|((high, _), _)| *high > 0)
        .map(|(_, family)| family)
}

/// **The mining plan, against the world** (doc 20 §6; the SM.7 brief's 3.3; lead rulings 2–5,
/// decisions R4–R6, R8, R11, R15, R16), called by `+tv/mine` (`reads` the command's `Ctx`) and by
/// the start's offer (`reads` the engine's snapshot) — one function, so the offer's re-plan equals
/// the preview's unless the world moved. **The gates, cheap first**:
///
/// 1. **the stand** — the layer and the view; under way; a rig in the bay → `MINE_IN_BAY` (no
///    implied launch: the drill works only a pocket it stands on); the ground half of `stand_of`
///    (`RECOVER_ELSEWHERE`); **then the lease** ([`lease_gate`]: `MINE_FORFEIT`, `MINE_ENDED`);
/// 2. **the pocket** under the stand: its address, block letter and number;
/// 3. **the site's ground** — resolved (`INSTRUMENTS_DARK`), the composite and the home field
///    peeked (`GROUND_COLD`, `INSTRUMENTS_DARK`), the footing (`MINE_NO_FOOTING`);
/// 4. **the licence** (`MINE_NO_LICENCE`, else `+licence`'s line);
/// 5. **the tenure** ([`mine_tenure`]: `MINE_CLAIMED`);
/// 6. **the record**: an act in flight → `RIG_BUSY`; a run → `MINE_RUNNING`;
/// 7. **the lock**: another rig's live lock on any family's row of the pocket, or another rig's
///    run on this pocket (decision R4: a barren pocket has no row) → `MINE_LOCKED`;
/// 8. **the reading and the family** — the operator's OWN row: read → the classes of their R2
///    (the named family barren → `MINE_NO_ORE`; every family barren → `MINE_ALL_BARREN`; the
///    default the family with most tonnes left, ties to the richer class); unread → a blind core:
///    the family named, else the operator's R1-richest, else metals (decision R6); a count past one
///    → `MINE_BLIND_COUNT`; **8b** the public pit (`site_map::pit_at`) shapes the blind preview by
///    one clause and never refuses (decision R16);
/// 9. **the truth, server-side** (never printed for a blind core): the class's yield, the pocket's
///    tonnes, the cursor's drawn, the lock rows (≤ [`MAX_LOCK_ROWS`]), the block vault;
/// 10. **the yields and the fit** — read: `pocket_yields` (empty → `MINE_WORKED_OUT`) and
///     `mine::fit` against the hopper's and the hold's unreserved room in every measure (a named
///     count that fits; past the fit `MINE_NO_FIT` naming the count that does; nothing fits → the
///     bound's line); blind: one cycle needing the blind room in both (`MINE_BLIND_ROOM`, the only
///     room line);
/// 11. **the chart's body caps** (blind only: the core adds the block to the row) → `CHART_FULL`.
///
/// The reserve is the caller's (the command's words, the prepare's rule), over the plan's run
/// cost and its trip home.
///
/// # Errors
///
/// The first [`Refusal`] in that order.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)] // one ordered gate list
pub(crate) fn mine_plan(
    w: &World,
    reads: &dyn SurfaceReads,
    actor: ObjId,
    rig: &RigAt,
    choice: &MineChoice,
    drill: &DrillDef,
    rates: Option<&DriveRates>,
    peeks: &mut Peeks,
) -> Result<MinePlan, Refusal> {
    let n = rig.slot;
    // 1. The stand, then the lease.
    let (view, name) = view_of(w, rig)?;
    if matches!(berth_of(w, rig.hull), Berth::UnderWay) {
        return Err(Refusal::UnderWay { name });
    }
    if where_is(w, rig.tv) == (Where::Bay { hull: rig.hull }) {
        return Err(Refusal::MineInBay { n });
    }
    let stand = stand_of(w, actor, rig)?;
    let berth = site_berth(w, &view)?;
    if let Err(bar) = lease_gate(w, &rig.record) {
        return Err(mine_lease_refusal(w, rig, bar));
    }
    let dark = || Refusal::Dark { name: name.clone() };
    // 2. The pocket.
    let (px, py) = address::pocket_of_cell(stand.cell).ok_or_else(dark)?;
    let addr = address::pocket_address(px, py).ok_or_else(dark)?;
    let letter = address::block_letter(px, py).ok_or_else(dark)?;
    let number = address::pocket_number(px, py).ok_or_else(dark)?;
    // 3. The site's ground: resolved, peeked, the way home, the footing.
    let site = chart::site_context(w, &stand.site).map_err(|_| Refusal::InstrumentsDark)?;
    let ground = match chart::peek_site(w, &stand.site, peeks) {
        SitePeek::Warm(ground) => ground,
        SitePeek::Cold => {
            return Err(Refusal::GroundCold {
                site: stand.site_name.clone(),
                entry: stand.site.clone(),
            });
        }
        SitePeek::Dark => return Err(Refusal::InstrumentsDark),
    };
    let home_cu = home_here(w, &stand, rates, &name, peeks)?;
    let surface = chart::ground_surface(&ground).ok_or(Refusal::InstrumentsDark)?;
    let footing =
        chart::footing_at(surface, stand.cell).map_err(|terrain| Refusal::MineNoFooting {
            n,
            terrain: terrain.label().replace('_', " "),
        })?;
    // 4. The licence.
    let now = w.world_ic_now();
    let lapses_ic = mine_licence(w, actor, now.at)?;
    // 5. The tenure, block-level.
    let site_def = berth.def.site.as_ref().ok_or_else(dark)?;
    mine_tenure(w, reads, actor, (&stand.site, site_def), letter, &now).map_err(
        |bar| match bar {
            TenureBar::Claimed => Refusal::Claimed,
            TenureBar::Dark(_) => dark(),
        },
    )?;
    // 6. The record: busy, or drilling.
    let record = reads.rig(rig.tv).map_err(|error| {
        tracing::warn!(tv = rig.tv.0, %error, "tacsim: a rig's excursion record does not read");
        dark()
    })?;
    if let Some(record) = &record {
        if let Some(busy) = busy_refusal(w, rig, record) {
            return Err(busy);
        }
        if let Some(running) = run_refusal(w, rig, record) {
            return Err(running);
        }
    }
    // 7. The lock: the cursor rows (one home a family), then every other rig's run.
    let mut deposits: Vec<(OreFamily, mush_industry::OreDepositId)> = Vec::new();
    let mut cursors: BTreeMap<u8, PocketCursor> = BTreeMap::new();
    for family in OreFamily::ALL {
        let receipt = crate::surface::opening::receipt(&stand.site, letter, family);
        let id = mush_industry::derive_system_deposit_id(&receipt).map_err(|_| dark())?;
        deposits.push((family, id));
        if let Some(cursor) = reads.cursor(id).map_err(|_| dark())? {
            if let Some(lock) = cursor.row(number).and_then(|row| row.lock.as_ref())
                && lock.tv != rig.tv
            {
                let holder = reads.rig(lock.tv).map_err(|_| dark())?;
                let row = LockRow {
                    deposit: id,
                    pocket: number,
                };
                if lock_live(lock, holder.as_ref(), &stand.site, &row) {
                    return Err(Refusal::Locked);
                }
            }
            cursors.insert(family.code(), cursor);
        }
    }
    for other in world_rigs(w).into_iter().filter(|other| *other != rig.tv) {
        let found = reads.rig(other).map_err(|_| dark())?;
        if found
            .and_then(|record| record.run)
            .is_some_and(|run| run.site == stand.site && run.pocket == [px, py])
        {
            return Err(Refusal::Locked);
        }
    }
    let drawn_of = |family: OreFamily| {
        cursors
            .get(&family.code())
            .and_then(|cursor| cursor.row(number))
            .map_or(0, |row| row.drawn_t)
    };
    // 8. The reading and the family — the operator's own row, never the ground.
    let row = reads.chart(actor, &stand.site).map_err(|_| dark())?;
    let read = chart::row_reads_pocket(row.as_ref(), (px, py));
    let truth = chart::pocket_at(&site, (px, py)).map_err(|_| Refusal::InstrumentsDark)?;
    let fam = choice
        .word
        .as_ref()
        .map_or_else(String::new, |word| format!(" {word}"));
    let ore_of = |family: OreFamily| tv::ore_words(crate::surface::opening::ore_good(family));
    let classes = crate::surface::reading::read_r2(&truth);
    let family = if read {
        if let Some(named) = choice.family {
            if by_family(classes, named) == Word::Barren {
                return Err(Refusal::NoOre {
                    addr,
                    ore: ore_of(named),
                });
            }
            named
        } else {
            let mut best: Option<((u32, u8), OreFamily)> = None;
            for family in OreFamily::ALL {
                let class = by_family(classes, family);
                if class == Word::Barren {
                    continue;
                }
                let left =
                    u32::from(by_family(truth.tonnes, family)).saturating_sub(drawn_of(family));
                let rank = (left, class.code());
                if best.is_none_or(|(kept, _)| rank > kept) {
                    best = Some((rank, family));
                }
            }
            let Some((_, family)) = best else {
                return Err(Refusal::AllBarren { addr });
            };
            family
        }
    } else {
        if choice.cycles.is_some_and(|m| m > 1) {
            return Err(Refusal::BlindCount { n, fam });
        }
        choice
            .family
            .or_else(|| {
                address::field_of_pocket((px, py))
                    .and_then(|field| richest_read(&site, row.as_ref(), field))
            })
            .unwrap_or(OreFamily::Metals)
    };
    let blind = !read;
    let also: Vec<(OreFamily, Word)> = if blind {
        Vec::new()
    } else {
        OreFamily::ALL
            .into_iter()
            .filter(|other| *other != family)
            .map(|other| (other, by_family(classes, other)))
            .filter(|(_, class)| *class != Word::Barren)
            .collect()
    };
    // 8b. The public pit (blind only): one clause, never a refusal.
    let pit = if blind {
        crate::port::site_map::pit_at(reads, &stand.site, &ground, (px, py))
    } else {
        None
    };
    // 9. The truth, server-side.
    let class = by_family(classes, family);
    let pocket_t = by_family(truth.tonnes, family);
    let class_t = u8::try_from(class_yield(drill, class)).map_err(|_| dark())?;
    let drawn = drawn_of(family);
    let deposit = deposits
        .iter()
        .find(|(f, _)| *f == family)
        .map(|(_, id)| *id)
        .ok_or_else(dark)?;
    let mut lock_rows: Vec<LockRow> = Vec::new();
    for (other, id) in &deposits {
        if by_family(truth.tonnes, *other) > 0 && reads.deposit(*id).map_err(|_| dark())?.is_some()
        {
            lock_rows.push(LockRow {
                deposit: *id,
                pocket: number,
            });
        }
    }
    if lock_rows.len() > MAX_LOCK_ROWS {
        tracing::error!(
            site = %stand.site,
            %addr,
            rows = lock_rows.len(),
            "tacsim: a pocket carries more ore families than the geology allows (SM.1's two); \
             mining it is refused"
        );
        return Err(dark());
    }
    let source = if class_t > 0 && pocket_t > 0 {
        let Some(vault) = crate::port::site::block_vault(w, berth.map, letter) else {
            tracing::error!(
                site = %stand.site,
                block = %letter,
                "tacsim: an ore-bearing block's vault is gone; mining it is refused (staff repair \
                 the site map)"
            );
            return Err(dark());
        };
        Some(vault.0)
    } else {
        None
    };
    // 10. The yields and the fit.
    let good = CanonicalId::new(crate::surface::opening::ore_good(family)).map_err(|_| dark())?;
    let goods = w
        .module_state::<TacsimStore>()
        .and_then(TacsimStore::goods)
        .ok_or_else(dark)?;
    let good_def = goods.good(&good).ok_or_else(dark)?;
    let pool_unit = tv::pool_unit(w);
    let hopper = crate::port::vehicle::hopper_of(w, rig.tv).ok_or_else(dark)?;
    let read_pool = |endpoint: EndpointRef, good: &CanonicalId| reads.pool(endpoint, good);
    let contents = tv::hopper_contents_with(w, rig.tv, pool_unit.as_ref(), &read_pool).map_err(
        |error| {
            tracing::warn!(tv = rig.tv.0, %error, "tacsim: a mining plan could not read the hopper");
            dark()
        },
    )?;
    let hopper_room = {
        let comps = tv::hopper_compartments(w, rig.tv, pool_unit.as_ref()).ok_or_else(dark)?;
        let mut merged = crate::port::capacity::Compartment {
            id: CanonicalId::new("hopper").map_err(|_| dark())?,
            accepts: std::collections::BTreeSet::new(),
            limits: BTreeMap::new(),
        };
        for comp in comps {
            merged.accepts.extend(comp.accepts);
            for (measure, limit) in comp.limits {
                let slot = merged.limits.entry(measure).or_insert(0);
                *slot = slot.saturating_add(limit);
            }
        }
        let load = crate::port::capacity::load(goods, &merged, &contents).map_err(|_| dark())?;
        merged
            .limits
            .iter()
            .map(|(measure, limit)| {
                (
                    measure.clone(),
                    limit.saturating_sub(load.get(measure).copied().unwrap_or(0)),
                )
            })
            .collect::<Load>()
    };
    let mass = CanonicalId::new(crate::definitions::logistics::MASS_KG).map_err(|_| dark())?;
    let hopper_cap_kg = tv::hopper_compartments(w, rig.tv, pool_unit.as_ref())
        .map(|comps| {
            comps
                .iter()
                .filter_map(|comp| comp.limits.get(&mass).copied())
                .sum::<u64>()
        })
        .unwrap_or(0);
    let hopper_before_kg =
        hopper_cap_kg.saturating_sub(hopper_room.get(&mass).copied().unwrap_or(0));
    let hull_rigs: Vec<ObjId> = tv::live_on_hull(w, rig.hull)
        .iter()
        .map(|lease| lease.tv)
        .collect();
    let hold_room =
        crate::port::ship::hold_unreserved(w, &view, &hull_rigs, &read_pool, &|tv| reads.rig(tv))
            .ok_or_else(dark)?;
    let mut per_unit: Vec<(CanonicalId, u64)> = Vec::new();
    for measure in hopper_room.keys().chain(hold_room.keys()) {
        if per_unit.iter().any(|(m, _)| m == measure) {
            continue;
        }
        per_unit.push((
            measure.clone(),
            good_def.per_unit(measure).ok_or_else(dark)?,
        ));
    }
    let (yields, reserve_t, cycles): (Vec<u32>, Vec<u32>, u8) = if blind {
        let room = blind_room_t(drill);
        if fit(&[room], &per_unit, &hopper_room, &hold_room).0 == 0 {
            return Err(Refusal::BlindRoom { room });
        }
        (Vec::new(), vec![room], 1)
    } else {
        let all = pocket_yields(
            u32::from(class_t),
            u32::from(pocket_t).saturating_sub(drawn),
        );
        let ore = ore_of(family);
        if all.is_empty() {
            return Err(Refusal::WorkedOut { addr, ore });
        }
        let (n_fit, bound) = fit(&all, &per_unit, &hopper_room, &hold_room);
        let zero = |bound: Bound| match bound {
            Bound::Hopper => Refusal::HopperFull {
                n,
                hopper: tv::hopper_text(hopper_before_kg, hopper_cap_kg),
            },
            Bound::Hold => Refusal::HoldFull { name: name.clone() },
            Bound::WorkedOut | Bound::Cap => Refusal::WorkedOut {
                addr: addr.clone(),
                ore: ore.clone(),
            },
        };
        let cycles = match choice.cycles {
            Some(m) if m <= n_fit => m,
            Some(_) | None if n_fit == 0 => return Err(zero(bound)),
            Some(m) => {
                return Err(Refusal::NoFit {
                    n,
                    m,
                    fit: n_fit,
                    bound,
                    fam,
                });
            }
            None => n_fit,
        };
        let taken: Vec<u32> = all.into_iter().take(usize::from(cycles)).collect();
        (taken.clone(), taken, cycles)
    };
    let reserve_t: Vec<u8> = reserve_t
        .into_iter()
        .map(u8::try_from)
        .collect::<Result<_, _>>()
        .map_err(|_| dark())?;
    let run_t: u64 = reserve_t.iter().map(|t| u64::from(*t)).sum();
    let hold_after = room_less(&hold_room, &per_unit, run_t);
    let mass_per = good_def.per_unit(&mass).unwrap_or(0);
    let shown_t: u64 = yields.iter().map(|t| u64::from(*t)).sum();
    let hopper_after_kg = hopper_before_kg.saturating_add(shown_t.saturating_mul(mass_per));
    let mut outstanding: Vec<(ObjId, Option<(u32, u64)>)> = Vec::new();
    for tv in &hull_rigs {
        let found = reads.rig(*tv).map_err(|_| dark())?;
        outstanding.push((
            *tv,
            found
                .and_then(|record| record.run)
                .map(|run| (run.ordinal, run.outstanding_t())),
        ));
    }
    // 11. The chart's body caps (blind: the core adds the block to the row).
    if blind {
        let mut rows = Vec::new();
        for other in chart::body_sites(w, &site.spec.body) {
            if let Some(found) = reads.chart(actor, &other).map_err(|_| dark())? {
                rows.push(found);
            }
        }
        if !body_admits(&rows, &stand.site, &[], &[letter.to_string()]) {
            return Err(Refusal::ChartFull {
                body: crate::port::site::body_name(&site.spec.body),
            });
        }
    }
    let secs = cycle_secs(drill, footing).ok_or_else(dark)?;
    Ok(MinePlan {
        site: stand.site.clone(),
        body: site.spec.body.clone(),
        stand: [stand.cell.0, stand.cell.1],
        pocket: [px, py],
        number,
        letter,
        addr,
        family,
        good,
        deposit,
        source,
        hopper: hopper.0,
        cycles,
        yields,
        reserve_t,
        class_t,
        pocket_t,
        drawn,
        class: if blind { Word::Barren } else { class },
        also,
        blind,
        pit,
        secs,
        cost_cu: drill.energy_cu,
        home_cu,
        lock_rows,
        lapses_ic: Some(lapses_ic),
        hold_after,
        hopper_before_kg,
        hopper_after_kg,
        hopper_cap_kg,
        outstanding,
    })
}

/// **`+tv/<n> mine [family] [cycles] [go]`** (alias `drill`; doc 20 §1 beat 8, §6, §10; cut SM.7,
/// the brief's 3.4; D-20-9's `go` rule): the drill and the battery (a run spends: `TV_DARK`
/// without either), the rig's rates; [`mine_plan`] (a cold site or home asked for and refused
/// `GROUND_COLD`); the reserve over the run's cost and the trip home (`RESERVE_LOW`); then the
/// preview — `MINE_PREVIEW` from the operator's own reading, or `MINE_PREVIEW_BLIND` — or, with
/// `go`, the order written ([`ActOrders`]) and ONE `tacsim_surface_act` submission with `act:
/// mine` keyed `tacsim.surface/{tv}/{ordinal}/act`, and `MINE_FILED` (`CORE_FILED` blind).
/// `words` are the family and count as typed (the preview's `{arg}`).
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line.
#[allow(clippy::too_many_lines)] // the plan, the reserve, then the preview or the go
pub(crate) fn mine_cmd(
    ctx: &mut Ctx<'_>,
    rig: &RigAt,
    choice: &MineChoice,
    words: &str,
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
    // Doc 20 cut SM.11 (D8): a live plan holds its rig.
    plan_holds(ctx, rig)?;
    let name = name_of(ctx.w, rig.hull);
    let dark = || CmdError::msg(tv::TV_DARK.replace("{name}", &name));
    let tools = handles.surface_tools.clone().ok_or_else(dark)?;
    let Some(drill) = rig_drill_id(ctx.w, rig.tv).and_then(|id| drill_of(&tools, &id).copied())
    else {
        tracing::warn!(tv = rig.tv.0, "tacsim: a rig carries no readable drill");
        return Err(dark());
    };
    // A run spends (`needs_battery(Mine, …)`): no battery row, no run.
    let Some(battery) = rig_battery(ctx.w, &handles, rig) else {
        return Err(dark());
    };
    let rates = rig_rates(ctx.w, rig.tv, Some(&battery));
    let mut peeks = Peeks::default();
    let planned = mine_plan(
        ctx.w,
        &*ctx,
        actor,
        rig,
        choice,
        &drill,
        rates.as_ref(),
        &mut peeks,
    );
    let plan = match planned {
        Ok(plan) => plan,
        Err(refusal) => {
            want_cold(ctx, rig, &refusal);
            return Err(CmdError::msg(refusal.line()));
        }
    };
    let w: &World = ctx.w;
    let record = record_of(ctx, &handles, rig.tv, Some(&battery)).map_err(|()| dark())?;
    let now_ic = w.world_ic_now().at;
    let run_cost = plan.run_cost_cu().ok_or_else(dark)?;
    let run_secs = plan.run_secs().ok_or_else(dark)?;
    let (now, after, home) = reserve_words(
        &record,
        now_ic,
        false,
        (run_cost, plan.home_cu),
        &battery,
        rig.slot,
    )?;
    let n = rig.slot.to_string();
    let ore = tv::ore_words(plan.good.as_str());
    let arg = if words.is_empty() {
        String::new()
    } else {
        format!(" {words}")
    };
    if !go {
        let real =
            match crate::port::voyage::real_until(w, now_ic.saturating_add(i64::from(run_secs))) {
                Some(real) => format!(", about {}", tv::span_words(real)),
                None => String::new(),
            };
        let line = if plan.blind {
            let pit = match plan.pit {
                Some('v') => tv::PIT_WORKED,
                Some(_) => tv::PIT_WORKED_OUT,
                None => "",
            };
            tv::MINE_PREVIEW_BLIND
                .replace("{n}", &n)
                .replace("{mark}", &rig.mark)
                .replace("{addr}", &plan.addr)
                .replace("{ore}", &ore)
                .replace("{secs}", &run_secs.to_string())
                .replace("{real}", &real)
                .replace(
                    "{room}",
                    &plan.reserve_t.first().copied().unwrap_or(0).to_string(),
                )
                .replace("{now}", &now.to_string())
                .replace("{after}", &after.to_string())
                .replace("{home}", &home.to_string())
                .replace("{pit}", pit)
                .replace("{arg}", &arg)
        } else {
            let cycles = if plan.cycles == 1 {
                tv::ONE_CYCLE.to_owned()
            } else {
                tv::SOME_CYCLES.replace("{c}", &plan.cycles.to_string())
            };
            let yields: Vec<String> = plan.yields.iter().map(u32::to_string).collect();
            let total: u32 = plan.yields.iter().sum();
            let mass = CanonicalId::new(crate::definitions::logistics::MASS_KG).ok();
            let room_kg = mass
                .and_then(|mass| plan.hold_after.get(&mass).copied())
                .unwrap_or(0);
            let also: String = plan
                .also
                .iter()
                .map(|(family, class)| {
                    tv::MINE_ALSO
                        .replace("{class}", class.name())
                        .replace(
                            "{ore}",
                            &tv::ore_words(crate::surface::opening::ore_good(*family)),
                        )
                        .replace("{n}", &n)
                        .replace("{word}", family_word(*family))
                })
                .collect();
            tv::MINE_PREVIEW
                .replace("{n}", &n)
                .replace("{mark}", &rig.mark)
                .replace("{addr}", &plan.addr)
                .replace("{class}", plan.class.name())
                .replace("{ore}", &ore)
                .replace("{cycles}", &cycles)
                .replace("{yields}", &yields.join("/"))
                .replace("{total}", &total.to_string())
                .replace("{secs}", &run_secs.to_string())
                .replace("{real}", &real)
                .replace("{hop0}", &(plan.hopper_before_kg / 1_000).to_string())
                .replace("{hop1}", &(plan.hopper_after_kg / 1_000).to_string())
                .replace("{cap}", &(plan.hopper_cap_kg / 1_000).to_string())
                .replace("{room}", &(room_kg / 1_000).to_string())
                .replace("{now}", &now.to_string())
                .replace("{after}", &after.to_string())
                .replace("{home}", &home.to_string())
                .replace("{also}", &also)
                .replace("{arg}", &arg)
        };
        crate::port::coop::say(ctx, line);
        return Ok(());
    }
    // `go`: the order, then ONE submission as the actor.
    let Some(ordinal) = record.ordinal.checked_add(1) else {
        tracing::error!(tv = rig.tv.0, "tacsim: a rig's act ordinal is exhausted");
        return Err(dark());
    };
    let Some(key) = act_key(rig.tv, ordinal) else {
        tracing::error!(tv = rig.tv.0, "tacsim: a rig act's key did not form");
        return Err(dark());
    };
    let when = tv::due_words(w, now_ic.saturating_add(i64::from(plan.secs)));
    let line = if plan.blind {
        tv::CORE_FILED
    } else {
        tv::MINE_FILED
    }
    .replace("{n}", &n)
    .replace("{addr}", &plan.addr)
    .replace("{when}", &when);
    put_order(
        ctx,
        rig.tv,
        ActOrder::Mine(Box::new(MineOrder {
            actor,
            ordinal,
            site: plan.site.clone(),
            choice: choice.clone(),
            plan,
        })),
    );
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        Vec::new(),
        json!({
            "tv": rig.tv.0.to_string(),
            "act": wire_word(ActKind::Mine, false),
            "ordinal": ordinal,
        }),
        key,
    );
    crate::port::coop::say(ctx, line);
    Ok(())
}

/// **`+tv/<n> mine stop`** (doc 20 §6: "lets the running cycle complete and yield and drops the
/// cycles after it at no cost"; cut SM.7, the brief's 3.4 and decision R7): committed at once, with
/// no `go` — a control on a committed run that only drops future cycles. The record's run: none →
/// `MINE_NOT_RUNNING`; already stopping → `MINE_STOPPING_ALREADY` (nothing queued); else ONE
/// `tacsim_surface_act` with `act: mine_stop` and the run's ordinal, keyed
/// `tacsim.surface/{tv}/{ordinal}/stop`, and `MINE_STOPPING`. Never a refund of the running cycle.
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line.
pub(crate) fn mine_stop_cmd(ctx: &mut Ctx<'_>, rig: &RigAt) -> Result<(), CmdError> {
    let handles = ctx
        .w
        .module_state::<PortHandles>()
        .cloned()
        .ok_or_else(|| CmdError::msg(tv::TV_UNWIRED))?;
    let Some(action) = handles.surface_act.clone() else {
        return Err(CmdError::msg(tv::TV_UNWIRED));
    };
    // Doc 20 cut SM.11 (D8): a live plan holds its rig — its stop too.
    plan_holds(ctx, rig)?;
    let name = name_of(ctx.w, rig.hull);
    let dark = || CmdError::msg(tv::TV_DARK.replace("{name}", &name));
    let battery = rig_battery(ctx.w, &handles, rig);
    let record = record_of(ctx, &handles, rig.tv, battery.as_ref()).map_err(|()| dark())?;
    let n = rig.slot.to_string();
    let Some(run) = record.run else {
        return Err(CmdError::msg(tv::MINE_NOT_RUNNING.replace("{n}", &n)));
    };
    if run.stop {
        return Err(CmdError::msg(tv::MINE_STOPPING_ALREADY.replace("{n}", &n)));
    }
    let Some(key) = stop_key(rig.tv, run.ordinal) else {
        tracing::error!(tv = rig.tv.0, "tacsim: a run's stop key did not form");
        return Err(dark());
    };
    let line = tv::MINE_STOPPING
        .replace("{n}", &n)
        .replace(
            "{addr}",
            &address::pocket_address(run.pocket[0], run.pocket[1]).unwrap_or_default(),
        )
        .replace("{when}", &tv::due_words(ctx.w, run.due_ic));
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        Vec::new(),
        json!({
            "tv": rig.tv.0.to_string(),
            "act": "mine_stop",
            "ordinal": run.ordinal,
        }),
        key,
    );
    crate::port::coop::say(ctx, line);
    Ok(())
}

// ---- doc 20 cut SM.11: the fill plan's verbs ---------------------------------------------------

/// **What a plan's preview and its filing read** (doc 20 cut SM.11; the brief's 3.5): the words
/// the lines name and the figures beside the compile.
#[derive(Clone, Debug)]
pub(crate) struct PlanFacts {
    /// The rig's record (fresh when absent).
    pub record: RigRecord,
    /// The hull's name.
    pub name: String,
    /// The site's entry.
    pub site: CanonicalId,
    /// The site's name.
    pub site_name: String,
    /// The hold's unreserved room, whole tonnes.
    pub room_t: u64,
    /// The slots of the hull's other rigs working a plan live for their lessee.
    pub shared: Vec<u8>,
}

/// **Gather a plan's compile over the world** (doc 20 cut SM.11; the brief's 3.5 — one function,
/// the command and the start's offer both call it, `mine_plan`'s precedent), cheap first: the
/// layer and the view; under way; the hull on a site's pad (`LAUNCH_NOT_SITE`); the actor aboard
/// (`TV_NOT_ABOARD`: filing is a hand act); the record (unreadable → `TV_DARK`); a plan live for
/// the actor (`plan_filed`); the site's ground and home warm (`GROUND_COLD`, `INSTRUMENTS_DARK`);
/// the tools rows and the rig's rates (`TV_DARK`); the hold's room (`TV_DARK`); the hopper; the
/// site's ground resolved; the author's chart row; the rig's cell (the ramp's from the bay) — then
/// [`crate::surface::plan::compile`] over the author's licence, lease, tenure and readings. `Err` is
/// the world's refusal; `Ok` carries the compile's own answer.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)] // one ordered gather
pub(crate) fn plan_ask(
    w: &World,
    reads: &dyn SurfaceReads,
    actor: ObjId,
    rig: &RigAt,
    list: &str,
    family: Option<OreFamily>,
    tools: &BTreeMap<CanonicalId, ToolDef>,
    peeks: &mut Peeks,
    now_ic: i64,
) -> Result<
    (
        Result<crate::surface::plan::Compiled, crate::surface::plan::PlanRefusal>,
        PlanFacts,
    ),
    Refusal,
> {
    let (view, name) = view_of(w, rig)?;
    if matches!(berth_of(w, rig.hull), Berth::UnderWay) {
        return Err(Refusal::UnderWay { name });
    }
    let berth = site_berth(w, &view)?;
    if !crate::port::voyage::aboard(w, rig.hull, actor) {
        return Err(Refusal::NotAboard { name });
    }
    let dark = || Refusal::Dark { name: name.clone() };
    let record = reads
        .rig(rig.tv)
        .map_err(|error| {
            tracing::warn!(tv = rig.tv.0, %error, "tacsim: a rig's excursion record does not read");
            dark()
        })?
        .unwrap_or_else(|| RigRecord::fresh_unread(rig.tv));
    if let Some(plan) = record.live_plan_for(actor) {
        return Err(Refusal::PlanFiled {
            n: rig.slot,
            no: plan.no,
        });
    }
    let cold = || Refusal::GroundCold {
        site: berth.name.clone(),
        entry: berth.entry.clone(),
    };
    match chart::peek_site(w, &berth.entry, peeks) {
        SitePeek::Warm(_) => {}
        SitePeek::Cold => return Err(cold()),
        SitePeek::Dark => return Err(Refusal::InstrumentsDark),
    }
    let ramp_cell = site_cell(berth.ramp.x_cm, berth.ramp.y_cm);
    let field = match crate::surface::peek_home(w, &berth.entry, ramp_cell, peeks) {
        HomePeek::Warm(field) => field,
        HomePeek::Cold => return Err(cold()),
        HomePeek::Dark => return Err(Refusal::InstrumentsDark),
    };
    let battery = rig_battery_id(w, rig.tv)
        .and_then(|id| battery_of(tools, &id).copied())
        .ok_or_else(dark)?;
    let scanner = rig_scanner_id(w, rig.tv)
        .and_then(|id| scanner_of(tools, &id).copied())
        .ok_or_else(dark)?;
    let drill = rig_drill_id(w, rig.tv)
        .and_then(|id| drill_of(tools, &id).copied())
        .ok_or_else(dark)?;
    let rates = rig_rates(w, rig.tv, Some(&battery)).ok_or_else(dark)?;
    let hull_rigs: Vec<ObjId> = tv::live_on_hull(w, rig.hull)
        .iter()
        .map(|lease| lease.tv)
        .collect();
    let read_pool = |endpoint: EndpointRef, good: &CanonicalId| reads.pool(endpoint, good);
    let read_rig = |tv: ObjId| reads.rig(tv);
    let room_kg =
        crate::port::ship::hold_unreserved_kg(w, &view, &hull_rigs, &read_pool, &read_rig)
            .ok_or_else(dark)?;
    let pool_unit = tv::pool_unit(w);
    let (_, hopper_cap_kg) =
        tv::hopper_load_with(w, rig.tv, pool_unit.as_ref(), &read_pool).map_err(|_| dark())?;
    let site = chart::site_context(w, &berth.entry).map_err(|_| Refusal::InstrumentsDark)?;
    let row = reads.chart(actor, &berth.entry).map_err(|_| dark())?;
    let start_cell = match where_is(w, rig.tv) {
        Where::Bay { hull } if hull == rig.hull => ramp_cell,
        Where::Ground {
            domain, x_cm, y_cm, ..
        } if domain == berth.domain => site_cell(x_cm, y_cm),
        Where::Bay { .. } | Where::Ground { .. } | Where::Elsewhere => {
            return Err(Refusal::Elsewhere { n: rig.slot, name });
        }
    };
    let site_def = berth.def.site.as_ref().ok_or_else(dark)?;
    let now = w.world_ic_now();
    let tenure =
        |letter: char| mine_tenure(w, reads, actor, (&berth.entry, site_def), letter, &now);
    let cache = std::cell::RefCell::new(BTreeMap::new());
    let classes = |pocket: (i64, i64)| {
        crate::surface::plan::read_classes(&site, row.as_ref(), pocket, &mut cache.borrow_mut())
    };
    let fields = |field: (i64, i64)| crate::surface::plan::read_field(&site, row.as_ref(), field);
    let home = |stand: (i64, i64)| drive::home_cu_to(&field, stand, &rates);
    let compiled = crate::surface::plan::compile(&crate::surface::plan::CompileInput {
        row: row.as_ref(),
        list,
        family,
        licence: licence_verdict(w, actor, now_ic),
        lease: lease_gate(w, &rig.record),
        tenure: &tenure,
        classes: &classes,
        field_readings: &fields,
        start_cell,
        home: &home,
        battery: &battery,
        scanner: &scanner,
        drill: &drill,
        hold_room_t: room_kg / 1_000,
        hopper_t: hopper_cap_kg / 1_000,
    });
    let mut shared: Vec<u8> = Vec::new();
    for lease in tv::live_on_hull(w, rig.hull) {
        if lease.tv == rig.tv {
            continue;
        }
        if reads
            .rig(lease.tv)
            .ok()
            .flatten()
            .is_some_and(|other| other.live_plan_for(lease.pilot).is_some())
        {
            shared.push(lease.slot);
        }
    }
    shared.sort_unstable();
    Ok((
        compiled,
        PlanFacts {
            record,
            name,
            site: berth.entry.clone(),
            site_name: berth.name.clone(),
            room_t: room_kg / 1_000,
            shared,
        },
    ))
}

/// **A compile's refusal as the player hears it** (the brief's 3.5 texts): the two §5 lines byte
/// for byte behind the address (`PLAN_REFUSED_AT`), the licence through `+licence`'s own line, the
/// lease through the mining lines.
fn plan_refusal_line(
    w: &World,
    actor: ObjId,
    rig: &RigAt,
    name: &str,
    refusal: &crate::surface::plan::PlanRefusal,
) -> String {
    use crate::surface::plan::{MAX_PLAN_ADDRESSES, MAX_PLAN_STEPS, PlanRefusal};
    let n = rig.slot.to_string();
    let at = |addr: &str, line: &str| {
        tv::PLAN_REFUSED_AT
            .replace("{addr}", addr)
            .replace("{line}", line)
    };
    match refusal {
        PlanRefusal::Usage => tv::PLAN_USAGE.to_owned(),
        PlanRefusal::TooMany => tv::PLAN_TOO_MANY.replace("{max}", &MAX_PLAN_ADDRESSES.to_string()),
        PlanRefusal::Twice { addr } => tv::PLAN_TWICE.replace("{addr}", addr),
        PlanRefusal::NoReading { addr } => at(addr, tv::NO_READING),
        PlanRefusal::Barren { addr, family } => tv::PLAN_BARREN
            .replace("{addr}", addr)
            .replace("{family}", family.label()),
        PlanRefusal::Licence { why } => licence_end_line(w, actor, *why),
        PlanRefusal::Lease { bar } => mine_lease_refusal(w, rig, *bar).line(),
        PlanRefusal::Claimed { addr } => at(addr, tv::MINE_CLAIMED),
        PlanRefusal::Dark { addr } => at(addr, tv::PLAN_DARK_LINE),
        PlanRefusal::TooFar { addr } => at(addr, &tv::PLAN_TOO_FAR_LINE.replace("{n}", &n)),
        PlanRefusal::TooLong { hours } => tv::PLAN_TOO_LONG.replace("{hours}", &hours.to_string()),
        PlanRefusal::TooManySteps => {
            tv::PLAN_TOO_MANY_STEPS.replace("{max}", &MAX_PLAN_STEPS.to_string())
        }
        PlanRefusal::HoldFull => tv::PLAN_HOLD_FULL.replace("{name}", name),
    }
}

/// `{hours}` for an estimate: whole IC hours, rounded up.
fn hours_up(secs: u64) -> u64 {
    secs.div_ceil(3_600)
}

/// **`+tv/<n> plan <list> [family=<good>] [go]`** (doc 20 §1 beat 11, §8 "Leaving it running", §10;
/// cut SM.11, the brief's 3.5; lead rulings 1 and 2, decision D10): [`plan_ask`] (the same gather
/// the start's offer runs), then the compile's answer — a refusal's line, or bare the preview
/// (`PLAN_PREVIEW_HEAD`, one `PLAN_PREVIEW_STEP` a step, `PLAN_PREVIEW_TAIL`), nothing written;
/// with `go`, the filing written as an `ActOrders` entry (`ActOrder::Plan`) and ONE
/// `tacsim_surface_act` with `act: plan_file` keyed `tacsim.surface/{tv}/plan/{no}/file`, and
/// `PLAN_FILED`.
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line.
#[allow(clippy::too_many_lines)] // the gather, then the preview or the go
pub(crate) fn plan_cmd(
    ctx: &mut Ctx<'_>,
    rig: &RigAt,
    list: &str,
    family: Option<OreFamily>,
    family_word: Option<&str>,
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
    let tools = handles.surface_tools.clone().ok_or_else(dark)?;
    let now_ic = ctx.w.world_ic_now().at;
    let mut peeks = Peeks::default();
    let asked = plan_ask(
        ctx.w, &*ctx, actor, rig, list, family, &tools, &mut peeks, now_ic,
    );
    let (compiled, facts) = match asked {
        Ok(asked) => asked,
        Err(refusal) => {
            want_cold(ctx, rig, &refusal);
            return Err(CmdError::msg(refusal.line()));
        }
    };
    let w: &World = ctx.w;
    let compiled = compiled
        .map_err(|refusal| CmdError::msg(plan_refusal_line(w, actor, rig, &name, &refusal)))?;
    let n = rig.slot.to_string();
    let ore = tv::ore_words(crate::surface::opening::ore_good(compiled.family));
    let typed_list = {
        let mut words = compiled.list.join(",");
        if let Some(word) = family_word {
            words.push_str(" family=");
            words.push_str(word);
        }
        words
    };
    if !go {
        let real = match crate::port::voyage::real_until(
            w,
            now_ic.saturating_add(crate::surface::plan::PLAN_BOUND_IC_SECS),
        ) {
            Some(real) => format!(", about {}", tv::span_words(real)),
            None => String::new(),
        };
        let mut lines = vec![
            tv::PLAN_PREVIEW_HEAD
                .replace("{n}", &n)
                .replace("{mark}", &rig.mark)
                .replace("{site}", &facts.site_name)
                .replace("{ore}", &ore)
                .replace("{real}", &real),
        ];
        for (k, step) in compiled.steps.iter().enumerate() {
            let secs = compiled.step_secs.get(k).copied().unwrap_or(0);
            let hours = if secs > 0 {
                tv::PLAN_STEP_HOURS.replace("{h}", &hours_up(secs).to_string())
            } else {
                String::new()
            };
            let far = compiled
                .far
                .iter()
                .find(|(at, _)| usize::from(*at) == k)
                .map_or_else(String::new, |(_, count)| {
                    tv::PLAN_PREVIEW_FAR.replace("{k}", &count.to_string())
                });
            lines.push(
                tv::PLAN_PREVIEW_STEP
                    .replace("{k}", &k.saturating_add(1).to_string())
                    .replace("{words}", &crate::surface::plan::step_words(step, true))
                    .replace("{hours}", &hours)
                    .replace("{far}", &far),
            );
        }
        let shared: String = facts
            .shared
            .iter()
            .map(|m| tv::PLAN_SHARED.replace("{m}", &m.to_string()))
            .collect();
        lines.push(
            tv::PLAN_PREVIEW_TAIL
                .replace("{name}", &facts.name)
                .replace("{room}", &facts.room_t.to_string())
                .replace("{shared}", &shared)
                .replace("{hours}", &hours_up(compiled.est_secs).to_string())
                .replace("{n}", &n)
                .replace("{list}", &typed_list),
        );
        for line in lines {
            crate::port::coop::say(ctx, line);
        }
        return Ok(());
    }
    // `go`: the order, then ONE submission as the actor.
    let ordinal = facts.record.ordinal;
    let Some(no) = facts.record.plans.checked_add(1) else {
        return Err(dark());
    };
    let Some(key) = plan_file_key(rig.tv, no) else {
        tracing::error!(tv = rig.tv.0, "tacsim: a plan's filing key did not form");
        return Err(dark());
    };
    let when = tv::due_words(
        ctx.w,
        now_ic.saturating_add(crate::surface::plan::PLAN_BOUND_IC_SECS),
    );
    put_order(
        ctx,
        rig.tv,
        ActOrder::Plan(Box::new(PlanOrder {
            actor,
            ordinal,
            no,
            list: list.to_owned(),
            family,
            compiled,
        })),
    );
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        Vec::new(),
        json!({
            "tv": rig.tv.0.to_string(),
            "act": "plan_file",
            "ordinal": ordinal.max(1),
        }),
        key,
    );
    crate::port::coop::say(
        ctx,
        tv::PLAN_FILED
            .replace("{n}", &n)
            .replace("{no}", &no.to_string())
            .replace("{when}", &when),
    );
    Ok(())
}

/// The words a plan's lines name (`{n}`, `{name}`, `{site}`, `{ramp}`, `{body}`).
pub(crate) struct PlanWords {
    /// The slot.
    pub n: String,
    /// The hull's name.
    pub name: String,
    /// The site's name.
    pub site: String,
    /// The egress's label.
    pub ramp: String,
    /// The site's body's name.
    pub body: String,
}

impl PlanWords {
    /// The words of `plan` on `w`.
    pub(crate) fn of(w: &World, plan: &FillPlan) -> PlanWords {
        let body = chart::lane_sites(w)
            .into_iter()
            .find(|site| site.entry == plan.site)
            .map_or_else(String::new, |site| crate::port::site::body_name(&site.body));
        PlanWords {
            n: plan.slot.to_string(),
            name: name_of(w, plan.hull),
            site: site_name(w, &plan.site),
            ramp: ramp_label(w, plan.hull),
            body,
        }
    }
}

/// **Why a plan parked or ended, in a sentence** (the brief's 3.5 reason words): each ends in a
/// full stop; the licence is `+licence`'s own line.
#[must_use]
pub(crate) fn end_why(w: &World, author: ObjId, why: PlanEnd, words: &PlanWords) -> String {
    match why {
        PlanEnd::WorkedOut => tv::END_WORKED_OUT.to_owned(),
        PlanEnd::HoldFull => tv::END_HOLD_FULL.replace("{name}", &words.name),
        PlanEnd::Bound => tv::END_BOUND.to_owned(),
        PlanEnd::Licence { why } => licence_end_line(w, author, why),
        PlanEnd::Claimed => tv::MINE_CLAIMED.to_owned(),
        PlanEnd::Lease { forfeit: true } => tv::END_LEASE_FORFEIT.to_owned(),
        PlanEnd::Lease { forfeit: false } => tv::END_LEASE_ENDED.to_owned(),
        PlanEnd::Dark => tv::END_DARK.to_owned(),
        PlanEnd::ChartFull => tv::END_CHART_FULL.replace("{body}", &words.body),
        PlanEnd::Cleared => tv::END_CLEARED.to_owned(),
        PlanEnd::Stranded => tv::END_STRANDED.replace("{ramp}", &words.ramp),
    }
}

/// **Why a plan waits, in words** (the brief's 3.5): `Busy` is never told.
#[must_use]
pub(crate) fn wait_why(
    w: &World,
    wait: crate::surface::plan::PlanWait,
    words: &PlanWords,
) -> String {
    use crate::surface::plan::PlanWait;
    match wait {
        PlanWait::Helm => tv::WAIT_HELM.replace("{name}", &words.name),
        PlanWait::Ashore => tv::WAIT_ASHORE.replace("{name}", &words.name),
        PlanWait::Cap => tv::WAIT_CAP.to_owned(),
        PlanWait::Charging { until_ic } => {
            tv::WAIT_CHARGING.replace("{when}", &tv::due_words(w, until_ic))
        }
        PlanWait::Cold => tv::WAIT_COLD.to_owned(),
        PlanWait::Markers => tv::WAIT_MARKERS.to_owned(),
        PlanWait::Dark | PlanWait::Busy => tv::WAIT_DARK.to_owned(),
    }
}

/// **Why a plan passed something over, in words** (the brief's 3.5).
#[must_use]
pub(crate) fn pass_why(why: crate::surface::plan::PassWhy, family: OreFamily) -> String {
    use crate::surface::plan::PassWhy;
    match why {
        PassWhy::WorkedOut => tv::PASS_WORKED_OUT.to_owned(),
        PassWhy::Barren => tv::PASS_BARREN.replace("{family}", family.label()),
        PassWhy::Locked => tv::PASS_LOCKED.to_owned(),
        PassWhy::NoWay => tv::PASS_NO_WAY.to_owned(),
        PassWhy::TooFar => tv::PASS_TOO_FAR.to_owned(),
        PassWhy::NothingNew => tv::PASS_NOTHING_NEW.to_owned(),
    }
}

/// **The excursion caps' census for `operator` at `site`** (doc 20 cut SM.11; the brief's 3.4
/// "the caller gathers"): the world's rig records' open excursions, read through `read_rig` — the
/// launch's own count.
pub(crate) fn excursion_counts(
    w: &World,
    read_rig: &dyn Fn(ObjId) -> Result<Option<RigRecord>, String>,
    operator: ObjId,
    site: &CanonicalId,
) -> ExcursionCounts {
    let mut counts = ExcursionCounts::default();
    for tv in world_rigs(w) {
        if let Ok(Some(record)) = read_rig(tv)
            && let Some(excursion) = record.excursion.as_ref()
        {
            counts.world = counts.world.saturating_add(1);
            if &excursion.site == site {
                counts.site = counts.site.saturating_add(1);
            }
            if excursion.operator == operator {
                counts.operator = counts.operator.saturating_add(1);
            }
        }
    }
    counts
}

/// **What a plan's next move does, in words** (`PLAN_SHOW_NOW`): the act's doing words, the wait's
/// reason, or the end's.
fn plan_now_words(
    w: &World,
    rig: &RigAt,
    plan: &FillPlan,
    words: &PlanWords,
    moved: Option<crate::surface::plan::PlanMove>,
) -> String {
    use crate::surface::plan::{PlanActInner, PlanMove};
    match moved {
        Some(PlanMove::Start { word, act, .. }) => match act.0 {
            PlanActInner::Act(planned) | PlanActInner::Drive(planned) => {
                let kind = ActKind::from_word(word).unwrap_or(ActKind::Recover);
                doing_words(kind, &planned.params, &ramp_label(w, rig.hull))
            }
            PlanActInner::Mine(planned) => drilling_words(&planned.addr),
        },
        Some(PlanMove::End { why, .. }) => {
            format!("it ends: {}", end_why(w, plan.author, why, words))
                .trim_end_matches('.')
                .to_owned()
        }
        Some(PlanMove::Wait(wait)) => match wait {
            crate::surface::plan::PlanWait::Busy => tv::WAIT_BUSY.to_owned(),
            other => wait_why(w, other, words),
        },
        None => tv::WAIT_DARK.to_owned(),
    }
}

/// **`+tv/<n> plan show`** (doc 20 cut SM.11; the brief's 3.5): a pure read of the record — no plan
/// → `PLAN_NONE`; else `PLAN_SHOW_HEAD`, one `PLAN_SHOW_STEP` a step (done, now, next, pending,
/// passed over), the tally, and for a live plan `PLAN_SHOW_NOW` from the interpreter over the
/// command's own reads; an ended plan `PLAN_SHOW_ENDED`. Never a class, a tonnage of the ground or
/// a reserve.
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line.
pub(crate) fn plan_show_cmd(ctx: &mut Ctx<'_>, rig: &RigAt) -> Result<(), CmdError> {
    let handles = ctx
        .w
        .module_state::<PortHandles>()
        .cloned()
        .ok_or_else(|| CmdError::msg(tv::TV_UNWIRED))?;
    let name = name_of(ctx.w, rig.hull);
    let dark = || CmdError::msg(tv::TV_DARK.replace("{name}", &name));
    let battery = rig_battery(ctx.w, &handles, rig);
    let record = record_of(ctx, &handles, rig.tv, battery.as_ref()).map_err(|()| dark())?;
    let n = rig.slot.to_string();
    // Only the asker's own plan (the fixer round's conformance/F1): a previous lessee's plan left
    // on the record reads as none — its pockets, family and tally are theirs.
    let Some(plan) = record.plan.as_ref().filter(|plan| plan.author == ctx.actor) else {
        return Err(CmdError::msg(tv::PLAN_NONE.replace("{n}", &n)));
    };
    let w: &World = ctx.w;
    let words = PlanWords::of(w, plan);
    let ore = tv::ore_words(crate::surface::opening::ore_good(plan.family));
    let state = match plan.state {
        FillState::Live => tv::STATE_LIVE,
        FillState::Parking { .. } => tv::STATE_PARKING,
        FillState::Ended { .. } => tv::STATE_ENDED,
    };
    let mut lines = vec![
        tv::PLAN_SHOW_HEAD
            .replace("{n}", &n)
            .replace("{no}", &plan.no.to_string())
            .replace("{site}", &words.site)
            .replace("{ore}", &ore)
            .replace("{state}", state)
            .replace(
                "{filed}",
                &format!(
                    "IC {}",
                    mush_core::CivilDate::from_ic_seconds(plan.filed_ic)
                ),
            )
            .replace("{deadline}", &tv::due_words(w, plan.deadline_ic)),
    ];
    let passed: std::collections::BTreeSet<(i64, i64, bool)> = plan
        .notes
        .iter()
        .filter_map(|note| match note.note {
            PlanNoteKind::Passed { at, field, .. } => Some((at[0], at[1], field)),
            PlanNoteKind::Parking { .. } | PlanNoteKind::Ended { .. } => None,
        })
        .collect();
    let cursor = usize::from(plan.cursor);
    for (k, step) in plan.steps.iter().enumerate() {
        let skipped = match step {
            PlanStep::Mine { pocket } => passed.contains(&(pocket[0], pocket[1], false)),
            PlanStep::Sound { field } => address::field_of_index(*field)
                .is_some_and(|(fx, fy)| passed.contains(&(fx, fy, true))),
            PlanStep::Block { .. } => false,
        };
        let mark = if k < cursor && skipped {
            tv::MARK_PASSED
        } else if k < cursor {
            tv::MARK_DONE
        } else if k == cursor && matches!(plan.state, FillState::Live) {
            tv::MARK_NOW
        } else if k == cursor.saturating_add(1) && matches!(plan.state, FillState::Live) {
            // A parking plan works no further step (the fixer round's correctness/F4).
            tv::MARK_NEXT
        } else {
            tv::MARK_PENDING
        };
        lines.push(
            tv::PLAN_SHOW_STEP
                .replace("{k}", &k.saturating_add(1).to_string())
                .replace("{words}", &crate::surface::plan::step_words(step, false))
                .replace("{mark}", mark),
        );
    }
    lines.push(
        tv::PLAN_SHOW_TALLY
            .replace("{t}", &plan.tally.tonnes.to_string())
            .replace("{ore}", &ore)
            .replace(
                "{c}",
                &tv::count_words(u64::from(plan.tally.cycles), "cycle", "cycles"),
            )
            .replace(
                "{trips}",
                &tv::count_words(u64::from(plan.tally.trips), "trip", "trips"),
            ),
    );
    match plan.state {
        FillState::Ended { why, at_ic } => lines.push(
            tv::PLAN_SHOW_ENDED
                .replace(
                    "{at}",
                    &format!("IC {}", mush_core::CivilDate::from_ic_seconds(at_ic)),
                )
                .replace(
                    "{why}",
                    end_why(w, plan.author, why, &words).trim_end_matches('.'),
                ),
        ),
        FillState::Live | FillState::Parking { .. } => {
            let tools = handles.surface_tools.clone().unwrap_or_default();
            let read_rig = |tv: ObjId| SurfaceReads::rig(&*ctx, tv);
            let counts = excursion_counts(w, &read_rig, plan.author, &plan.site);
            let mut peeks = Peeks::default();
            let moved = crate::surface::plan::decide(
                w,
                &*ctx,
                rig,
                &record,
                &counts,
                &tools,
                &mut peeks,
                w.world_ic_now().at,
            );
            lines.push(
                tv::PLAN_SHOW_NOW.replace("{doing}", &plan_now_words(w, rig, plan, &words, moved)),
            );
        }
    }
    for line in lines {
        crate::port::coop::say(ctx, line);
    }
    Ok(())
}

/// **`+tv/<n> plan clear`** (doc 20 cut SM.11; the brief's 3.5, D6 and D17): committed with no
/// `go` (`+tv/mine stop`'s R7 precedent). No plan → `PLAN_NONE`, nothing submitted; a live plan →
/// ONE `tacsim_surface_act` with `act: plan_clear` keyed `tacsim.surface/{tv}/plan/{no}/clear/
/// {seq}`, `ordinal` [`PLAN_CLEAR_ORDINAL`], and `PLAN_CLEARED` (the step in flight finishes by
/// its own rules); an ended plan → ONE with `act: plan_strike` keyed `tacsim.surface/{tv}/plan/
/// {no}/strike/{ordinal}` and `PLAN_DROPPED`. Two writes on two keys, so a second clear is never a silent replay of the first.
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line.
pub(crate) fn plan_clear_cmd(ctx: &mut Ctx<'_>, rig: &RigAt) -> Result<(), CmdError> {
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
    let battery = rig_battery(ctx.w, &handles, rig);
    let record = record_of(ctx, &handles, rig.tv, battery.as_ref()).map_err(|()| dark())?;
    let n = rig.slot.to_string();
    // Only the asker's own plan (the fixer round's conformance/F1): a previous lessee's plan is
    // inert, and the asker's own next filing replaces it.
    let Some(plan) = record.plan.as_ref().filter(|plan| plan.author == ctx.actor) else {
        return Err(CmdError::msg(tv::PLAN_NONE.replace("{n}", &n)));
    };
    let no = plan.no;
    // A clear carries `PLAN_CLEAR_ORDINAL` (the lead's pre-commit round, item 2: its key names the
    // plan's `seq`, never the record's ordinal, so neither do its parameters); a strike, keyed by
    // the ordinal, carries it.
    let (word, key, line, ordinal) = if plan.live() {
        (
            "plan_clear",
            plan_clear_key(rig.tv, no, plan.seq),
            tv::PLAN_CLEARED,
            PLAN_CLEAR_ORDINAL,
        )
    } else {
        (
            "plan_strike",
            plan_strike_key(rig.tv, no, record.ordinal),
            tv::PLAN_DROPPED,
            record.ordinal.max(1),
        )
    };
    let Some(key) = key else {
        tracing::error!(tv = rig.tv.0, "tacsim: a plan's clear key did not form");
        return Err(dark());
    };
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        Vec::new(),
        json!({
            "tv": rig.tv.0.to_string(),
            "act": word,
            "ordinal": ordinal,
            "plan": no,
        }),
        key,
    );
    crate::port::coop::say(
        ctx,
        line.replace("{n}", &n).replace("{no}", &no.to_string()),
    );
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
    /// **Doc 20 cut SM.4a**: a scan's or a sounding's body's site entries (at most
    /// `MAX_CHART_SITES_PER_BODY`), whose chart rows the prepare reads for the body caps.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    body_sites: Vec<CanonicalId>,
    /// **Doc 20 cut SM.4b** (the brief's decision P2): a drive's order's ordinal, which the
    /// prepare binds to the invocation's (`Conflict` otherwise). Doc 20 cut SM.7: a run's too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ordinal: Option<u32>,
    /// **Doc 20 cut SM.7**: a run's server facts (truth among them — fixed parameters are
    /// server-only, never projected to a player).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mine: Option<MineFixed>,
    /// **Doc 20 cut SM.8**: an unload's two possible sources, fixed (the offer cannot see the
    /// start's `how`; the prepare picks by it — the brief's decision R15).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    unload: Option<UnloadFixed>,
    /// **Doc 20 cut SM.11**: a plan step's binding — the plan, its `seq`, the step and the word
    /// the interpreter chose, the update its start writes — fixed under the rig's `plan` key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    plan: Option<PlanFixed>,
    /// **Doc 20 cut SM.11**: a filing's compiled plan, fixed under the rig's `plan_file` key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fill: Option<FillFixed>,
}

/// **A plan step as the start's offer fixes it** (doc 20 cut SM.11; the brief's 3.6): the
/// interpreter's choice over the durable record, which the prepare holds the start's `plan`,
/// `step` and `act` to (`Conflict` "the plan moved on" otherwise). Server-only fixed parameters,
/// no truth: the act's own plan rides `FixedPlan` as the hand's does.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanFixed {
    no: u32,
    seq: u32,
    step: u16,
    word: String,
    update: PlanUpdate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    end: Option<PlanEnd>,
}

/// **A filing as the start's offer fixes it** (doc 20 cut SM.11; the brief's 3.6): the order's
/// number and ordinal and its re-compiled list, family and steps — required equal to the preview.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FillFixed {
    no: u32,
    ordinal: u32,
    list: Vec<String>,
    family: OreFamily,
    steps: Vec<PlanStep>,
}

/// **An unload's server facts, as the start's offer fixes them** (doc 20 cut SM.8; the brief's
/// 3.7, decisions R4, R15 and R16): the vaults, both variants the prepare may run, the hull's
/// hold plan (so the prepare weighs with the one table and compartment the preview did, with no
/// World), the goods the hold's ore compartment accepts (table order) and the hull's other rigs
/// (their hoppers and runs are the reservation the prepare re-reads in its transaction).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UnloadFixed {
    hopper: u64,
    hold: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hand: Option<HandFixed>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    queued: Option<QueuedFixed>,
    plan: crate::port::market::HoldPlan,
    goods_of_hold: Vec<CanonicalId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    siblings: Vec<Sibling>,
}

/// **The hand variant**: the operator's `+tv/unload go` order for the rig's next ordinal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HandFixed {
    ordinal: u32,
    how: UnloadHow,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    qty: Option<u64>,
    goods: BTreeMap<CanonicalId, u64>,
}

/// **The queued variant**: the recover's queued unload, split against the hold's room now
/// (possibly moving nothing).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct QueuedFixed {
    ordinal: u32,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    goods: BTreeMap<CanonicalId, u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    excursion: Option<ExcursionId>,
}

/// One of the hull's other rigs, as the unload's re-check reads it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Sibling {
    tv: ObjId,
    hopper: u64,
    goods: Vec<CanonicalId>,
}

/// **A run's server facts, as the start's offer fixes them** (doc 20 cut SM.7; the brief's 3.4):
/// everything the prepare writes into the run (so the finish needs no geology and no site read)
/// and checks against the transaction. Serialises because fixed parameters must; they are
/// server-only (`InternalOffer` is not serializable to a player), the cursor's precedent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MineFixed {
    body: CanonicalId,
    stand: [i64; 2],
    pocket: [i64; 2],
    number: u16,
    letter: char,
    family: OreFamily,
    good: CanonicalId,
    deposit: mush_industry::OreDepositId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<u64>,
    hopper: u64,
    cycles: u8,
    yields: Vec<u32>,
    reserve_t: Vec<u8>,
    class_t: u8,
    pocket_t: u8,
    blind: bool,
    secs: u32,
    cost_cu: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    lock_rows: Vec<LockRow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lapses_ic: Option<i64>,
    drawn: u32,
    outstanding: Vec<(ObjId, Option<(u32, u64)>)>,
}

impl MineFixed {
    fn of(plan: &MinePlan) -> MineFixed {
        MineFixed {
            body: plan.body.clone(),
            stand: plan.stand,
            pocket: plan.pocket,
            number: plan.number,
            letter: plan.letter,
            family: plan.family,
            good: plan.good.clone(),
            deposit: plan.deposit,
            source: plan.source,
            hopper: plan.hopper,
            cycles: plan.cycles,
            yields: plan.yields.clone(),
            reserve_t: plan.reserve_t.clone(),
            class_t: plan.class_t,
            pocket_t: plan.pocket_t,
            blind: plan.blind,
            secs: plan.secs,
            cost_cu: plan.cost_cu,
            lock_rows: plan.lock_rows.clone(),
            lapses_ic: plan.lapses_ic,
            drawn: plan.drawn,
            outstanding: plan.outstanding.clone(),
        }
    }
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
                ordinal: None,
                mine: None,
                unload: None,
                plan: None,
                fill: None,
            },
            Err(refusal) => FixedPlan::refused(refusal.word()),
        }
    }
}

/// **The start's fixed plan of a drive** (doc 20 cut SM.4b; the brief's decision P2): the rig's
/// [`DriveOrder`] — none for this rig and actor → `unplanned` — re-checked against the world:
/// the same stand gate a scan runs (`stand_of`: from the bay the launch's whole order), and the
/// rig's stand must be where the order's leg begins on the order's site (the ramp's cell from the
/// bay), else `moved`. Then the order's plan with its ordinal. The offer reads no composite for a
/// drive.
fn drive_fixed(w: &World, actor: ObjId, rig: &RigAt) -> FixedPlan {
    let Some(order) = order_of(w, rig.tv).filter(|order| order.actor == actor) else {
        return FixedPlan::refused(Refusal::Unplanned.word());
    };
    let stand = match stand_of(w, actor, rig) {
        Ok(stand) => stand,
        Err(refusal) => return FixedPlan::refused(refusal.word()),
    };
    let start = order
        .params
        .leg()
        .and_then(|(waypoints, _)| waypoints.first())
        .map(|first| site_cell(first.x_cm, first.y_cm));
    if stand.site != order.site || stand.from_bay != order.from_bay || start != Some(stand.cell) {
        return FixedPlan::refused(Refusal::Moved.word());
    }
    FixedPlan {
        ok: true,
        code: None,
        site: Some(order.site),
        secs: order.secs,
        cost_cu: order.cost_cu,
        home_cu: order.home_cu,
        params: Some(order.params),
        from_bay: order.from_bay,
        goods: Vec::new(),
        body_sites: Vec::new(),
        ordinal: Some(order.ordinal),
        mine: None,
        unload: None,
        plan: None,
        fill: None,
    }
}

/// **The start's fixed plan of a run** (doc 20 cut SM.7; the brief's 3.4, decision R10): the
/// rig's [`MineOrder`] — none for this rig and actor → `unplanned`; nor one whose ordinal is not
/// the rig's next (`next_ordinal`, read from its record by the offer: the order's start landed, or
/// another act overtook it), dropped quietly **without a re-plan** (the SM.7 lead's pre-commit
/// round, item 5; the tick's `prune_mine_orders` then clears it) — then [`mine_plan`] re-run
/// with the order's choice over the engine's own snapshot (the offer's peeks) and **required equal
/// to the order's plan**, else `mine_changed`: `go` commits exactly what was previewed or nothing.
/// The run's charge and its trip home ride the plan's figures; its server facts ride
/// [`MineFixed`]; a blind core names the body's site entries for the prepare's body caps.
#[allow(clippy::too_many_arguments)]
fn mine_fixed(
    engine: &mush_core::Engine,
    actor: ObjId,
    rig: &RigAt,
    drill: Option<&DrillDef>,
    has_battery: bool,
    rates: Option<&DriveRates>,
    peeks: &mut Peeks,
    next_ordinal: Option<u32>,
) -> FixedPlan {
    let w: &World = &engine.world;
    let Some(order) = mine_order_of(w, rig.tv)
        .filter(|order| order.actor == actor)
        .filter(|order| next_ordinal.is_none_or(|next| next == order.ordinal))
    else {
        return FixedPlan::refused(Refusal::Unplanned.word());
    };
    let (Some(drill), true) = (drill, has_battery) else {
        return FixedPlan::refused("tv_dark");
    };
    let reads = crate::port::site_map::EngineReads(engine);
    let plan = match mine_plan(w, &reads, actor, rig, &order.choice, drill, rates, peeks) {
        Ok(plan) => plan,
        Err(refusal) => return FixedPlan::refused(refusal.word()),
    };
    if plan != order.plan {
        return FixedPlan::refused(Refusal::MineChanged.word());
    }
    let (Some(cost_cu), Some(secs)) = (plan.run_cost_cu(), plan.run_secs()) else {
        return FixedPlan::refused("tv_dark");
    };
    FixedPlan {
        ok: true,
        code: None,
        site: Some(plan.site.clone()),
        secs,
        cost_cu,
        home_cu: plan.home_cu,
        params: None,
        from_bay: false,
        goods: Vec::new(),
        body_sites: if plan.blind {
            chart::body_sites(w, &plan.body)
        } else {
            Vec::new()
        },
        ordinal: Some(order.ordinal),
        mine: Some(MineFixed::of(&plan)),
        unload: None,
        plan: None,
        fill: None,
    }
}

/// **The start's fixed plan of an unload** (doc 20 cut SM.8; the brief's 3.7, decisions R4, R6,
/// R15): both possible sources, since the legal-action query carries no parameters and the
/// prepare picks by the start's `how`:
///
/// - **the hand variant** — the actor's [`UnloadOrder`] for this rig **planned for the rig's next
///   ordinal** (an order left from an earlier, landed unload is stale and ignored: RAM orders are
///   never removed on landing), [`unload_plan`] re-run (`queued` false);
/// - **the queued variant** — the record's [`Queued`] for this actor, [`unload_plan`] re-run
///   (`queued` true: no aboard gate) and the hopper's available goods split against the hold's
///   room now ([`unload_room`], `crate::surface::unload::whole`) — possibly moving nothing — and,
///   doc 20 cut SM.11 (the lead's pre-commit round, item 1), **the hopper's marker ring able to
///   record the split's moves at the offer's real second** ([`ring_admits`]): while it
///   cannot, the variant is refused `unload_markers`, nothing lands and the queue stands. The
///   surface tick reads the same rule first ([`queued_ring_full`], the reviewer's R1): while the
///   ring is full it submits nothing, tells `WHY_MARKERS_QUEUED` once and re-reads the ring on the
///   queue's back-off (every 60 fires), so the unload, and a plan waiting on it, goes on by itself
///   once the ring frees; this refusal is the backstop for a ring that filled in between.
///
/// `ok` when either variant planned; otherwise refused with the hand variant's word, else the
/// queued's (`unload_markers` for a full ring), else `unplanned`. The plan names the site,
/// `UNLOAD_S`, and the server facts ([`UnloadFixed`]); its `params` are `None` — the prepare builds
/// the act's parameters from the chosen variant.
fn unload_fixed(engine: &mush_core::Engine, actor: ObjId, rig: &RigAt) -> FixedPlan {
    let w: &World = &engine.world;
    let reads = crate::port::site_map::EngineReads(engine);
    let Ok(record) = reads.rig(rig.tv) else {
        return FixedPlan::refused("tv_dark");
    };
    let Some(next) = record
        .as_ref()
        .map_or(Some(1), |record| record.ordinal.checked_add(1))
    else {
        return FixedPlan::refused("tv_dark");
    };
    let order =
        unload_order_of(w, rig.tv).filter(|order| order.actor == actor && order.ordinal == next);
    let queued = record
        .as_ref()
        .and_then(|record| record.queued.clone())
        .filter(|queued| queued.operator == actor && queued.kind == ActKind::Unload);
    if order.is_none() && queued.is_none() {
        return FixedPlan::refused(Refusal::Unplanned.word());
    }
    let hand_plan = order.as_ref().map(|_| unload_plan(w, actor, rig, false));
    let queued_plan = queued.as_ref().map(|_| unload_plan(w, actor, rig, true));
    let base = match (&hand_plan, &queued_plan) {
        (Some(Ok(plan)), _) | (_, Some(Ok(plan))) => plan.clone(),
        (Some(Err(refusal)), _) | (None, Some(Err(refusal))) => {
            return FixedPlan::refused(refusal.word());
        }
        (None, None) => return FixedPlan::refused(Refusal::Unplanned.word()),
    };
    let ActParams::Unload { hopper, hold, .. } = &base.params else {
        return FixedPlan::refused("tv_dark");
    };
    let (hopper, hold) = (*hopper, *hold);
    let Some(view) = hull_view(w, rig.hull) else {
        return FixedPlan::refused("tv_dark");
    };
    let read_pool = |endpoint: EndpointRef, good: &CanonicalId| reads.pool(endpoint, good);
    let read_rig = |tv: ObjId| reads.rig(tv);
    let Some(room) = unload_room(w, &view, rig.tv, &read_pool, &read_rig) else {
        return FixedPlan::refused("tv_dark");
    };
    let mut siblings = Vec::with_capacity(room.siblings.len());
    for other in &room.siblings {
        let (Some(vault), Some((goods, _))) = (
            crate::port::vehicle::hopper_of(w, *other),
            tv::hopper_goods_of(w, *other),
        ) else {
            return FixedPlan::refused("tv_dark");
        };
        siblings.push(Sibling {
            tv: *other,
            hopper: vault.0,
            goods,
        });
    }
    let goods_of_hold: Vec<CanonicalId> = room
        .goods
        .goods
        .iter()
        .filter(|good| room.compartment.accepts.contains(&good.stowage))
        .map(|good| good.id.clone())
        .collect();
    let hand = match (order, &hand_plan) {
        (Some(order), Some(Ok(_))) => Some(HandFixed {
            ordinal: next,
            how: order.how,
            qty: order.qty,
            goods: order.goods,
        }),
        _ => None,
    };
    let mut markers_full = false;
    let queued = match (queued, &queued_plan) {
        (Some(queued), Some(Ok(_))) => {
            // Doc 20 cut SM.11 (the lead's pre-commit round, item 1): a ring that cannot record
            // the moves refuses the queued start before it lands (its finish would only record
            // `MarkersFull` again); a split that moves nothing writes no marker. The surface tick
            // reads the same two functions first (`queued_ring_full`, the reviewer's R1) and does
            // not submit into a full ring, so this refusal is the backstop for a ring that filled
            // between the tick's read and this offer.
            match queued_split(w, &reads, rig.tv, &room) {
                Some(split) if ring_admits(&reads, hopper, split.moves.len(), w.now()) => {
                    Some(QueuedFixed {
                        ordinal: next,
                        goods: split.moves,
                        excursion: queued.excursion,
                    })
                }
                Some(_) => {
                    markers_full = true;
                    None
                }
                None => None,
            }
        }
        _ => None,
    };
    if hand.is_none() && queued.is_none() {
        let word = match (&hand_plan, &queued_plan) {
            (Some(Err(refusal)), _) | (None, Some(Err(refusal))) => refusal.word(),
            _ if markers_full => Refusal::UnloadMarkers { n: rig.slot }.word(),
            _ => "tv_dark",
        };
        return FixedPlan::refused(word);
    }
    FixedPlan {
        ok: true,
        code: None,
        site: Some(base.site),
        secs: UNLOAD_S,
        cost_cu: 0,
        home_cu: 0,
        params: None,
        from_bay: false,
        goods: Vec::new(),
        body_sites: Vec::new(),
        ordinal: Some(next),
        mine: None,
        unload: Some(UnloadFixed {
            hopper,
            hold,
            hand,
            queued,
            plan: room.plan,
            goods_of_hold,
            siblings,
        }),
        plan: None,
        fill: None,
    }
}

/// **The queued unload's split** (doc 20 cut SM.8's whole split; cut SM.11, the reviewer's R1):
/// the hopper's available goods (`SurfaceReads::available`) split against `room` —
/// `crate::surface::unload::whole` — exactly as the queued start's offer moves them. `None` when
/// the hopper or the split will not read. One function for the offer, the surface tick and the
/// plan's `Markers` wait, so the three never disagree on how many markers the start would write.
pub(crate) fn queued_split(
    w: &World,
    reads: &dyn SurfaceReads,
    tv: ObjId,
    room: &UnloadRoom,
) -> Option<crate::surface::unload::Split> {
    let pool_unit = tv::pool_unit(w);
    let available = |endpoint: EndpointRef, good: &CanonicalId| reads.available(endpoint, good);
    tv::hopper_contents_with(w, tv, pool_unit.as_ref(), &available)
        .ok()
        .and_then(|contents| unload_split(room, &contents, None).ok())
        .and_then(Result::ok)
}

/// **Whether the hopper's marker ring can record `moves` more markers at `now_wall`** (doc 20 cut
/// SM.11, the lead's pre-commit round, item 1): the ring read through `reads`
/// ([`SurfaceReads::marker_ring`]) and `mush_inventory::MarkerRing::admits` at the real second
/// (`World::now`, the `wall` the finish's offer stamps). Nothing to move needs no marker; an
/// absent ring is empty; a ring or a roster that will not read admits (the finish's own
/// `MarkerRingFull` refusal stays the authority, so a dark read never wedges a queue).
pub(crate) fn ring_admits(
    reads: &dyn SurfaceReads,
    hopper: u64,
    moves: usize,
    now_wall: i64,
) -> bool {
    if moves == 0 {
        return true;
    }
    match reads.marker_ring(EndpointRef(VaultId(hopper))) {
        Ok(Some(ring)) => ring.admits(moves, now_wall),
        Ok(None) | Err(_) => true,
    }
}

/// **Whether a queued unload waits on its hopper's full marker ring** (doc 20 cut SM.11, the
/// reviewer's R1): `queued`'s rig's split ([`queued_split`] against [`unload_room`] for the queue's
/// hull) moves goods the ring cannot record at `now_wall` ([`ring_admits`]) — the very test the
/// queued start's offer refuses `unload_markers` by. The surface tick reads it before it submits
/// the queued start (and holds, telling `WHY_MARKERS_QUEUED` once, rather than submit a start
/// known to fail); the plan's interpreter reads it for its `Markers` wait. `false` whenever any
/// read is dark (the offer and the finish stay the authority).
pub(crate) fn queued_ring_full(
    w: &World,
    reads: &dyn SurfaceReads,
    tv: ObjId,
    queued: &Queued,
    now_wall: i64,
) -> bool {
    if queued.kind != ActKind::Unload {
        return false;
    }
    let Some(view) = hull_view(w, queued.hull) else {
        return false;
    };
    let read_pool = |endpoint: EndpointRef, good: &CanonicalId| reads.pool(endpoint, good);
    let read_rig = |rig: ObjId| reads.rig(rig);
    let Some(room) = unload_room(w, &view, tv, &read_pool, &read_rig) else {
        return false;
    };
    let (Some(split), Some(hopper)) = (
        queued_split(w, reads, tv, &room),
        crate::port::vehicle::hopper_of(w, tv),
    ) else {
        return false;
    };
    !ring_admits(reads, hopper.0, split.moves.len(), now_wall)
}

/// **A plan step's act as the start fixes it** (doc 20 cut SM.11; the brief's 3.6): the plan
/// variant built exactly as the hand path builds that act's — a sounding's or a recover's plan
/// (the body's sites for a reading's caps), a drive's leg with the record's next ordinal (the
/// drive's order shape), a run's [`MineFixed`] — plus its [`PlanFixed`] binding.
fn plan_fixed(
    w: &World,
    plan: &FillPlan,
    next: u32,
    moved: crate::surface::plan::PlanMove,
    has_battery: bool,
) -> FixedPlan {
    use crate::surface::plan::{PlanActInner, PlanMove};
    match moved {
        PlanMove::Start {
            word,
            step,
            update,
            act,
        } => {
            let binding = PlanFixed {
                no: plan.no,
                seq: plan.seq,
                step,
                word: word.to_owned(),
                update,
                end: None,
            };
            let mut fixed = match act.0 {
                PlanActInner::Act(planned) => {
                    if word == "sound" {
                        reading_fixed(w, Some(Ok(planned)), has_battery)
                    } else {
                        FixedPlan::of(Ok(planned))
                    }
                }
                PlanActInner::Drive(planned) => {
                    if !has_battery {
                        return FixedPlan::refused("tv_dark");
                    }
                    let mut fixed = FixedPlan::of(Ok(planned));
                    fixed.ordinal = Some(next);
                    fixed
                }
                PlanActInner::Mine(planned) => {
                    let (Some(cost_cu), Some(secs), true) =
                        (planned.run_cost_cu(), planned.run_secs(), has_battery)
                    else {
                        return FixedPlan::refused("tv_dark");
                    };
                    FixedPlan {
                        ok: true,
                        code: None,
                        site: Some(planned.site.clone()),
                        secs,
                        cost_cu,
                        home_cu: planned.home_cu,
                        params: None,
                        from_bay: false,
                        goods: Vec::new(),
                        body_sites: if planned.blind {
                            chart::body_sites(w, &planned.body)
                        } else {
                            Vec::new()
                        },
                        ordinal: Some(next),
                        mine: Some(MineFixed::of(&planned)),
                        unload: None,
                        plan: None,
                        fill: None,
                    }
                }
            };
            if fixed.ok {
                fixed.plan = Some(binding);
            }
            fixed
        }
        PlanMove::End { why, update } => FixedPlan {
            ok: true,
            plan: Some(PlanFixed {
                no: plan.no,
                seq: plan.seq,
                step: plan.cursor,
                word: "plan_end".to_owned(),
                update,
                end: Some(why),
            }),
            ..FixedPlan::default()
        },
        PlanMove::Wait(_) => FixedPlan::refused("plan_waits"),
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
    /// own ordinal counts, never a stale attempt of an earlier act of the rig. **Doc 20 cut SM.7**:
    /// `(ordinal, cycle)` pairs — cycle 0 an act's, `k` a run's cycle — so a cycle whose transfer
    /// refused mid-bundle records `Unworked` on its next finish.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    failed: Vec<(u32, u8)>,
    /// **The rights of the run's next cycle, read at landing** (doc 20 cut SM.7; §8 "rights are per
    /// act"; the brief's 3.7): `Some` for a rig whose record holds a run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rights: Option<RightsVerdict>,
    /// **The rig's hopper at landing** (doc 20 cut SM.8): its vault and the goods it accepts — a
    /// `Done` recover reads them to queue the unload; the unload reads them to stay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hopper: Option<HopperFacts>,
    /// **The hold of the rig's lease record's hull at landing** (doc 20 cut SM.8): its vault and
    /// its plan — the unload's finish re-checks the hold against it with no World.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hold: Option<HoldFacts>,
}

/// A rig's hopper, as the finish's offer fixes it (doc 20 cut SM.8).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HopperFacts {
    vault: u64,
    goods: Vec<CanonicalId>,
}

/// A hull's hold, as the finish's offer fixes it (doc 20 cut SM.8).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HoldFacts {
    vault: u64,
    plan: crate::port::market::HoldPlan,
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

/// The `act` parameter through [`start_word`] (D23; doc 20 cut SM.7): an act's kind and the
/// sounding's quiet flag, or `mine_stop`.
fn act_param(parameters: &Map<String, Value>) -> Result<StartWord, ActionError> {
    parameters
        .get("act")
        .and_then(Value::as_str)
        .and_then(start_word)
        .ok_or_else(|| {
            invalid(
                "a surface act names launch, recover, dump, scan, sound, sound_quiet, drive, mine, \
                 mine_stop, unload, plan_file, plan_clear, plan_strike or plan_end",
            )
        })
}

/// **The finish's `cycle` parameter** (doc 20 cut SM.7): absent for an act's finish, else a run's
/// cycle `1..=`[`crate::surface::excursion::MAX_RUN_CYCLES`].
fn cycle_param(parameters: &Map<String, Value>) -> Result<Option<u8>, ActionError> {
    match parameters.get("cycle") {
        None => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|k| u8::try_from(k).ok())
            .filter(|k| (1..=crate::surface::excursion::MAX_RUN_CYCLES).contains(k))
            .map(Some)
            .ok_or_else(|| invalid("a run's finish names its cycle, 1 to 8")),
    }
}

/// **A drive start's `to` parameter** (the review's correctness/F5): the planned end cell's
/// word ([`order_to_word`]), `None` when absent (every other act's start, and a forged drive
/// start, which the prepare then refuses).
fn to_word_of(parameters: &Map<String, Value>) -> Option<u32> {
    parameters
        .get("to")
        .and_then(Value::as_u64)
        .and_then(|word| u32::try_from(word).ok())
}

/// **An unload start's `how` parameter** (doc 20 cut SM.8; decision R15): `None` when absent or
/// not one of the three words.
fn how_of(parameters: &Map<String, Value>) -> Option<UnloadHow> {
    parameters
        .get("how")
        .and_then(Value::as_str)
        .and_then(UnloadHow::from_word)
}

/// **A part unload's `qty` parameter** (doc 20 cut SM.8): whole tonnes `1..=99`, `None` when
/// absent or out of range.
fn qty_of(parameters: &Map<String, Value>) -> Option<u64> {
    parameters
        .get("qty")
        .and_then(Value::as_u64)
        .filter(|qty| (1..=tv::MAX_TYPED_TONNES).contains(qty))
}

/// The fixed `now`.
fn now_of(parameters: &Map<String, Value>) -> Result<i64, ActionError> {
    parameters
        .get("now")
        .and_then(Value::as_i64)
        .ok_or_else(|| invalid("a surface act's offer fixes the instant"))
}

/// **The start's `by` word** (doc 20 cut SM.11; the brief's 3.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ByWord {
    /// A fill plan's step or end.
    Plan,
    /// The idle return.
    Idle,
}

/// **The start's `by`, `plan` and `step` parameters**, read and held to the parameter rule.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ByParams {
    /// `by`.
    pub by: Option<ByWord>,
    /// `plan`: the plan's number.
    pub plan: Option<u32>,
    /// `step`: the step.
    pub step: Option<u16>,
}

/// **The parameter rule** (doc 20 cut SM.11; the brief's 3.6): `by: plan` requires `plan` and
/// `step` and is legal only on `drive`, `scan`, `sound`, `mine`, `recover` and `plan_end`; `by:
/// idle` only on `recover`, carrying neither; `plan_file` carries none of the three; `plan_clear`
/// and `plan_strike` carry `plan` only; any other combination is `InvalidRequest` "a plan's start
/// names its plan and step". A malformed value of any of the three is refused the same way.
fn by_params(word: StartWord, parameters: &Map<String, Value>) -> Result<ByParams, ActionError> {
    let refuse = || invalid("a plan's start names its plan and step");
    let by = match parameters.get("by") {
        None => None,
        Some(value) => match value.as_str() {
            Some("plan") => Some(ByWord::Plan),
            Some("idle") => Some(ByWord::Idle),
            _ => return Err(refuse()),
        },
    };
    let plan = match parameters.get("plan") {
        None => None,
        Some(value) => Some(
            value
                .as_u64()
                .and_then(|no| u32::try_from(no).ok())
                .filter(|no| *no >= 1)
                .ok_or_else(refuse)?,
        ),
    };
    let step = match parameters.get("step") {
        None => None,
        Some(value) => Some(
            value
                .as_u64()
                .and_then(|step| u16::try_from(step).ok())
                .filter(|step| usize::from(*step) <= MAX_PLAN_STEPS)
                .ok_or_else(refuse)?,
        ),
    };
    let ok = match (word, by) {
        (StartWord::PlanFile, None) => plan.is_none() && step.is_none(),
        (StartWord::PlanClear | StartWord::PlanStrike, None) => plan.is_some() && step.is_none(),
        (StartWord::PlanEnd, Some(ByWord::Plan)) => plan.is_some() && step.is_some(),
        (
            StartWord::Act {
                kind:
                    ActKind::Drive | ActKind::Scan | ActKind::Sound | ActKind::Mine | ActKind::Recover,
                ..
            },
            Some(ByWord::Plan),
        ) => plan.is_some() && step.is_some(),
        (
            StartWord::Act {
                kind: ActKind::Recover,
                ..
            },
            Some(ByWord::Idle),
        ) => plan.is_none() && step.is_none(),
        (StartWord::Act { .. } | StartWord::MineStop, None) => plan.is_none() && step.is_none(),
        (
            StartWord::PlanFile
            | StartWord::PlanClear
            | StartWord::PlanStrike
            | StartWord::PlanEnd
            | StartWord::MineStop
            | StartWord::Act { .. },
            Some(_),
        )
        | (StartWord::PlanEnd, None) => false,
    };
    if ok {
        Ok(ByParams { by, plan, step })
    } else {
        Err(refuse())
    }
}

/// **Who set a start going, and its plan binding** (doc 20 cut SM.11): what the write stamps on
/// the act or the run, and the plan's update it applies in the same bundle.
#[derive(Clone, Debug, Default)]
struct Stamp {
    by: Option<ActBy>,
    plan: Option<PlanFixed>,
}

/// `Conflict` "the plan moved on" (the brief's D16: the world moved between the decision and the
/// landing; the next fire decides again).
fn moved_on() -> ActionError {
    port_error(ActionErrorCode::Conflict, "the plan moved on")
}

/// Push `note` onto `plan`'s ring under a fresh `seq`, the oldest dropped past
/// [`MAX_PLAN_NOTES`].
fn push_note(plan: &mut FillPlan, note: PlanNoteKind, at_ic: i64) -> Result<(), ActionError> {
    plan.seq = plan
        .seq
        .checked_add(1)
        .ok_or_else(|| port_error(ActionErrorCode::Integrity, "a plan's seq overflows"))?;
    plan.notes.push(PlanNote {
        seq: plan.seq,
        at_ic,
        note,
    });
    while plan.notes.len() > MAX_PLAN_NOTES {
        plan.notes.remove(0);
    }
    Ok(())
}

/// **A plan step's update, applied** (doc 20 cut SM.11; the brief's 3.6): the cursor, the passed
/// pockets, a parking, the notes (each under its own `seq`, in order) and one more `seq` — on the
/// record the start writes. For a `plan_end` binding the plan then ends with its reason and one
/// `Ended` note.
fn apply_update(record: &mut RigRecord, binding: &PlanFixed, now: i64) -> Result<(), ActionError> {
    let integrity = || port_error(ActionErrorCode::Integrity, "a plan's update does not apply");
    let plan = record.plan.as_mut().ok_or_else(integrity)?;
    let update = &binding.update;
    if usize::from(update.cursor) > plan.steps.len() {
        return Err(integrity());
    }
    plan.cursor = update.cursor;
    plan.passed.clone_from(&update.passed);
    if let Some(why) = update.parking {
        plan.state = FillState::Parking { why };
    }
    for note in &update.notes {
        push_note(plan, *note, now)?;
    }
    if let Some(why) = binding.end {
        plan.passed.clear();
        plan.state = FillState::Ended { why, at_ic: now };
        push_note(plan, PlanNoteKind::Ended { why }, now)?;
    }
    plan.seq = plan.seq.checked_add(1).ok_or_else(integrity)?;
    plan.check().map_err(|_| integrity())
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
    /// Doc 20 cut SM.7: the pocket cursors (`tacsim.pockets`) a run locks and draws.
    pub pockets: StateFamilyHandle<mush_industry::OreDepositId, PocketCursor>,
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
        ActParams::None {}
        | ActParams::Dump { .. }
        | ActParams::Drive { .. }
        | ActParams::Unload { .. } => None,
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
        // The lead's pre-commit round, item 3: the actor's leased rigs off the helm, offered only
        // their plan's variants below.
        let off_helm: Vec<RigAt> = leased_rigs(w, actor)
            .into_iter()
            .filter(|rig| rigs.iter().all(|at| at.tv != rig.tv))
            .collect();
        if rigs.is_empty() && off_helm.is_empty() {
            return Ok(None);
        }
        // Doc 20 cut SM.4a (the brief's reviewer finding F5): each site's composite is peeked
        // once for the whole projection and shared by every rig's scan plan — and since cut
        // SM.4b each site's home field too. The offer never wants a cold one (it cannot
        // mutate): a cold peek is refused `ground_cold`, as the command's is.
        let mut peeks = Peeks::default();
        let mut fixed_rigs: BTreeMap<String, FixedRig> = BTreeMap::new();
        for rig in &rigs {
            let battery_id = rig_battery_id(w, rig.tv);
            let scanner = rig_scanner_id(w, rig.tv).and_then(|id| scanner_of(&self.tools, &id));
            let battery = battery_id
                .as_ref()
                .and_then(|id| battery_of(&self.tools, id));
            let rates = rig_rates(w, rig.tv, battery);
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
            // Doc 20 cut SM.4b: a far recover plans its leg home the same way (a descent of the
            // warm home field, O(path), no search).
            plans.insert(
                wire_word(ActKind::Recover, false).to_owned(),
                FixedPlan::of(recover_plan(w, actor, rig, &mut peeks, rates)),
            );
            // Doc 20 cut SM.4b (the brief's decision P2): the drive's order, fixed.
            plans.insert(
                wire_word(ActKind::Drive, false).to_owned(),
                if has_battery {
                    drive_fixed(w, actor, rig)
                } else {
                    FixedPlan::refused("tv_dark")
                },
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
            let scan = scanner
                .map(|scanner| scan_plan(w, actor, rig, scanner, &mut peeks, rates.as_ref()));
            plans.insert(
                wire_word(ActKind::Scan, false).to_owned(),
                reading_fixed(w, scan, has_battery),
            );
            for quiet in [false, true] {
                let sound = scanner.map(|scanner| {
                    sound_plan(w, actor, rig, scanner, quiet, &mut peeks, rates.as_ref())
                });
                plans.insert(
                    wire_word(ActKind::Sound, quiet).to_owned(),
                    reading_fixed(w, sound, has_battery),
                );
            }
            // Doc 20 cut SM.7 (the brief's 3.4, decision R10): the run's order, re-planned and
            // required equal; `mine_stop` is the prepare's to decide.
            let drill = rig_drill_id(w, rig.tv).and_then(|id| drill_of(&self.tools, &id));
            // The rig's next ordinal, from its record (absent: 1); unreadable → `None`, and the
            // re-plan refuses as the record's read does.
            let next_ordinal = engine
                .action_state()
                .get(&self.excursions, &RigKey(rig.tv))
                .ok()
                .map(|found| found.map_or(1, |record| record.ordinal.saturating_add(1)));
            plans.insert(
                wire_word(ActKind::Mine, false).to_owned(),
                mine_fixed(
                    engine,
                    actor,
                    rig,
                    drill,
                    has_battery,
                    rates.as_ref(),
                    &mut peeks,
                    next_ordinal,
                ),
            );
            plans.insert(
                "mine_stop".to_owned(),
                FixedPlan {
                    ok: true,
                    ..FixedPlan::default()
                },
            );
            // Doc 20 cut SM.8 (the brief's 3.7): the unload's two sources, fixed; the prepare
            // picks by the start's `how`.
            plans.insert(
                wire_word(ActKind::Unload, false).to_owned(),
                unload_fixed(engine, actor, rig),
            );
            // Doc 20 cut SM.11 (the brief's 3.6): the plan's variants, each under its own key so
            // it never meets a hand variant — the filing (the RAM order, re-compiled and required
            // equal), the plan's next move (the interpreter over the durable record, only for a
            // rig whose record holds a live plan: no plan, no cost), the clear and the strike.
            let record: Option<RigRecord> = engine
                .action_state()
                .get(&self.excursions, &RigKey(rig.tv))
                .ok()
                .flatten()
                .cloned();
            plans.insert(
                "plan_file".to_owned(),
                self.fill_fixed(engine, actor, rig, record.as_ref(), &mut peeks),
            );
            plans.extend(self.plan_variants(
                engine,
                actor,
                rig,
                record.as_ref(),
                &mut peeks,
                has_battery,
                true,
            ));
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
        // The lead's pre-commit round, item 3 (the judge's correctness/F5; D22 "stays
        // clearable"): the actor's own leased rigs off the helm whose record holds the actor's own
        // plan — the plan's variants only: its next move (off the helm the interpreter answers a
        // wait, or past the bound the `Bound` end, which the tick submits as the author wherever
        // they stand) and the clear and the strike, while the author is aboard the rig's hull.
        for rig in &off_helm {
            let record: Option<RigRecord> = engine
                .action_state()
                .get(&self.excursions, &RigKey(rig.tv))
                .ok()
                .flatten()
                .cloned();
            if !record
                .as_ref()
                .and_then(|record| record.plan.as_ref())
                .is_some_and(|plan| plan.author == actor)
            {
                continue;
            }
            let battery_id = rig_battery_id(w, rig.tv);
            let has_battery = battery_id.is_some();
            let aboard = crate::port::voyage::aboard(w, rig.hull, actor);
            let plans: BTreeMap<String, FixedPlan> = self
                .plan_variants(
                    engine,
                    actor,
                    rig,
                    record.as_ref(),
                    &mut peeks,
                    has_battery,
                    aboard,
                )
                .into_iter()
                .collect();
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
        if fixed_rigs.is_empty() {
            return Ok(None);
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
                    // Every rig the offer fixes: the helm's and, the lead's pre-commit round
                    // item 3, the actor's own off it.
                    choices: rigs
                        .iter()
                        .chain(off_helm.iter())
                        .map(|rig| rig.tv.0.to_string())
                        .filter(|tv| fixed_rigs.contains_key(tv))
                        .collect(),
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
            // Doc 20 cut SM.4b (the review's correctness/F5): a drive's planned end cell, which
            // the prepare holds to the order the offer fixed.
            parameter(
                "to",
                false,
                ProjectedParameterKind::Integer {
                    minimum: 0,
                    maximum: 65_535,
                },
            )?,
            // Doc 20 cut SM.8 (the brief's 3.7, decision R15; the drive's `to` precedent): how an
            // unload was planned, and a part unload's tonnes — the prepare holds the start to the
            // variant they name.
            parameter(
                "how",
                false,
                ProjectedParameterKind::Choice {
                    choices: ["part", "queued", "whole"]
                        .iter()
                        .map(|w| (*w).to_owned())
                        .collect(),
                },
            )?,
            parameter(
                "qty",
                false,
                ProjectedParameterKind::Integer {
                    minimum: 1,
                    maximum: i64::try_from(tv::MAX_TYPED_TONNES).unwrap_or(99),
                },
            )?,
            // Doc 20 cut SM.11 (the brief's 3.6): who set the act going, and a plan step's plan
            // and step — server-authored by the surface tick; the prepare holds them to the
            // offer's plan variant and refuses any other combination.
            parameter(
                "by",
                false,
                ProjectedParameterKind::Choice {
                    choices: ["idle", "plan"].iter().map(|w| (*w).to_owned()).collect(),
                },
            )?,
            parameter(
                "plan",
                false,
                ProjectedParameterKind::Integer {
                    minimum: 1,
                    maximum: i64::from(u32::MAX),
                },
            )?,
            parameter(
                "step",
                false,
                ProjectedParameterKind::Integer {
                    minimum: 0,
                    maximum: i64::try_from(MAX_PLAN_STEPS).unwrap_or(64),
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
                "Send a rig down the ramp, scan, sound or mine the ground, bring it in, unload its \
                 hopper into the hold, or empty it onto the spoil.",
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
        let word = act_param(parameters)?;
        // Doc 20 cut SM.8 (decision R15): `how` and `qty` belong to an unload's start alone.
        let unloading = matches!(
            word,
            StartWord::Act {
                kind: ActKind::Unload,
                ..
            }
        );
        if !unloading && (parameters.contains_key("how") || parameters.contains_key("qty")) {
            return Err(invalid("an unload names how it was planned"));
        }
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
        // Doc 20 cut SM.11 (the brief's 3.6): who set it going, and a plan step's binding — the
        // parameter rule first.
        let by = by_params(word, parameters)?;
        // Doc 20 cut SM.11 (D8): a live plan holds its rig from every hand start — a hand unload
        // too, never the recover's queued one — and its stop.
        let held = read_record(txn, &self.excursions, tv)?
            .and_then(|record| record.live_plan_for(actor).map(|plan| plan.no));
        let hand = by.by.is_none();
        let queued_unload = matches!(
            word,
            StartWord::Act {
                kind: ActKind::Unload,
                ..
            }
        ) && how_of(parameters) == Some(UnloadHow::Queued);
        let control = matches!(
            word,
            StartWord::PlanFile | StartWord::PlanClear | StartWord::PlanStrike | StartWord::PlanEnd
        );
        if hand && !control && !queued_unload && held.is_some() {
            return Err(port_error(
                ActionErrorCode::ProviderRejected,
                code_of_word("plan_live"),
            ));
        }
        // Doc 20 cut SM.7: `mine_stop` is a control on a committed run, no act; doc 20 cut SM.11:
        // so are the plan's four.
        let (kind, quiet) = match word {
            StartWord::Act { kind, quiet } => (kind, quiet),
            StartWord::MineStop => {
                return self.stop_run(invocation, txn, (actor, tv, ordinal));
            }
            StartWord::PlanFile => {
                return self.file_plan(invocation, txn, (actor, tv, ordinal, now), rig);
            }
            StartWord::PlanClear | StartWord::PlanStrike => {
                return self.clear_plan(
                    invocation,
                    txn,
                    (actor, tv, now),
                    rig,
                    word == StartWord::PlanStrike,
                    by.plan,
                );
            }
            StartWord::PlanEnd => {
                return self.end_plan(invocation, txn, (actor, tv, now), rig, &by);
            }
        };
        let wire = wire_word(kind, quiet);
        let (plan, stamp) = match by.by {
            None => (
                rig.plans
                    .get(wire)
                    .cloned()
                    .ok_or_else(|| invalid("that act is not planned"))?,
                Stamp::default(),
            ),
            // A plan's step: the offer's plan variant, held to the start's plan, step and word.
            Some(ByWord::Plan) => {
                let fixed = rig
                    .plans
                    .get("plan")
                    .cloned()
                    .ok_or_else(|| invalid("that act is not planned"))?;
                if !fixed.ok {
                    return Err(port_error(
                        ActionErrorCode::ProviderRejected,
                        code_of_word(fixed.code.as_deref().unwrap_or("plan_moved")),
                    ));
                }
                let binding = fixed.plan.clone().ok_or_else(moved_on)?;
                let live = read_record(txn, &self.excursions, tv)?
                    .and_then(|record| record.live_plan_for(actor).map(|plan| (plan.no, plan.seq)));
                if Some(binding.no) != by.plan
                    || Some(binding.step) != by.step
                    || binding.word != wire
                    || live != Some((binding.no, binding.seq))
                {
                    return Err(moved_on());
                }
                let step = binding.step;
                (
                    fixed,
                    Stamp {
                        by: Some(ActBy::Plan {
                            no: binding.no,
                            step,
                        }),
                        plan: Some(binding),
                    },
                )
            }
            // The idle return: the recover's own plan, the idle condition checked on the record.
            Some(ByWord::Idle) => {
                let fixed = rig
                    .plans
                    .get(wire)
                    .cloned()
                    .ok_or_else(|| invalid("that act is not planned"))?;
                let record = read_record(txn, &self.excursions, tv)?;
                let idle = record.as_ref().is_some_and(|record| {
                    record.live_plan_for(actor).is_none()
                        && record.act.is_none()
                        && record.run.is_none()
                        && record.queued.is_none()
                        && record.excursion.is_some()
                        && record.idle_since().is_some_and(|since| {
                            since
                                .checked_add(crate::surface::plan::IDLE_PARK_IC_SECS)
                                .is_some_and(|due| due <= now)
                        })
                });
                if !idle {
                    return Err(port_error(
                        ActionErrorCode::Conflict,
                        "that rig is not idle",
                    ));
                }
                (
                    fixed,
                    Stamp {
                        by: Some(ActBy::Idle),
                        plan: None,
                    },
                )
            }
        };
        if !plan.ok {
            return Err(port_error(
                ActionErrorCode::ProviderRejected,
                code_of_word(plan.code.as_deref().unwrap_or("tv_dark")),
            ));
        }
        // Doc 20 cut SM.11: nothing in flight for a plan's step (SM.7/SM.8's idleness rule).
        if stamp.plan.is_some()
            && read_record(txn, &self.excursions, tv)?.is_some_and(|record| {
                record.act.is_some() || record.run.is_some() || record.queued.is_some()
            })
        {
            return Err(port_error(ActionErrorCode::Conflict, "that rig is busy"));
        }
        // Doc 20 cut SM.7: a run starts on its own path (one start action, SM.3b's decision 25).
        if kind == ActKind::Mine {
            return self.start_run(
                invocation,
                txn,
                (actor, tv, ordinal, now),
                rig,
                &plan,
                &world,
                &stamp,
            );
        }
        // Doc 20 cut SM.8: an unload starts on its own path too.
        if kind == ActKind::Unload {
            return self.start_unload(invocation, txn, (actor, tv, ordinal, now), rig, &plan);
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
        // Doc 20 cut SM.4b (the brief's decision P2): a drive's fixed plan is its order, bound
        // to the ordinal it was planned for — and (the review's correctness/F5) to the end cell
        // the start names, so a start whose order a later `go` replaced never runs the later
        // order's leg.
        // Doc 20 cut SM.11: a plan's drive is bound by its plan and step instead of `to`.
        if kind == ActKind::Drive
            && (plan.ordinal != Some(ordinal)
                || (stamp.plan.is_none()
                    && plan.params.as_ref().and_then(order_to_word) != to_word_of(parameters)))
        {
            return Err(port_error(
                ActionErrorCode::Conflict,
                "that rig's orders moved on",
            ));
        }
        // 3. One act at a time — and (doc 20 cut SM.7, decision R1) only a scan or a sounding
        // beside a run.
        if record.act.is_some() {
            return Err(port_error(ActionErrorCode::Conflict, "that rig is busy"));
        }
        if record.run.is_some() && !matches!(kind, ActKind::Scan | ActKind::Sound) {
            return Err(port_error(
                ActionErrorCode::ProviderRejected,
                code_of_word("mine_running"),
            ));
        }
        // 4. The launch's open excursion and the caps — a launch's, and a scan's, a sounding's
        // or (doc 20 cut SM.4b) a drive's that implies one (doc 20 cut SM.4a, lead ruling 2); a
        // recover from the bay closes one.
        let launches =
            kind == ActKind::Launch || plan.params.as_ref().is_some_and(ActParams::launches);
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
        // spend above), which the rule always passes. **Doc 20 cut SM.4b** (lead ruling 5,
        // decision P6): a recover is never held to the floor — it IS the way home — and a
        // recover with a leg must only afford its own leg, on the record's own value (no
        // battery read). SM.11's idle return-and-park must treat that refusal as "parked where
        // it stands".
        if kind == ActKind::Recover {
            if !leg_affordable(&record.charge, reserved_total(&record), plan.cost_cu) {
                return Err(port_error(
                    ActionErrorCode::ProviderRejected,
                    code_of_word("leg_short"),
                ));
            }
        } else if let Some(battery) = battery
            && reserve_check(
                charge_for_act(
                    &record.charge,
                    now,
                    plan.params.as_ref().is_some_and(ActParams::launches),
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
            by: stamp.by,
        });
        // Doc 20 cut SM.8 (lead ruling 1): any start that lands supersedes a queued unload.
        record.queued = None;
        if launches {
            record.excursion = Some(Excursion {
                id: ExcursionId { tv, launch_ic: now },
                operator: actor,
                hull: rig.hull,
                slot: rig.slot,
                site,
            });
        }
        // Doc 20 cut SM.11: a plan step's update rides the same bundle.
        if let Some(binding) = &stamp.plan {
            apply_update(&mut record, binding, now)?;
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

/// A pocket cursor read in the transaction, its key re-checked (the families rule).
fn read_cursor(
    txn: &mut ActionTxn<'_>,
    family: &StateFamilyHandle<mush_industry::OreDepositId, PocketCursor>,
    deposit: mush_industry::OreDepositId,
) -> Result<Option<PocketCursor>, ActionError> {
    let found = txn
        .get(family, &deposit)
        .map_err(|_| {
            port_error(
                ActionErrorCode::Integrity,
                "the pocket cursors are unavailable",
            )
        })?
        .cloned();
    if let Some(cursor) = &found
        && cursor.deposit != deposit
    {
        return Err(port_error(
            ActionErrorCode::Integrity,
            "a pocket cursor disagrees with its own key",
        ));
    }
    Ok(found)
}

/// **Stage a cursor's new state** (insert, replace or remove; `before` is whether one stood):
/// `None` removes it (an emptied cursor), and nothing is staged when none stood and none is due.
fn write_cursor(
    txn: &mut ActionTxn<'_>,
    family: &StateFamilyHandle<mush_industry::OreDepositId, PocketCursor>,
    deposit: mush_industry::OreDepositId,
    before: bool,
    after: Option<PocketCursor>,
) -> Result<(), ActionError> {
    let integrity = || {
        port_error(
            ActionErrorCode::Integrity,
            "the pocket cursor did not stage",
        )
    };
    match (before, after) {
        (_, Some(cursor)) => {
            cursor.check().map_err(|_| integrity())?;
            if cursor.deposit != deposit {
                return Err(integrity());
            }
            let staged = if before {
                txn.replace(family, &deposit, cursor)
            } else {
                txn.insert(family, &deposit, cursor)
            };
            staged.map(|_| ()).map_err(|_| integrity())
        }
        (true, None) => txn
            .remove(family, &deposit)
            .map(|_| ())
            .map_err(|_| integrity()),
        (false, None) => Ok(()),
    }
}

/// **The pocket's occupancy, in the transaction** (doc 20 cut SM.7; lead ruling 3, decision R4):
/// another rig's live lock on any of `lock_rows` (`lock_live` over the holder's record read here,
/// matched by the row itself: the SM.7 lead's pre-commit round, item 1),
/// or another of the world's rigs whose run stands on `(site, pocket)` — the record check makes an
/// all-barren pocket, which has no row, refuse alike.
fn pocket_taken(
    txn: &mut ActionTxn<'_>,
    (excursions, pockets): (
        &StateFamilyHandle<RigKey, RigRecord>,
        &StateFamilyHandle<mush_industry::OreDepositId, PocketCursor>,
    ),
    tv: ObjId,
    (site, pocket): (&CanonicalId, [i64; 2]),
    lock_rows: &[LockRow],
    world: &[u32],
) -> Result<bool, ActionError> {
    for row in lock_rows {
        let lock = read_cursor(txn, pockets, row.deposit)?
            .and_then(|cursor| cursor.row(row.pocket).and_then(|r| r.lock.clone()));
        if let Some(lock) = lock
            && lock.tv != tv
        {
            let holder = read_record(txn, excursions, lock.tv)?;
            if lock_live(&lock, holder.as_ref(), site, row) {
                return Ok(true);
            }
        }
    }
    for other in world
        .iter()
        .copied()
        .map(ObjId)
        .filter(|other| *other != tv)
    {
        if read_record(txn, excursions, other)?
            .and_then(|record| record.run)
            .is_some_and(|run| &run.site == site && run.pocket == pocket)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

impl SurfaceActProvider {
    /// **A run's start** (doc 20 cut SM.7; the brief's 3.4; lead rulings 1–6): the order's
    /// ordinal (`Conflict` "that rig's orders moved on"), the rig idle — no act, no run
    /// (`Conflict` "that rig is busy") — the licence's lapse after `now` (the offer re-read the
    /// standing at landing; `unlicensed`), the pocket free in the transaction (`locked`), the
    /// family's cursor row drawn exactly as the preview saw it (`Conflict` "that pocket changed;
    /// preview it again"), the hull's runs exactly as the hold's room counted them (`Conflict` "the
    /// hold's room changed; preview it again"), a blind core's body caps (`chart_full`), and the
    /// reserve over the run's whole cost and the trip home; then **one bundle**: the record's
    /// ordinal and its run (cycle 1, due `now + secs`, the whole reservation), each lock row
    /// written on its cursor. The lease, the tenure and the plan itself are the offer's re-plan at
    /// landing (`mine_fixed`: the prepare holds no world). The outcome is the start's own shape,
    /// identical for a barren and an ore blind core.
    #[allow(clippy::too_many_lines, clippy::too_many_arguments)] // one ordered list, one bundle
    fn start_run(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        (actor, tv, ordinal, now): (ObjId, ObjId, u32, i64),
        rig: &FixedRig,
        plan: &FixedPlan,
        world: &[u32],
        stamp: &Stamp,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
        let conflict = |message: &'static str| port_error(ActionErrorCode::Conflict, message);
        let rejected =
            |word: &str| port_error(ActionErrorCode::ProviderRejected, code_of_word(word));
        let fixed = plan
            .mine
            .clone()
            .ok_or_else(|| invalid("a run's plan names its pocket"))?;
        let site = plan
            .site
            .clone()
            .ok_or_else(|| invalid("an act's plan names its site"))?;
        let battery = rig
            .battery_id
            .as_ref()
            .and_then(|id| battery_of(&self.tools, id))
            .ok_or_else(|| rejected("tv_dark"))?;
        // 2. The record and the ordinal.
        let existing = read_record(txn, &self.excursions, tv)?;
        let existed = existing.is_some();
        let mut record = existing.unwrap_or_else(|| RigRecord::fresh(tv, battery));
        if record.ordinal.checked_add(1) != Some(ordinal) || plan.ordinal != Some(ordinal) {
            return Err(conflict("that rig's orders moved on"));
        }
        // 3. Idle: no act and no run.
        if record.act.is_some() || record.run.is_some() {
            return Err(conflict("that rig is busy"));
        }
        // 4. The licence: in force at landing.
        if !fixed.lapses_ic.is_some_and(|lapses| now < lapses) {
            return Err(rejected("unlicensed"));
        }
        // 5. The lock.
        if pocket_taken(
            txn,
            (&self.excursions, &self.pockets),
            tv,
            (&site, fixed.pocket),
            &fixed.lock_rows,
            world,
        )? {
            return Err(rejected("locked"));
        }
        // 6. The cursor's drawn, as previewed.
        let drawn = read_cursor(txn, &self.pockets, fixed.deposit)?
            .and_then(|cursor| cursor.row(fixed.number).map(|row| row.drawn_t))
            .unwrap_or(0);
        if drawn != fixed.drawn {
            return Err(conflict("that pocket changed; preview it again"));
        }
        // 7. The hold's room, as counted: every hull rig's run as the plan saw it.
        if fixed.outstanding.len()
            > usize::try_from(crate::definitions::logistics::MAX_BAY_ITEMS).unwrap_or(usize::MAX)
        {
            return Err(invalid("a run's plan names a bounded set of hull rigs"));
        }
        for (other, seen) in &fixed.outstanding {
            let now_seen = read_record(txn, &self.excursions, *other)?
                .and_then(|record| record.run)
                .map(|run| (run.ordinal, run.outstanding_t()));
            if now_seen != *seen {
                return Err(conflict("the hold's room changed; preview it again"));
            }
        }
        // 8. A blind core's body caps (the core adds the block to the row).
        if fixed.blind {
            if plan.body_sites.len() > chart::MAX_CHART_SITES_PER_BODY {
                return Err(invalid(
                    "a reading act's offer names a bounded set of sites",
                ));
            }
            let mut rows = Vec::new();
            for other in &plan.body_sites {
                if let Some(found) = read_chart(txn, &self.charts, actor, other)? {
                    rows.push(found);
                }
            }
            if !plan.body_sites.contains(&site)
                && let Some(found) = read_chart(txn, &self.charts, actor, &site)?
            {
                rows.push(found);
            }
            if !body_admits(&rows, &site, &[], &[fixed.letter.to_string()]) {
                return Err(rejected("chart_full"));
            }
        }
        // 9. The reserve: the run's whole cost and the trip home (§4.1; lead ruling 1).
        if reserve_check(
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
        // 10. The write. A rig on the ground whose charge still reads the bay's (a staff
        // placement) leaves the umbilical now, so each cycle settles out of the bay without the
        // battery (the brief's 3.7 step 5).
        if record.charge.in_bay {
            record.charge = Charge {
                value_cu: charge_at(&record.charge, now, battery),
                at_ic: now,
                in_bay: false,
            };
        }
        let due_ic = now
            .checked_add(i64::from(fixed.secs))
            .ok_or_else(|| integrity("the cycle's due instant overflows"))?;
        record.ordinal = ordinal;
        record.run = Some(Run {
            ordinal,
            operator: actor,
            hull: rig.hull,
            slot: rig.slot,
            site: site.clone(),
            body: fixed.body.clone(),
            stand: fixed.stand,
            pocket: fixed.pocket,
            number: fixed.number,
            letter: fixed.letter,
            family: fixed.family,
            deposit: fixed.deposit,
            good: fixed.good.clone(),
            source: fixed.source,
            hopper: fixed.hopper,
            cycles: fixed.cycles,
            k: 1,
            secs: fixed.secs,
            cost_cu: fixed.cost_cu,
            reserve_t: fixed.reserve_t.clone(),
            class_t: fixed.class_t,
            pocket_t: fixed.pocket_t,
            blind: fixed.blind,
            stop: false,
            lapses_ic: fixed.lapses_ic,
            lock_rows: fixed.lock_rows.clone(),
            due_ic,
            reserved_cu: plan.cost_cu,
            by: stamp.by,
        });
        // Doc 20 cut SM.8 (lead ruling 1): a run's start supersedes a queued unload too (a run is
        // never started from the bay, so none is expected).
        record.queued = None;
        // Doc 20 cut SM.11: a plan's mine step's update rides the same bundle.
        if let Some(binding) = &stamp.plan {
            apply_update(&mut record, binding, now)?;
        }
        let lock = PocketLock { tv, ordinal };
        for row in &fixed.lock_rows {
            let before = read_cursor(txn, &self.pockets, row.deposit)?;
            let after = cursor_locked(before.as_ref(), row.deposit, row.pocket, lock.clone())
                .ok_or_else(|| integrity("the pocket's cursor cannot take the lock"))?;
            write_cursor(
                txn,
                &self.pockets,
                row.deposit,
                before.is_some(),
                Some(after),
            )?;
        }
        write_record(txn, &self.excursions, record, existed)?;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({
                "act": wire_word(ActKind::Mine, false),
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

    /// **An unload's start** (doc 20 cut SM.8; §7.1 "re-checked at prepare"; the brief's 3.7,
    /// decisions R4, R7, R15 and R16), one bundle:
    ///
    /// 1. **the parameters** — `how` names the variant and `qty` comes exactly with `part` (else
    ///    `InvalidRequest` "an unload names how it was planned");
    /// 2. **the ordinal** — the record's next (`Conflict` "that rig's orders moved on");
    /// 3. **the variant, held to the parameter** — `queued`: the offer's queued variant for this
    ///    ordinal and the record still holding its queue; `whole`/`part`: the offer's hand variant
    ///    for this ordinal with the same `how` and `qty`; anything else `Conflict` "that rig's
    ///    orders moved on" (a queued start never runs a hand order's goods, a hand `go` never a
    ///    queued plan or another `go`'s; a race of the two is settled here, whichever prepares
    ///    first);
    /// 4. **idle** — no act and no run (`Conflict` "that rig is busy");
    /// 5. **the re-check** — this hopper's pools, read here, hold each frozen good (`available`),
    ///    and the hold's room recomputed here with the one rule (`capacity::room_less` over the
    ///    hold's ore compartment of the fixed plan — the plan's own goods table — its contents,
    ///    and every sibling rig's hopper and outstanding run) admits the frozen goods in id order;
    ///    either failing → `ProviderRejected` `unload_changed`;
    /// 6. **the write** — the ordinal, the act (`UNLOAD_S`, or 0 s when a queued unload moves
    ///    nothing: told once, R7), the queue cleared.
    ///
    /// No licence, tenure, lease or battery is read (the battery's row names only a fresh record's
    /// charge). The outcome is the start's own shape.
    #[allow(clippy::too_many_lines)] // one ordered refusal list, then one bundle
    fn start_unload(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        (actor, tv, ordinal, now): (ObjId, ObjId, u32, i64),
        rig: &FixedRig,
        plan: &FixedPlan,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
        let conflict = |message: &'static str| port_error(ActionErrorCode::Conflict, message);
        let changed = || {
            port_error(
                ActionErrorCode::ProviderRejected,
                code_of_word("unload_changed"),
            )
        };
        let parameters = parameters_of(invocation)?;
        // 1. The parameters.
        let how =
            how_of(parameters).ok_or_else(|| invalid("an unload names how it was planned"))?;
        let qty = qty_of(parameters);
        let named = parameters.contains_key("qty");
        if (how == UnloadHow::Part) != named || (named && qty.is_none()) {
            return Err(invalid("an unload names how it was planned"));
        }
        let fixed = plan
            .unload
            .clone()
            .ok_or_else(|| invalid("an unload's plan names its vaults"))?;
        let site = plan
            .site
            .clone()
            .ok_or_else(|| invalid("an act's plan names its site"))?;
        if fixed.siblings.len()
            > usize::try_from(crate::definitions::logistics::MAX_BAY_ITEMS).unwrap_or(usize::MAX)
            || fixed.goods_of_hold.len() > mush_inventory::MAX_POOLS_PER_ENDPOINT
        {
            return Err(invalid(
                "an unload's plan names a bounded set of rigs and goods",
            ));
        }
        // 2. The record and the ordinal.
        let battery = rig
            .battery_id
            .as_ref()
            .and_then(|id| battery_of(&self.tools, id));
        let existing = read_record(txn, &self.excursions, tv)?;
        let existed = existing.is_some();
        let mut record = existing.unwrap_or_else(|| {
            battery.map_or_else(
                || RigRecord::fresh_unread(tv),
                |battery| RigRecord::fresh(tv, battery),
            )
        });
        if record.ordinal.checked_add(1) != Some(ordinal) {
            return Err(conflict("that rig's orders moved on"));
        }
        // 3. The variant, held to the parameter.
        let (goods, excursion) = match how {
            UnloadHow::Queued => match (&fixed.queued, &record.queued) {
                (Some(queued), Some(_)) if queued.ordinal == ordinal => {
                    (queued.goods.clone(), queued.excursion)
                }
                _ => return Err(conflict("that rig's orders moved on")),
            },
            UnloadHow::Whole | UnloadHow::Part => match &fixed.hand {
                Some(hand) if hand.ordinal == ordinal && hand.how == how && hand.qty == qty => {
                    (hand.goods.clone(), None)
                }
                _ => return Err(conflict("that rig's orders moved on")),
            },
        };
        if goods.len() > MAX_DUMP_GOODS || goods.values().any(|qty| *qty == 0) {
            return Err(invalid(
                "an unload moves a bounded set of positive quantities",
            ));
        }
        // 4. Idle: no act and no run.
        if record.act.is_some() || record.run.is_some() {
            return Err(conflict("that rig is busy"));
        }
        // 5. The re-check: the hopper holds the frozen goods; the hold's room admits them.
        let hopper = EndpointRef(VaultId(fixed.hopper));
        for (good, qty) in &goods {
            let available = self
                .inventory
                .pool(txn, hopper, good)
                .map_err(|_| integrity("the hopper cannot be read"))?
                .map_or(0, |pool| pool.available);
            if available < *qty {
                return Err(changed());
            }
        }
        if !goods.is_empty() {
            let table = crate::port::market::plan_goods(&fixed.plan).ok_or_else(|| {
                tracing::error!(
                    tv = tv.0,
                    "tacsim: an unload's hold plan has no goods table"
                );
                integrity("the hold's plan cannot be read")
            })?;
            let ore = CanonicalId::new("metal_ore").map_err(|_| integrity("the ore id"))?;
            let compartment = crate::port::capacity::compartment_for(
                table,
                &crate::port::market::plan_compartments(&fixed.plan),
                &ore,
            )
            .ok_or_else(|| {
                tracing::error!(
                    tv = tv.0,
                    "tacsim: an unload's hold has no one ore compartment"
                );
                integrity("the hold's plan cannot be read")
            })?;
            let hold = EndpointRef(VaultId(fixed.hold));
            let mut contents: BTreeMap<CanonicalId, u64> = BTreeMap::new();
            for good in &fixed.goods_of_hold {
                let held = self
                    .inventory
                    .pool(txn, hold, good)
                    .map_err(|_| integrity("the hold cannot be read"))?
                    .map_or(0, |pool| pool.quantity);
                if held > 0 {
                    contents.insert(good.clone(), held);
                }
            }
            let mut reserved: Vec<BTreeMap<CanonicalId, u64>> = Vec::new();
            for sibling in &fixed.siblings {
                let mut theirs: BTreeMap<CanonicalId, u64> = BTreeMap::new();
                for good in &sibling.goods {
                    let held = self
                        .inventory
                        .pool(txn, EndpointRef(VaultId(sibling.hopper)), good)
                        .map_err(|_| integrity("a rig's hopper cannot be read"))?
                        .map_or(0, |pool| pool.quantity);
                    if held > 0 {
                        theirs.insert(good.clone(), held);
                    }
                }
                if let Some(run) =
                    read_record(txn, &self.excursions, sibling.tv)?.and_then(|record| record.run)
                {
                    let slot = theirs.entry(run.good.clone()).or_insert(0);
                    *slot = slot.saturating_add(run.outstanding_t());
                }
                reserved.push(theirs);
            }
            let mut room =
                crate::port::capacity::room_less(table, &compartment, &contents, &reserved)
                    .map_err(|_| integrity("the hold's room cannot be weighed"))?;
            for (good, qty) in &goods {
                let fits = crate::port::capacity::units_in(table, &compartment, &room, good)
                    .map_err(|_| integrity("the hold's room cannot be weighed"))?;
                if fits < *qty {
                    return Err(changed());
                }
                crate::port::capacity::take_room(table, &compartment, &mut room, good, *qty)
                    .map_err(|_| integrity("the hold's room cannot be weighed"))?;
            }
        }
        // 6. The write: the act (a queued unload that moves nothing is a 0 s act, told once —
        // decision R7), the queue cleared.
        let secs = if goods.is_empty() { 0 } else { UNLOAD_S };
        let due_ic = now
            .checked_add(i64::from(secs))
            .ok_or_else(|| integrity("the act's due instant overflows"))?;
        record.ordinal = ordinal;
        record.act = Some(Act {
            ordinal,
            kind: ActKind::Unload,
            operator: actor,
            hull: rig.hull,
            slot: rig.slot,
            site,
            started_ic: now,
            due_ic,
            reserved_cu: 0,
            params: ActParams::Unload {
                hopper: fixed.hopper,
                hold: fixed.hold,
                goods,
                how,
                excursion,
            },
            by: None,
        });
        record.queued = None;
        write_record(txn, &self.excursions, record, existed)?;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({
                "act": wire_word(ActKind::Unload, false),
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

    /// **The plan's three variants a rig's offer fixes** (doc 20 cut SM.11; the brief's 3.6): the
    /// plan's next move — the interpreter over the durable record, only for a rig whose record
    /// holds a plan live for the actor (no plan, no cost) — and the clear and the strike, each
    /// only of the actor's own plan (a previous lessee's is inert: the lead's pre-commit round,
    /// item 3) and only when `controls` (the actor aboard the rig's hull, for a rig off the
    /// actor's helm; always at the helm, as before).
    #[allow(clippy::too_many_arguments)] // the offer's own reads, passed through
    fn plan_variants(
        &self,
        engine: &mush_core::Engine,
        actor: ObjId,
        rig: &RigAt,
        record: Option<&RigRecord>,
        peeks: &mut Peeks,
        has_battery: bool,
        controls: bool,
    ) -> [(String, FixedPlan); 3] {
        let w: &World = &engine.world;
        let next = record.map_or(Some(1), |record| record.ordinal.checked_add(1));
        let stepped = match (record, next) {
            (Some(record), Some(next)) => match record.live_plan_for(actor) {
                Some(plan) => {
                    let reads = crate::port::site_map::EngineReads(engine);
                    let counts = excursion_counts(w, &|tv| reads.rig(tv), plan.author, &plan.site);
                    match crate::surface::plan::decide(
                        w,
                        &reads,
                        rig,
                        record,
                        &counts,
                        &self.tools,
                        peeks,
                        w.world_ic_now().at,
                    ) {
                        Some(moved) => plan_fixed(w, plan, next, moved, has_battery),
                        None => FixedPlan::refused(Refusal::PlanNone { n: rig.slot }.word()),
                    }
                }
                None => FixedPlan::refused(Refusal::PlanNone { n: rig.slot }.word()),
            },
            _ => FixedPlan::refused(Refusal::PlanNone { n: rig.slot }.word()),
        };
        let (live, ended) = record
            .and_then(|record| record.plan.as_ref())
            .filter(|plan| plan.author == actor)
            .map_or((false, false), |plan| (plan.live(), !plan.live()));
        // The reviewer's R2: a control the record would allow but the actor may not work from
        // where they stand (off the helm and off the rig's hull) is refused `not_aboard`.
        let control = |ok: bool, word: &str| match (ok, controls) {
            (true, true) => FixedPlan {
                ok: true,
                ..FixedPlan::default()
            },
            (true, false) => FixedPlan::refused("not_aboard"),
            (false, _) => FixedPlan::refused(word),
        };
        [
            ("plan".to_owned(), stepped),
            (
                "plan_clear".to_owned(),
                control(live, if ended { "plan_ended" } else { "plan_none" }),
            ),
            (
                "plan_strike".to_owned(),
                control(ended, if live { "plan_live" } else { "plan_none" }),
            ),
        ]
    }

    /// **The offer's filing variant** (doc 20 cut SM.11; the brief's 3.6, the reviewer's F11): the
    /// rig's RAM [`PlanOrder`] for this actor **planned at the record's ordinal and for its next
    /// plan number**, both checked before any compile — a stale order (its filing landed, or the
    /// rig's orders moved on) is `unplanned` at once, with no compile — then [`plan_ask`] re-run
    /// over the engine with the order's list and family and **required equal** to the order's
    /// compile, else `plan_changed`: `go` files exactly the preview or nothing.
    fn fill_fixed(
        &self,
        engine: &mush_core::Engine,
        actor: ObjId,
        rig: &RigAt,
        record: Option<&RigRecord>,
        peeks: &mut Peeks,
    ) -> FixedPlan {
        let w: &World = &engine.world;
        let (ordinal, plans) = record.map_or((0, 0), |record| (record.ordinal, record.plans));
        let Some(order) = plan_order_of(w, rig.tv).filter(|order| {
            order.actor == actor
                && order.ordinal == ordinal
                && plans.checked_add(1) == Some(order.no)
        }) else {
            return FixedPlan::refused(Refusal::Unplanned.word());
        };
        crate::surface::plan::count(w, |c| &c.compiles);
        let reads = crate::port::site_map::EngineReads(engine);
        match plan_ask(
            w,
            &reads,
            actor,
            rig,
            &order.list,
            order.family,
            &self.tools,
            peeks,
            w.world_ic_now().at,
        ) {
            Err(refusal) => FixedPlan::refused(refusal.word()),
            Ok((Ok(compiled), facts)) if compiled == order.compiled => FixedPlan {
                ok: true,
                site: Some(facts.site),
                ordinal: Some(ordinal),
                fill: Some(FillFixed {
                    no: order.no,
                    ordinal,
                    list: compiled.list,
                    family: compiled.family,
                    steps: compiled.steps,
                }),
                ..FixedPlan::default()
            },
            Ok(_) => FixedPlan::refused(Refusal::PlanChanged { n: rig.slot }.word()),
        }
    }

    /// **A filing's prepare** (doc 20 cut SM.11; the brief's 3.6): the offer's filing variant ok
    /// (its word refused otherwise); the record's ordinal the order's (`Conflict` "that rig's orders
    /// moved on"); no plan live for the actor (`plan_filed`); the number the record's next — then
    /// ONE write of the record alone: `plans = no` and the plan, live, its deadline 288 IC hours on,
    /// its cursor 0 (an older ended plan replaced). `now` is the offer's instant.
    fn file_plan(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        (actor, tv, ordinal, now): (ObjId, ObjId, u32, i64),
        rig: &FixedRig,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
        let fixed = rig
            .plans
            .get("plan_file")
            .cloned()
            .ok_or_else(|| invalid("that act is not planned"))?;
        if !fixed.ok {
            return Err(port_error(
                ActionErrorCode::ProviderRejected,
                code_of_word(fixed.code.as_deref().unwrap_or("unplanned")),
            ));
        }
        let fill = fixed
            .fill
            .clone()
            .ok_or_else(|| invalid("a filing's plan names its steps"))?;
        let site = fixed
            .site
            .clone()
            .ok_or_else(|| invalid("an act's plan names its site"))?;
        let existing = read_record(txn, &self.excursions, tv)?;
        let existed = existing.is_some();
        let mut record = existing.unwrap_or_else(|| RigRecord::fresh_unread(tv));
        // A filing writes no act: the invocation's ordinal is the record's (at least 1, the
        // parameter's floor) and the order's.
        if record.ordinal != fill.ordinal || record.ordinal.max(1) != ordinal {
            return Err(port_error(
                ActionErrorCode::Conflict,
                "that rig's orders moved on",
            ));
        }
        if record.live_plan_for(actor).is_some() {
            return Err(port_error(
                ActionErrorCode::ProviderRejected,
                code_of_word("plan_filed"),
            ));
        }
        if record.plans.checked_add(1) != Some(fill.no) {
            return Err(port_error(
                ActionErrorCode::Conflict,
                "that rig's orders moved on",
            ));
        }
        let deadline_ic = now
            .checked_add(PLAN_BOUND_IC_SECS)
            .ok_or_else(|| integrity("the plan's deadline overflows"))?;
        record.plans = fill.no;
        record.plan = Some(FillPlan {
            no: fill.no,
            author: actor,
            hull: rig.hull,
            slot: rig.slot,
            site,
            family: fill.family,
            filed_ic: now,
            deadline_ic,
            list: fill.list,
            steps: fill.steps,
            cursor: 0,
            passed: Vec::new(),
            state: FillState::Live,
            seq: 1,
            tally: PlanTally::default(),
            notes: Vec::new(),
        });
        write_record(txn, &self.excursions, record, existed)?;
        self.control_outcome(invocation, "plan_file", actor, tv, ordinal, now)
    }

    /// **A clear's or a strike's prepare** (doc 20 cut SM.11; the brief's 3.6, D17): the record's
    /// plan of the invocation's number (`Conflict` "the plan moved on" otherwise). A clear: the plan
    /// live (`plan_ended` when it ended by itself between the command and the landing) → ended
    /// `Cleared` now, one `Ended` note, `seq` on — the step in flight untouched. A strike: the plan
    /// ended (`plan_live` otherwise) → struck from the record. One write each.
    fn clear_plan(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        (actor, tv, now): (ObjId, ObjId, i64),
        rig: &FixedRig,
        strike: bool,
        no: Option<u32>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let word = if strike { "plan_strike" } else { "plan_clear" };
        // The offer names the rig the actor's, and its variant must be offered ok: the reviewer's
        // R2 found the variant's `controls` (the author aboard the rig's hull, for a rig off their
        // helm) fixed by the offer and never read here, so an author who had left the hull could
        // still land a clear typed aboard. The variant's other answer (live or ended) is the
        // record's, re-read below: the record is the authority.
        match rig.plans.get(word) {
            None => return Err(invalid("that act is not planned")),
            Some(fixed) if !fixed.ok => {
                return Err(port_error(
                    ActionErrorCode::ProviderRejected,
                    code_of_word(fixed.code.as_deref().unwrap_or("plan_none")),
                ));
            }
            Some(_) => {}
        }
        let Some(mut record) = read_record(txn, &self.excursions, tv)? else {
            return Err(moved_on());
        };
        // Only the actor's own plan (the lead's pre-commit round, item 3: a rig off the helm is
        // offered to its plan's author; a previous lessee's plan is never cleared by another).
        let Some(plan) = record
            .plan
            .as_mut()
            .filter(|plan| Some(plan.no) == no && plan.author == actor)
        else {
            return Err(moved_on());
        };
        if strike {
            if plan.live() {
                return Err(port_error(
                    ActionErrorCode::ProviderRejected,
                    code_of_word("plan_live"),
                ));
            }
            record.plan = None;
        } else {
            if !plan.live() {
                return Err(port_error(
                    ActionErrorCode::ProviderRejected,
                    code_of_word("plan_ended"),
                ));
            }
            plan.passed.clear();
            plan.state = FillState::Ended {
                why: PlanEnd::Cleared,
                at_ic: now,
            };
            push_note(
                plan,
                PlanNoteKind::Ended {
                    why: PlanEnd::Cleared,
                },
                now,
            )?;
            plan.seq = plan
                .seq
                .checked_add(1)
                .ok_or_else(|| port_error(ActionErrorCode::Integrity, "a plan's seq overflows"))?;
        }
        let ordinal = record.ordinal.max(1);
        write_record(txn, &self.excursions, record, true)?;
        self.control_outcome(invocation, word, actor, tv, ordinal, now)
    }

    /// **A plan's end's prepare** (doc 20 cut SM.11; the brief's 3.6): the offer's plan variant an
    /// end (`word == plan_end`) bound to the start's plan and step and the record's live plan's
    /// `seq` (`Conflict` "the plan moved on" otherwise), nothing in flight — then the plan's update
    /// and its end, one `Ended` note, `seq` on. No act; a `Stranded` end writes nothing else.
    fn end_plan(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        (actor, tv, now): (ObjId, ObjId, i64),
        rig: &FixedRig,
        by: &ByParams,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let fixed = rig
            .plans
            .get("plan")
            .cloned()
            .ok_or_else(|| invalid("that act is not planned"))?;
        if !fixed.ok {
            return Err(port_error(
                ActionErrorCode::ProviderRejected,
                code_of_word(fixed.code.as_deref().unwrap_or("plan_moved")),
            ));
        }
        let binding = fixed.plan.ok_or_else(moved_on)?;
        let Some(mut record) = read_record(txn, &self.excursions, tv)? else {
            return Err(moved_on());
        };
        let live = record.live_plan_for(actor).map(|plan| (plan.no, plan.seq));
        if binding.word != "plan_end"
            || binding.end.is_none()
            || Some(binding.no) != by.plan
            || Some(binding.step) != by.step
            || live != Some((binding.no, binding.seq))
        {
            return Err(moved_on());
        }
        // An act or a run in flight holds the end (the step finishes by its own rules); a queued
        // unload does not — the interpreter answers an end with one standing only when the plan's
        // bound has passed or the queue is orphaned (the lead's pre-commit round, item 3), and the
        // queue is SM.8's own, which lands by itself whether the plan lives or not.
        if record.act.is_some() || record.run.is_some() {
            return Err(port_error(ActionErrorCode::Conflict, "that rig is busy"));
        }
        apply_update(&mut record, &binding, now)?;
        let ordinal = record.ordinal.max(1);
        write_record(txn, &self.excursions, record, true)?;
        self.control_outcome(invocation, "plan_end", actor, tv, ordinal, now)
    }

    /// The start's outcome for a plan's control (no act: `due_ic` is `now`).
    fn control_outcome(
        &self,
        invocation: &ActionInvocation,
        word: &str,
        actor: ObjId,
        tv: ObjId,
        ordinal: u32,
        now: i64,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({
                "act": word,
                "actor": actor.0,
                "due_ic": now,
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

    /// **`+tv/mine stop`'s prepare** (doc 20 cut SM.7; the brief's 3.4, decision R7): the record's
    /// run of the invocation's ordinal, the actor's (`Conflict` "that rig is not drilling"
    /// otherwise), marked to close after the cycle in flight — one write; already marked, nothing
    /// changes. No ordinal advance; never a refund.
    fn stop_run(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        (actor, tv, ordinal): (ObjId, ObjId, u32),
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let not_drilling = || port_error(ActionErrorCode::Conflict, "that rig is not drilling");
        let Some(mut record) = read_record(txn, &self.excursions, tv)? else {
            return Err(not_drilling());
        };
        let Some((due_ic, stopping)) = record
            .run
            .as_ref()
            .filter(|run| run.ordinal == ordinal && run.operator == actor)
            .map(|run| (run.due_ic, run.stop))
        else {
            return Err(not_drilling());
        };
        if !stopping {
            if let Some(run) = record.run.as_mut() {
                run.stop = true;
            }
            write_record(txn, &self.excursions, record, true)?;
        }
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({
                "act": "mine_stop",
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
    /// Doc 20 cut SM.7: the pocket cursors (`tacsim.pockets`) a run locks and draws.
    pub pockets: StateFamilyHandle<mush_industry::OreDepositId, PocketCursor>,
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

/// **How a reading was made, from who set its act going** (doc 20 cut SM.11; lead ruling 4): a
/// plan's step writes `via: Plan`; the hand (and the idle return, which reads nothing) `Hand`.
fn via_of(by: Option<ActBy>) -> chart::Via {
    match by {
        Some(ActBy::Plan { .. }) => chart::Via::Plan,
        Some(ActBy::Idle) | None => chart::Via::Hand,
    }
}

/// **The plan's tally, counted at a finish** (doc 20 cut SM.11; the brief's 3.7, the reviewer's
/// F9): only on the record's plan whose number `by` names — riding the record write the finish
/// already makes (no extra write, no `seq` bump).
fn tally(record: &mut RigRecord, by: Option<ActBy>, count: impl FnOnce(&mut PlanTally)) {
    let Some(ActBy::Plan { no, .. }) = by else {
        return;
    };
    if let Some(plan) = record.plan.as_mut().filter(|plan| plan.no == no) {
        count(&mut plan.tally);
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
        ActParams::None {}
        | ActParams::Dump { .. }
        | ActParams::Drive { .. }
        | ActParams::Unload { .. } => {
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
            write_scan(&mut row, *cell, fields, *footing, at, via_of(act.by));
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
            write_sound(&mut row, *cell, &pockets, *quiet, at, via_of(act.by));
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
        ActParams::None {}
        | ActParams::Dump { .. }
        | ActParams::Drive { .. }
        | ActParams::Unload { .. } => {
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
        cycle_param(parameters)?;
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
        let now = w.world_ic_now();
        let reads = crate::port::site_map::EngineReads(engine);
        for record in &records {
            // Doc 20 cut SM.7 (§8 "rights are per act"; the brief's 3.7): a rig drilling has the
            // rights of its run's next cycle read here, at landing — the licence's standing, the
            // block's tenure, the lease.
            let rights = engine
                .action_state()
                .get(&self.excursions, &RigKey(record.tv))
                .ok()
                .flatten()
                .and_then(|found| found.run.clone())
                .map(|run| RightsVerdict {
                    licence: licence_verdict(w, actor, now.at),
                    // The SM.7 lead's pre-commit round, item 3: a read that fails is `Dark`, never
                    // `Claimed` — the run ends with the dark line, not a false claim.
                    tenure: match site_def_of(w, &run.site) {
                        None => TenureRead::Dark {
                            why: DarkCause::Site,
                        },
                        Some(site) => {
                            match mine_tenure(
                                w,
                                &reads,
                                actor,
                                (&run.site, &site),
                                run.letter,
                                &now,
                            ) {
                                Ok(()) => TenureRead::Allowed,
                                Err(TenureBar::Claimed) => TenureRead::Claimed,
                                Err(TenureBar::Dark(why)) => TenureRead::Dark { why },
                            }
                        }
                    },
                    lease: lease_gate(w, record).map_err(|bar| bar == LeaseBar::Forfeit),
                });
            // Doc 20 cut SM.8: the hopper (a recover queues its unload from it) and the hull's
            // hold (an unload's finish re-checks against its plan).
            let hopper = crate::port::vehicle::hopper_of(w, record.tv).and_then(|vault| {
                tv::hopper_goods_of(w, record.tv).map(|(goods, _)| HopperFacts {
                    vault: vault.0,
                    goods,
                })
            });
            let hold = hull_view(w, record.hull).and_then(|view| {
                let vault = crate::port::ship::hold_vault(w, &view)?;
                let plan = crate::port::market::hold_plan(&view)?;
                Some(HoldFacts {
                    vault: vault.0,
                    plan,
                })
            });
            rigs.insert(
                record.tv.0.to_string(),
                DoneRig {
                    battery_id: rig_battery_id(w, record.tv),
                    facts: facts_of(w, record.tv, record.hull),
                    spoil: spoil_of(w, record.hull),
                    failed: crate::surface::tick::finish_retried(w, record.tv),
                    rights,
                    hopper,
                    hold,
                },
            );
        }
        let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
        // Doc 20 cut SM.8: the IC clock stamp and the real second beside `now` (the market lot
        // offer's precedent) — an unload's marked transfer stamps its markers with them.
        let fixed = json!({
            "now": w.world_ic_now().at,
            "clock": w.world_ic_now(),
            "wall": w.now(),
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
            // Doc 20 cut SM.7 (the brief's 3.7): a run's cycle; absent for an act's finish.
            parameter(
                "cycle",
                false,
                ProjectedParameterKind::Integer {
                    minimum: 1,
                    maximum: i64::from(crate::surface::excursion::MAX_RUN_CYCLES),
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
        // Doc 20 cut SM.7 (the brief's 3.7): a run's cycle finishes on its own path.
        if let Some(k) = cycle_param(parameters)? {
            return self.finish_cycle(invocation, txn, (actor, tv, ordinal, k, now), rig);
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
        let mut arrived: Option<[i64; 2]> = None;
        let mut stays: BTreeMap<CanonicalId, u64> = BTreeMap::new();
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
                    // Doc 20 cut SM.8 (lead ruling 1, decision R1): the recover's `Done` files the
                    // unload when the hopper holds ore — a durable queue on the record the surface
                    // tick starts as the operator (a provider never submits). A hopper that will
                    // not read queues nothing (the departure's `HOPPER_LOADED` then names the hand
                    // unload).
                    let loaded = match &rig.hopper {
                        Some(hopper) => {
                            let mut any = false;
                            for good in &hopper.goods {
                                match self.inventory.pool(
                                    txn,
                                    EndpointRef(VaultId(hopper.vault)),
                                    good,
                                ) {
                                    Ok(found) => {
                                        any |= found.is_some_and(|pool| pool.available > 0);
                                    }
                                    Err(error) => {
                                        tracing::warn!(tv = tv.0, %error, "tacsim: a recover's finish could not read the hopper; no unload is queued");
                                        any = false;
                                        break;
                                    }
                                }
                            }
                            any
                        }
                        None => false,
                    };
                    if loaded {
                        record.queued = Some(Queued {
                            kind: ActKind::Unload,
                            operator: act.operator,
                            hull: act.hull,
                            slot: act.slot,
                            site: act.site.clone(),
                            after: ordinal,
                            at_ic: act.due_ic,
                            excursion: record.excursion.as_ref().map(|open| open.id),
                        });
                        // Doc 20 cut SM.11 (the reviewer's F9): a plan's recover that brought ore
                        // home is a trip; an empty one is not.
                        tally(&mut record, act.by, |tally| {
                            tally.trips = tally.trips.saturating_add(1);
                        });
                    }
                    record.excursion = None;
                    // Doc 20 cut SM.4b (§3.6 of the brief): a recover that drove home spends its
                    // leg's charge off the record's own value — no battery read (§4.1) — and the
                    // bay's recharge starts at its due instant.
                    record.charge = if act.params.leg().is_some() {
                        Charge {
                            value_cu: record.charge.value_cu.saturating_sub(act.reserved_cu),
                            at_ic: act.due_ic,
                            in_bay: true,
                        }
                    } else {
                        settle(&record.charge, true)?
                    };
                    ActResult::Done
                } else {
                    ActResult::Refused {
                        code: RefusedCode::NotBeside,
                    }
                }
            }
            // Doc 20 cut SM.4b (§3.7 of the brief): a drive is `Done` when the rig stands, at
            // landing, on the act's site at the leg's end cell — the tick placed it there at the
            // due instant — and then the charge settles: from the ground the reservation off the
            // value (`settle_spent_out_of_bay`, no battery read: SM.4a's ground rule), from the bay
            // the bay's credit to the end of the launch half and then the leg's cost
            // (`spent_after_launch`). An implied launch whose rig never left the bay is
            // `NeverLeft` (the excursion closes); a from-bay drive moved after leaving keeps its
            // excursion and leaves the bay at the launch half (`left_after_launch`), `Moved`; a
            // ground drive found anywhere but its end is `Moved`, nothing spent.
            ActKind::Drive => {
                let from_bay = act.params.launches();
                let to = act.params.leg().map(|(_, to)| to);
                let arrived_there = matches!(
                    &rig.facts,
                    Facts::Ground { site: Some(site), cell: Some(cell), .. }
                        if *site == act.site && Some(*cell) == to
                );
                if arrived_there {
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
                        settle_spent_out_of_bay(&record.charge, act.reserved_cu, act.due_ic)
                            .ok_or_else(|| {
                                port_error(
                                    ActionErrorCode::Integrity,
                                    "the act spent more than the charge",
                                )
                            })?
                    };
                    arrived = to;
                    ActResult::Done
                } else if from_bay && matches!(&rig.facts, Facts::Bay { hull } if *hull == act.hull)
                {
                    record.excursion = None;
                    ActResult::Refused {
                        code: RefusedCode::NeverLeft,
                    }
                } else {
                    if from_bay && !matches!(&rig.facts, Facts::Bay { .. }) {
                        record.charge = left_after_launch(&record.charge, &act, battery);
                    }
                    ActResult::Refused {
                        code: RefusedCode::Moved,
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
                } else if rig.failed.contains(&(act.ordinal, 0)) {
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
            // Doc 20 cut SM.7: a run never sits in the act slot (`RigRecord::check`); its cycles
            // finish by their own path.
            ActKind::Mine => {
                return Err(port_error(
                    ActionErrorCode::Integrity,
                    "a run finishes by its cycle",
                ));
            }
            // Doc 20 cut SM.8 (§7.1; the brief's 3.8).
            ActKind::Unload => {
                let (result, done_moved, done_stays) =
                    self.finish_unload(txn, (tv, ordinal), &act, rig, parameters)?;
                // The unload spends nothing; in the bay the charge recharges through it (SM.3b's
                // lazy charge: with the battery `spent` of a zero reservation, so the bay credits
                // the whole act; without it `settle_free`, unchanged in the bay — §4.1 "never
                // stranded by its battery"). A rig a staff move took out of the bay settles as it
                // stands; leaving the bay with no battery row reads no credit (the stored value
                // stands, `left_after_launch`'s rule).
                let in_bay = matches!(&rig.facts, Facts::Bay { .. });
                record.charge = settle(&record.charge, in_bay).unwrap_or(record.charge);
                // Doc 20 cut SM.11 (the lead's pre-commit round, item 1): a QUEUED unload the
                // hopper's full marker ring refused keeps its queue — re-filed after this unload's
                // ordinal, so the surface tick starts it again under its attempt rule once the
                // ring frees (the start's offer refuses `unload_markers` until then, so nothing
                // lands and nothing more is told). A wait with the record's back-off, never a
                // park: a plan waiting on it resumes by itself. A hand unload's refusal stays
                // "try again later".
                if result
                    == (ActResult::Refused {
                        code: RefusedCode::MarkersFull,
                    })
                    && let ActParams::Unload {
                        how: UnloadHow::Queued,
                        excursion,
                        ..
                    } = &act.params
                {
                    record.queued = Some(Queued {
                        kind: ActKind::Unload,
                        operator: act.operator,
                        hull: act.hull,
                        slot: act.slot,
                        site: act.site.clone(),
                        after: ordinal,
                        at_ic: act.due_ic,
                        excursion: *excursion,
                    });
                }
                moved = done_moved;
                stays = done_stays;
                result
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
            arrived,
            mined: None,
            stays,
            by: act.by,
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

impl SurfaceDoneProvider {
    /// **An unload's finish** (doc 20 §7.1; cut SM.8, the brief's 3.8; lead rulings 2 and 3,
    /// decisions R3 and R8), decided before anything is staged, then at most ONE inventory call:
    ///
    /// 1. **in the bay** — the rig in its act's hull's bay at landing, else `Refused { LeftBay }`;
    /// 2. **the hopper** — each frozen good's `available`, read here, at least its quantity, else
    ///    `Refused { HopperChanged }`;
    /// 3. **the hold** — the hull's hold facts present with the frozen vault, the plan's goods
    ///    table and its one ore compartment, the compartment admitting every frozen good
    ///    cumulatively on top of the hold's pools read here (`capacity::admits`), and no earlier
    ///    finish of this unload refused mid-bundle (`DoneRig::failed` holding `(ordinal, 0)`: the
    ///    dump's `SpoilGone` precedent) — any failing → `Refused { HoldRefused }`, nothing staged,
    ///    paged (staff state only: §6's gate keeps an honest fill inside the hold's room);
    /// 4. **the move** — ONE `InventoryFamilies::transfer_many_marked` hopper → hold, one
    ///    `inventory.transferred` marker a good under `crate::surface::unload::marker`'s id, stamped
    ///    at the act's due on the offer's IC clock; the hopper's marker ring full of young markers
    ///    (`MarkerRingFull`, decided there before anything is staged) is `Refused { MarkersFull }`
    ///    (the fix round's correctness/F2: never told as a hold that would not take the load);
    ///    any other refusal there refuses the whole bundle (nothing is written) and the next
    ///    finish records `HoldRefused` through `failed`.
    ///
    /// Returns the result, what moved and — for a `Done` queued or whole unload — what stayed:
    /// each hopper good's availability before the move less what moved, when positive. A queued
    /// unload with nothing to move (the hold had no room) is `Done` with every good staying.
    #[allow(clippy::type_complexity)]
    fn finish_unload(
        &self,
        txn: &mut ActionTxn<'_>,
        (tv, ordinal): (ObjId, u32),
        act: &Act,
        rig: &DoneRig,
        parameters: &Map<String, Value>,
    ) -> Result<
        (
            ActResult,
            BTreeMap<CanonicalId, u64>,
            BTreeMap<CanonicalId, u64>,
        ),
        ActionError,
    > {
        let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
        let refused = |code: RefusedCode| {
            Ok((
                ActResult::Refused { code },
                BTreeMap::new(),
                BTreeMap::new(),
            ))
        };
        let ActParams::Unload {
            hopper,
            hold,
            goods,
            how,
            excursion,
        } = &act.params
        else {
            return Err(integrity("an unload carries its frozen goods"));
        };
        // 1. In the bay.
        if !matches!(&rig.facts, Facts::Bay { hull } if *hull == act.hull) {
            return refused(RefusedCode::LeftBay);
        }
        // 2. The hopper: every good's availability now (the frozen ones must hold).
        let source = EndpointRef(VaultId(*hopper));
        let mut accepted: Vec<CanonicalId> = rig
            .hopper
            .as_ref()
            .filter(|facts| facts.vault == *hopper)
            .map(|facts| facts.goods.clone())
            .unwrap_or_default();
        for good in goods.keys() {
            if !accepted.contains(good) {
                accepted.push(good.clone());
            }
        }
        if accepted.len() > mush_inventory::MAX_POOLS_PER_ENDPOINT {
            return Err(invalid(
                "a finish's offer fixes a bounded set of hopper goods",
            ));
        }
        let mut before: BTreeMap<CanonicalId, u64> = BTreeMap::new();
        for good in &accepted {
            let available = self
                .inventory
                .pool(txn, source, good)
                .map_err(|_| integrity("the hopper cannot be read"))?
                .map_or(0, |pool| pool.available);
            before.insert(good.clone(), available);
        }
        if goods
            .iter()
            .any(|(good, qty)| before.get(good).copied().unwrap_or(0) < *qty)
        {
            return refused(RefusedCode::HopperChanged);
        }
        if !goods.is_empty() {
            // 3. The hold, before anything is staged.
            let destination = EndpointRef(VaultId(*hold));
            let why = match rig.hold.as_ref() {
                None => Some("the hull's hold cannot be read"),
                Some(facts) if facts.vault != *hold => {
                    Some("the hull's hold is no longer that vault")
                }
                Some(_) if rig.failed.contains(&(act.ordinal, 0)) => {
                    Some("an earlier finish of this unload refused mid-transfer")
                }
                Some(facts) => {
                    match crate::port::market::plan_goods(&facts.plan).and_then(|table| {
                        let ore = CanonicalId::new("metal_ore").ok()?;
                        crate::port::capacity::compartment_for(
                            table,
                            &crate::port::market::plan_compartments(&facts.plan),
                            &ore,
                        )
                        .map(|compartment| (table, compartment))
                    }) {
                        None => Some("the hold's plan cannot be read"),
                        Some((table, compartment)) => {
                            let mut contents: BTreeMap<CanonicalId, u64> = BTreeMap::new();
                            let mut unread = false;
                            for good in table
                                .goods
                                .iter()
                                .filter(|good| compartment.accepts.contains(&good.stowage))
                            {
                                match self.inventory.pool(txn, destination, &good.id) {
                                    Ok(found) => {
                                        let held = found.map_or(0, |pool| pool.quantity);
                                        if held > 0 {
                                            contents.insert(good.id.clone(), held);
                                        }
                                    }
                                    Err(_) => unread = true,
                                }
                            }
                            let mut admitted = !unread;
                            for (good, qty) in goods {
                                if !admitted {
                                    break;
                                }
                                match crate::port::capacity::admits(
                                    table,
                                    &compartment,
                                    &contents,
                                    (good, *qty),
                                ) {
                                    crate::port::capacity::Admission::Admitted(_) => {
                                        let slot = contents.entry(good.clone()).or_insert(0);
                                        *slot = slot.saturating_add(*qty);
                                    }
                                    crate::port::capacity::Admission::Refused(_)
                                    | crate::port::capacity::Admission::Error(_) => {
                                        admitted = false
                                    }
                                }
                            }
                            (!admitted).then_some(if unread {
                                "a hold pool will not read"
                            } else {
                                "the hold's ore compartment would not admit the load"
                            })
                        }
                    }
                }
            };
            if let Some(why) = why {
                tracing::error!(
                    tv = tv.0,
                    act = ordinal,
                    hold = *hold,
                    why,
                    "tacsim: an unload's hold would not take the load; the finish records it \
                     refused (HoldRefused) and nothing moved; staff check the hull's hold and the \
                     rig's hopper"
                );
                return refused(RefusedCode::HoldRefused);
            }
            // 4. The move: ONE marked transfer, one marker a good.
            let clock = parameters
                .get("clock")
                .and_then(|value| {
                    serde_json::from_value::<mush_core::ClockStamp>(value.clone()).ok()
                })
                .ok_or_else(|| invalid("a finish's offer fixes the clock"))?;
            let wall = parameters
                .get("wall")
                .and_then(Value::as_i64)
                .ok_or_else(|| invalid("a finish's offer fixes the real second"))?;
            let mut moves: Vec<(CanonicalId, u64, smol_str::SmolStr)> = Vec::new();
            for (good, qty) in goods {
                let id = crate::surface::unload::marker(tv, *excursion, ordinal, good, act.due_ic)
                    .map_err(|_| integrity("an unload's marker did not form"))?;
                moves.push((good.clone(), *qty, id));
            }
            let stamp = mush_core::ClockStamp {
                clock: clock.clock,
                at: act.due_ic,
            };
            let moved = self.inventory.transfer_many_marked(
                txn,
                source,
                destination,
                &moves,
                act.operator,
                &stamp,
                wall,
            );
            if matches!(moved, Err(mush_inventory::InventoryError::MarkerRingFull)) {
                // Decided before anything was staged (`transfer_many_marked`'s contract), so the
                // refusal is this finish's outcome. The ring is the hopper vault's and outlives a
                // lease (the fix round's correctness/F2; the brief's ring-bound residual).
                tracing::error!(
                    tv = tv.0,
                    act = ordinal,
                    hopper = *hopper,
                    "tacsim: an unload's hopper marker ring is full of young markers; the finish \
                     records it refused (MarkersFull) and nothing moved; the ring frees as its \
                     markers pass the retention horizon"
                );
                return refused(RefusedCode::MarkersFull);
            }
            moved.map_err(|error| {
                tracing::warn!(
                    tv = tv.0,
                    act = ordinal,
                    hold = *hold,
                    goods = moves.len(),
                    %error,
                    "tacsim: an unload's transfer into the hold refused; the next finish \
                     records it refused"
                );
                port_error(
                    ActionErrorCode::ProviderRejected,
                    "the hopper could not be unloaded into the hold",
                )
            })?;
        }
        // The outcome: what moved, and for a queued or whole unload what stays.
        let mut stays: BTreeMap<CanonicalId, u64> = BTreeMap::new();
        if matches!(how, UnloadHow::Queued | UnloadHow::Whole) {
            for (good, available) in &before {
                let left = available.saturating_sub(goods.get(good).copied().unwrap_or(0));
                if left > 0 {
                    stays.insert(good.clone(), left);
                }
            }
        }
        Ok((ActResult::Done, goods.clone(), stays))
    }

    /// **One mining cycle's finish** (doc 20 §6 "the completion"; cut SM.7, the brief's 3.7; lead
    /// rulings 1, 5, 6 and 7), **one bundle** (≤ 8 writes, §11's Tick row):
    ///
    /// 1. **this cycle**: the run of the invocation's ordinal at cycle `k`, the actor's, due — else
    ///    `Conflict` "that cycle is already finished" (the replay guard behind the key's HIT);
    /// 2. **at the stand**: the rig on the run's site at its frozen stand cell, else `Moved` —
    ///    nothing moved, nothing spent, the chart untouched, the run closed;
    /// 3. **the yield**: `mine::cycle_yield` over the family's cursor row read here; tonnes to move
    ///    need the block vault to hold them (`available`), else `Unworked` with nothing staged (and
    ///    logged); then ONE `InventoryFamilies::transfer` of the family's ore good from the block
    ///    vault to the hopper — a refusal there refuses the whole bundle, and the tick's next finish
    ///    of the same `(ordinal, k)`, marked `failed`, records `Unworked` with nothing moved (the
    ///    dump's `SpoilGone` precedent);
    /// 4. **the end**: `mine::run_end` — refused, worked out, stopped, the next cycle's rights
    ///    (`DoneRig::rights`, read at landing: the licence lapsing at or before this cycle's due
    ///    instant, the block's tenure, the lease), the run's last cycle — or re-arm;
    /// 5. **the cursor**: the family's row drawn by the yield, its lock kept while the run re-arms
    ///    (`mine::cursor_after`); at a close every other lock row released (`cursor_released`); a
    ///    barren core writes no cursor of its family (lead ruling 2);
    /// 6. **the charge**: `settle_spent_out_of_bay` of one cycle's cost at its due instant — no
    ///    battery read (SM.4b's ground-drive precedent); a refused cycle spends nothing;
    /// 7. **the chart** (a `Done` cycle): the operator's row — created with the run's body when they
    ///    have none — gains the pocket's R2 with `PIT_SEEN` (`chart::write_core`);
    /// 8. **re-arm or close**, and the outcome (`Outcome::mined`). The action's outcome keeps the
    ///    finish's shape: nothing of the class, the tonnes or the deposit.
    ///
    /// The running cycle always yields: rights never undo it (§8).
    #[allow(clippy::too_many_lines)] // one ordered list, then one bundle
    fn finish_cycle(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        (actor, tv, ordinal, k, now): (ObjId, ObjId, u32, u8, i64),
        rig: &DoneRig,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
        let finished = || port_error(ActionErrorCode::Conflict, "that cycle is already finished");
        // 1. This cycle.
        let Some(mut record) = read_record(txn, &self.excursions, tv)? else {
            return Err(finished());
        };
        let Some(mut run) = record
            .run
            .clone()
            .filter(|run| run.ordinal == ordinal && run.k == k)
        else {
            return Err(finished());
        };
        if run.operator != actor {
            return Err(port_error(
                ActionErrorCode::Unauthorized,
                "that run is another pilot's",
            ));
        }
        if run.due_ic > now {
            return Err(port_error(ActionErrorCode::Conflict, "not due"));
        }
        let rights = rig
            .rights
            .clone()
            .ok_or_else(|| integrity("a run's next rights are read at landing"))?;
        // 2. At the stand.
        let at_stand = matches!(
            &rig.facts,
            Facts::Ground { site: Some(site), cell: Some(cell), .. }
                if *site == run.site && *cell == run.stand
        );
        // 3. The yield.
        let own = read_cursor(txn, &self.pockets, run.deposit)?;
        let drawn = own
            .as_ref()
            .and_then(|cursor| cursor.row(run.number))
            .map_or(0, |row| row.drawn_t);
        let mut result = ActResult::Done;
        let mut tonnes = 0_u32;
        if at_stand {
            let y = cycle_yield(run.class_t, run.pocket_t, drawn);
            if y > 0 {
                let source =
                    EndpointRef(VaultId(run.source.ok_or_else(|| {
                        integrity("an ore-bearing run names its block vault")
                    })?));
                let available = self
                    .inventory
                    .pool(txn, source, &run.good)
                    .map_err(|_| integrity("the block's ground cannot be read"))?
                    .map_or(0, |pool| pool.available);
                let retried = rig.failed.contains(&(ordinal, k));
                if available < u64::from(y) || retried {
                    tracing::error!(
                        tv = tv.0,
                        run = ordinal,
                        cycle = k,
                        deposit = %run.deposit,
                        available,
                        wanted = y,
                        retried,
                        runbook = "doc 20 cut SM.7: the cycle is recorded Unworked and nothing \
                                   moved; industry's sweep quarantines the block if its books \
                                   disagree (initial − pool == Σ drawn); staff read the block \
                                   vault's pool and the pocket cursor",
                        "tacsim: a mining cycle's block would not give up its ore"
                    );
                    result = ActResult::Refused {
                        code: RefusedCode::Unworked,
                    };
                } else {
                    self.inventory
                        .transfer(
                            txn,
                            source,
                            EndpointRef(VaultId(run.hopper)),
                            &run.good,
                            u64::from(y),
                        )
                        .map_err(|error| {
                            tracing::warn!(
                                tv = tv.0,
                                run = ordinal,
                                cycle = k,
                                %error,
                                "tacsim: a mining cycle's transfer refused; the next finish \
                                 records it Unworked"
                            );
                            port_error(
                                ActionErrorCode::ProviderRejected,
                                "the block's ore could not be moved to the hopper",
                            )
                        })?;
                    tonnes = y;
                }
            }
        } else {
            result = ActResult::Refused {
                code: RefusedCode::Moved,
            };
        }
        // 4. The end.
        let refused = result != ActResult::Done;
        let drawn_after = drawn.saturating_add(tonnes);
        let worked_out = !refused && u32::from(run.pocket_t).saturating_sub(drawn_after) == 0;
        let end = run_end(
            refused,
            worked_out,
            run.stop,
            run.k == run.cycles,
            &rights,
            run.due_ic,
        );
        let ours = PocketLock { tv, ordinal };
        // 5. The cursor: the family's row drawn and its lock kept or released; at a close every
        // other row this run locked released.
        if !refused {
            let after = cursor_after(
                own.as_ref(),
                run.deposit,
                run.number,
                tonnes,
                &ours,
                end.is_none(),
            )
            .ok_or_else(|| integrity("the pocket's cursor cannot take the draw"))?;
            // A barren core of a family with no row writes nothing (lead ruling 2); nor does any
            // cycle that leaves the cursor as it stood.
            if after != own {
                write_cursor(txn, &self.pockets, run.deposit, own.is_some(), after)?;
            }
        }
        if end.is_some() {
            for row in &run.lock_rows {
                if !refused && row.deposit == run.deposit {
                    continue;
                }
                let before = read_cursor(txn, &self.pockets, row.deposit)?;
                if let Some(before) = before {
                    let after = cursor_released(&before, row.pocket, &ours);
                    if after.as_ref() != Some(&before) {
                        write_cursor(txn, &self.pockets, row.deposit, true, after)?;
                    }
                }
            }
        }
        // 6. The charge, and 7. the chart (a `Done` cycle).
        if !refused {
            record.charge = settle_spent_out_of_bay(&record.charge, run.cost_cu, run.due_ic)
                .ok_or_else(|| integrity("the cycle spent more than the charge"))?;
            run.reserved_cu = run
                .reserved_cu
                .checked_sub(run.cost_cu)
                .ok_or_else(|| integrity("the run's reservation is short of its cycle"))?;
            let existing = read_chart(txn, &self.charts, run.operator, &run.site)?;
            let existed = existing.is_some();
            let mut row = existing.unwrap_or_else(|| {
                ChartRecord::new(run.operator, run.site.clone(), run.body.clone())
            });
            chart::write_core(
                &mut row,
                (run.pocket[0], run.pocket[1]),
                run.due_ic,
                via_of(run.by),
            );
            row.check()
                .map_err(|_| integrity("the chart row is out of bounds"))?;
            let key = row.key();
            let staged = if existed {
                txn.replace(&self.charts, &key, row)
            } else {
                txn.insert(&self.charts, &key, row)
            };
            staged.map_err(|_| integrity("the chart row did not stage"))?;
        }
        // 8. Re-arm or close, and the outcome.
        let mined = Mined {
            k: run.k,
            cycles: run.cycles,
            pocket: run.pocket,
            good: run.good.clone(),
            tonnes,
            blind: run.blind,
            end,
        };
        let at_ic = run.due_ic;
        let (slot, site) = (run.slot, run.site.clone());
        // Doc 20 cut SM.11 (the brief's 3.7): a plan's cycle counts its tonnes and itself.
        let run_by = run.by;
        if !refused {
            tally(&mut record, run_by, |tally| {
                tally.tonnes = tally.tonnes.saturating_add(tonnes);
                tally.cycles = tally.cycles.saturating_add(1);
            });
        }
        record.run = match end {
            Some(_) => None,
            None => {
                run.k = run
                    .k
                    .checked_add(1)
                    .ok_or_else(|| integrity("a run's cycle overflows"))?;
                run.due_ic = run
                    .due_ic
                    .checked_add(i64::from(run.secs))
                    .ok_or_else(|| integrity("a cycle's due instant overflows"))?;
                Some(run)
            }
        };
        record.last = Some(Outcome {
            ordinal,
            kind: ActKind::Mine,
            operator: actor,
            slot,
            site,
            at_ic,
            result,
            moved: BTreeMap::new(),
            span: None,
            arrived: None,
            mined: Some(mined),
            stays: BTreeMap::new(),
            by: run_by,
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
        // Doc 20 cut SM.11: one sample of every variant, shared with the plan's refusal table.
        let refusals = refusal_samples();
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
        // Doc 20 cut SM.4b: 24 → 33 words; the drive's lines verbatim. Doc 20 cut SM.7: 33 → 49
        // (mining's sixteen). Doc 20 cut SM.8: 49 → 55 (unloading's six), 55 → 56 at the lead's
        // pre-commit round (item 4: `unload_no_stock` — a named tonnage past the hopper's stock is
        // no longer told as not fitting the hold). Doc 20 cut SM.11: 56 → 63 (the fill plan's
        // seven prepare refusals); 63 → 64 at the lead's pre-commit round (item 1:
        // `unload_markers`, the queued unload's start while the hopper's unload log is full).
        assert_eq!(REFUSAL_CODES.len(), 64);
        assert_eq!(
            Refusal::PlanLive { n: 1, no: 2 }.code(),
            "that rig is working a plan"
        );
        assert_eq!(
            Refusal::UnloadNoFit {
                n: 1,
                q: 9,
                fit: 3,
                ore: "Metal ore".to_owned(),
            }
            .line(),
            "9 t do not fit: rig 1 can unload 3 t of Metal ore now — +tv/1 unload 3, or +tv/1 \
             unload with no number."
        );
        assert_eq!(
            Refusal::UnloadNoStock {
                n: 1,
                fit: 4,
                ore: "ice ore".to_owned(),
            }
            .line(),
            "Rig 1's hopper holds only 4 t of ice ore: +tv/1 unload 4, or +tv/1 unload with no \
             number."
        );
        assert_eq!(
            Refusal::UnloadNoStock {
                n: 1,
                fit: 4,
                ore: String::new(),
            }
            .code(),
            "the hopper holds less than that"
        );
        assert_eq!(
            Refusal::UnloadChanged { n: 2 }.code(),
            "the hopper or the hold's room changed; preview it again"
        );
        assert_eq!(
            Refusal::NoFit {
                n: 1,
                m: 4,
                fit: 3,
                bound: Bound::Hopper,
                fam: String::new(),
            }
            .line(),
            "4 cycles do not fit: the hopper is full after the third — +tv/1 mine 3, or +tv/1 \
             mine with no number."
        );
        assert_eq!(Refusal::Claimed.line(), "That block is under claim.");
        assert_eq!(
            Refusal::Locked.line(),
            "Another rig is working that pocket."
        );
        assert_eq!(
            Refusal::NoWayHome {
                n: 1,
                target: "C117".to_owned(),
                ramp: "stern ramp".to_owned()
            }
            .line(),
            "Rig 1 could not get back to the stern ramp from C117, so it stays where it is."
        );
        assert_eq!(
            Refusal::Winding {
                n: 1,
                target: "field C-NW".to_owned(),
                needed: 33
            }
            .line(),
            "The way to field C-NW winds too much to drive in one leg (32 waypoints at most). \
             Drive to a nearer point first."
        );
        // Doc 20 cut SM.7 (reviewer F7): the shared word speaks of an act, never a drive.
        assert_eq!(
            Refusal::Unplanned.code(),
            "that act was not planned; preview it again"
        );
        assert_eq!(
            Refusal::Moved.code(),
            "that rig has moved since the drive was planned"
        );
        assert_eq!(
            Refusal::LegShort {
                n: 2,
                now: "2".to_owned(),
                need: "3".to_owned(),
                ramp: "stern ramp".to_owned()
            }
            .line(),
            "Rig 2 has 2 % charge and the drive to the stern ramp needs 3 %; it cannot come in \
             on its own."
        );
    }

    /// **The start's `act` words parse** (doc 20 cut SM.4a, D23; the brief's reviewer finding
    /// F6): every wire word round-trips through `act_of_word`/`wire_word`; `sound_quiet` is a
    /// quiet sounding; `probe` (the retired test kind) and `survey` (the verb's alias only) do
    /// not parse; a sounding's quiet flag is spelled, every other kind's ignored.
    #[test]
    fn sm4a_the_wire_words_parse() {
        // Doc 20 cut SM.7: `mine_stop` is a control, no act (`sm7_the_wire_words_parse`).
        for word in ACT_WIRE_WORDS
            .into_iter()
            .filter(|word| *word != "mine_stop" && !word.starts_with("plan_"))
        {
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
            refused.to_string().contains(
                "a surface act names launch, recover, dump, scan, sound, sound_quiet, drive, \
                 mine, mine_stop, unload, plan_file, plan_clear, plan_strike or plan_end"
            ),
            "{refused}"
        );
        assert_eq!(
            act_param(&parameters("sound_quiet")).ok(),
            Some(StartWord::Act {
                kind: ActKind::Sound,
                quiet: true
            })
        );
    }

    /// **Tenure reads the block, not the family** (doc 20 cut SM.7; the brief's 3.3 item 5,
    /// reviewer F5, decisions R5 and R20) — `authored_of` over a synthetic `SiteDef` and
    /// `tenure_verdict` over hand-built `ClaimRecord`s, with **industry's own
    /// `extraction_authorized`** as the authorizer: on a bare `TestWorld` with no licence
    /// instrument named, its tenure half passing reads `Err(NoLicenceInstrument)` (the licence half
    /// is the world tests'), so the holder rule here is industry's, not a model of it. Commons with a
    /// claim → Ok (no claim read); `(Held, None)` → open → Ok; `(Held, Some)` with no claim →
    /// `Claimed`; open with a live claim of another → `Claimed`; open with a term-expired unlapsed
    /// claim → Ok; open with a lapsed claim → Ok; open with an uncomparable term → `Claimed`; the
    /// holder of a `Suspended` claim → `Claimed`; the holder of an uncontested in-term claim → Ok.
    #[test]
    fn sm7_mine_tenure_reads_the_block_not_the_family() {
        use crate::port::def::{BlockDef, SiteDef, SiteTenureDef, Tenure, Zone};
        use mush_core::{ClockRef, ClockStamp};
        use mush_industry::{
            Accessibility, BandId, ClaimId, ClaimRecord, ClaimSubject, ContestState, CurvePoint,
            Exclusivity, ExtractionRefusal, OreDeposit, OreDepositId, Transferability, YieldCurve,
        };
        use mush_org::PrincipalId;
        let cid = |text: &str| CanonicalId::new(text).unwrap_or_else(|_| panic!("{text}"));
        let block = |tenure: Option<Tenure>, holder: Option<&str>| BlockDef {
            tenure,
            holder: holder.map(cid),
            ..BlockDef::default()
        };
        let site = SiteDef {
            name: "Test Flank".to_owned(),
            issuer: cid("freeport_registry"),
            zone: Zone::Core,
            tenure: SiteTenureDef {
                default: Tenure::Open,
            },
            blocks: [
                ("A".to_owned(), block(Some(Tenure::Commons), None)),
                ("F".to_owned(), block(Some(Tenure::Held), Some("occator"))),
                ("G".to_owned(), block(Some(Tenure::Held), None)),
            ]
            .into_iter()
            .collect(),
            papers: std::collections::BTreeMap::new(),
        };
        assert_eq!(authored_of(&site, 'A'), Authored::Commons);
        assert_eq!(authored_of(&site, 'F'), Authored::Held);
        assert_eq!(
            authored_of(&site, 'G'),
            Authored::Open,
            "a holder-less held block"
        );
        assert_eq!(authored_of(&site, 'C'), Authored::Open, "the default");
        let stamp = |at: i64, generation: u64| ClockStamp {
            clock: ClockRef::WorldIc { generation },
            at,
        };
        let now = stamp(1_000, 1);
        let me = ObjId(42);
        let other = ObjId(43);
        let body = OreDeposit::new(
            OreDepositId(7),
            ObjId(7),
            cid("metal_ore"),
            mush_inventory::Grade::new(0).unwrap_or_else(|_| panic!("a grade")),
            Accessibility::new(100).unwrap_or_else(|_| panic!("accessible")),
            BandId::new("surface").unwrap_or_else(|_| panic!("a band")),
            BandId::new("surface").unwrap_or_else(|_| panic!("a band")),
            YieldCurve::new(vec![CurvePoint {
                cumulative: 0,
                per_cycle: 1,
            }])
            .unwrap_or_else(|_| panic!("a curve")),
            mush_inventory::EndpointRef(VaultId(9)),
            40,
        )
        .unwrap_or_else(|_| panic!("a deposit"));
        let claim = |id: u64, holder: ObjId, term: ClockStamp| {
            ClaimRecord::new(
                ClaimId(id),
                ClaimSubject::Deposit {
                    deposit: OreDepositId(7),
                },
                PrincipalId::Object(ObjId(1)),
                PrincipalId::Object(holder),
                ObjId(900),
                Exclusivity::Exclusive,
                1,
                term,
                Transferability::Transferable,
            )
            .unwrap_or_else(|_| panic!("a claim"))
        };
        let world = mush_testkit::TestWorld::new();
        let authorizes = |claim: &ClaimRecord| {
            matches!(
                mush_industry::extraction_authorized(
                    world.world(),
                    me,
                    &PrincipalId::Object(me),
                    &body,
                    body.initial_reserve(),
                    std::slice::from_ref(claim),
                    None,
                    &now,
                ),
                Ok(_) | Err(ExtractionRefusal::NoLicenceInstrument)
            )
        };
        let verdict = |authored: Authored, claims: &[ClaimRecord]| {
            tenure_verdict(authored, claims, &now, &authorizes)
        };
        let live_other = claim(1, other, stamp(5_000, 1));
        assert_eq!(
            verdict(Authored::Commons, std::slice::from_ref(&live_other)),
            Ok(())
        );
        assert_eq!(verdict(Authored::Open, &[]), Ok(()));
        assert_eq!(verdict(Authored::Held, &[]), Err(TenureBar::Claimed));
        assert_eq!(
            verdict(Authored::Open, std::slice::from_ref(&live_other)),
            Err(TenureBar::Claimed)
        );
        let expired = claim(2, other, stamp(500, 1));
        assert!(!claim_live(&expired, &now));
        assert_eq!(
            verdict(Authored::Open, &[expired]),
            Ok(()),
            "term past, not lapsed"
        );
        let lapsed = claim(3, other, stamp(5_000, 1))
            .lapsed(stamp(900, 1))
            .unwrap_or_else(|_| panic!("lapses"));
        assert_eq!(verdict(Authored::Open, &[lapsed]), Ok(()));
        let uncomparable = claim(4, other, stamp(5_000, 2));
        assert!(
            claim_live(&uncomparable, &now),
            "a claim we cannot date bars"
        );
        assert_eq!(
            verdict(Authored::Open, &[uncomparable]),
            Err(TenureBar::Claimed)
        );
        let suspended = claim(5, me, stamp(5_000, 1))
            .with_contest_state(ContestState::Suspended {
                by: PrincipalId::Object(ObjId(1)),
            })
            .unwrap_or_else(|_| panic!("suspends"));
        assert_eq!(
            verdict(Authored::Open, &[suspended]),
            Err(TenureBar::Claimed),
            "a suspended claim bars its holder too"
        );
        let mine = claim(6, me, stamp(5_000, 1));
        assert_eq!(verdict(Authored::Open, std::slice::from_ref(&mine)), Ok(()));
        assert_eq!(
            verdict(Authored::Held, &[mine]),
            Ok(()),
            "a holder's own claim"
        );
    }

    /// **The start's nine words parse** (doc 20 cut SM.7; the brief's 3.4): `mine` is an act (the
    /// `Mine` kind), `mine_stop` a [`StartWord::MineStop`]; every other word's [`start_word`] is its
    /// [`act_of_word`]; the Choice is sorted and holds nine; the cycle and stop keys form.
    #[test]
    fn sm7_the_wire_words_parse() {
        // Doc 20 cut SM.8: nine → ten with `unload`; doc 20 cut SM.11: ten → fourteen with the
        // plan's four controls.
        assert_eq!(ACT_WIRE_WORDS.len(), 14);
        let mut sorted = ACT_WIRE_WORDS;
        sorted.sort_unstable();
        assert_eq!(sorted, ACT_WIRE_WORDS);
        assert_eq!(act_of_word("mine"), Some((ActKind::Mine, false)));
        assert_eq!(wire_word(ActKind::Mine, false), "mine");
        assert_eq!(act_of_word("mine_stop"), None, "no act");
        assert_eq!(start_word("mine_stop"), Some(StartWord::MineStop));
        for word in ACT_WIRE_WORDS {
            let parsed = start_word(word).unwrap_or_else(|| panic!("{word}"));
            match parsed {
                StartWord::Act { kind, quiet } => {
                    assert_eq!(act_of_word(word), Some((kind, quiet)));
                }
                StartWord::MineStop => assert_eq!(word, "mine_stop"),
                StartWord::PlanFile => assert_eq!(word, "plan_file"),
                StartWord::PlanClear => assert_eq!(word, "plan_clear"),
                StartWord::PlanStrike => assert_eq!(word, "plan_strike"),
                StartWord::PlanEnd => assert_eq!(word, "plan_end"),
            }
        }
        for word in ["plan_file", "plan_clear", "plan_strike", "plan_end"] {
            assert_eq!(act_of_word(word), None, "{word} is no act");
        }
        assert_eq!(start_word("drill"), None, "the alias is the verb's only");
        assert_eq!(
            cycle_key(ObjId(900), 3, 2).map(|k| k.as_str().to_owned()),
            Some("tacsim.surface/900/3/cycle/2".to_owned())
        );
        assert_eq!(
            stop_key(ObjId(900), 3).map(|k| k.as_str().to_owned()),
            Some("tacsim.surface/900/3/stop".to_owned())
        );
        let parameters = |cycle: Value| {
            json!({ "tv": "900", "ordinal": 1, "cycle": cycle })
                .as_object()
                .cloned()
                .unwrap_or_default()
        };
        assert_eq!(cycle_param(&parameters(json!(1))).ok(), Some(Some(1)));
        assert_eq!(cycle_param(&parameters(json!(8))).ok(), Some(Some(8)));
        for bad in [json!(0), json!(9), json!("1"), json!(-1)] {
            assert!(cycle_param(&parameters(bad.clone())).is_err(), "{bad}");
        }
        let none = json!({ "tv": "900", "ordinal": 1 })
            .as_object()
            .cloned()
            .unwrap_or_default();
        assert_eq!(cycle_param(&none).ok(), Some(None));
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
            by: None,
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
            by: None,
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
        // Doc 20 cut SM.8 (decision R15): the queued unload's start has its own key.
        assert_eq!(
            queued_key(ObjId(900), 3).map(|k| k.as_str().to_owned()),
            Some("tacsim.surface/900/3/queued".to_owned())
        );
        // Doc 20 cut SM.11 (the brief's 3.6, D9, D17): the plan's keys.
        let text = |key: Option<IdempotencyKeyText>| key.map(|k| k.as_str().to_owned());
        assert_eq!(
            text(plan_file_key(ObjId(900), 2)),
            Some("tacsim.surface/900/plan/2/file".to_owned())
        );
        assert_eq!(
            text(plan_clear_key(ObjId(900), 2, 7)),
            Some("tacsim.surface/900/plan/2/clear/7".to_owned())
        );
        assert_eq!(
            text(plan_strike_key(ObjId(900), 2, 11)),
            Some("tacsim.surface/900/plan/2/strike/11".to_owned())
        );
        assert_eq!(
            text(plan_end_key(ObjId(900), 2)),
            Some("tacsim.surface/900/plan/2/end".to_owned())
        );
        assert_eq!(
            text(plan_step_key(ObjId(900), 12, 3, "drive")),
            Some("tacsim.surface/900/12/plan/3/drive".to_owned())
        );
        assert_eq!(
            text(idle_key(ObjId(900), 12)),
            Some("tacsim.surface/900/12/idle".to_owned())
        );
        assert_ne!(
            text(plan_clear_key(ObjId(900), 2, 7)),
            text(plan_strike_key(ObjId(900), 2, 7)),
            "a clear and a strike never share a key"
        );
    }
    /// **A plan's filing leaves the slot once it landed or the rig moved on** (doc 20 cut SM.11;
    /// the brief's 3.8, the reviewer's F11): `prune_mine_orders` keeps a filing while
    /// `plans < no` and the record's ordinal is the one it was planned at; drops it once the filing
    /// landed (`plans ≥ no`) or the ordinal moved; a rig with no record keeps a first filing
    /// planned at ordinal 0 and drops one planned at another.
    #[test]
    fn sm11_the_prune_drops_a_landed_or_overtaken_filing() {
        let mut tw = mush_testkit::TestWorld::with_modules(vec![Box::new(
            crate::TacsimRuleset::with_config(crate::fixture::config()),
        )]);
        let w = &mut tw.engine_mut().world;
        let filing = |no: u32, ordinal: u32| {
            ActOrder::Plan(Box::new(PlanOrder {
                actor: ObjId(42),
                ordinal,
                no,
                list: "D".to_owned(),
                family: None,
                compiled: crate::surface::plan::Compiled {
                    list: vec!["D".to_owned()],
                    family: OreFamily::Metals,
                    steps: vec![crate::surface::plan::PlanStep::Block { letter: 'D' }],
                    est_secs: 1,
                    step_secs: vec![1],
                    far: Vec::new(),
                },
            }))
        };
        let record = |tv: u32, ordinal: u32, plans: u32| {
            let mut record = RigRecord::fresh_unread(ObjId(tv));
            record.ordinal = ordinal;
            record.plans = plans;
            record
        };
        let mut run = |orders: Vec<(u32, ActOrder)>, records: &[RigRecord]| {
            if let Some(slot) = w.module_state_mut::<ActOrders>() {
                slot.0 = orders.into_iter().map(|(tv, o)| (ObjId(tv), o)).collect();
            }
            let dropped = prune_mine_orders(w, records);
            let kept: Vec<u32> = w
                .module_state::<ActOrders>()
                .map(|slot| slot.0.keys().map(|tv| tv.0).collect())
                .unwrap_or_default();
            (dropped, kept)
        };
        assert_eq!(
            run(vec![(1, filing(1, 3))], &[record(1, 3, 0)]),
            (0, vec![1]),
            "not yet landed"
        );
        assert_eq!(
            run(vec![(1, filing(1, 3))], &[record(1, 3, 1)]),
            (1, Vec::new()),
            "the filing landed: plans ≥ no"
        );
        assert_eq!(
            run(vec![(1, filing(2, 3))], &[record(1, 4, 1)]),
            (1, Vec::new()),
            "the rig's orders moved on"
        );
        assert_eq!(
            run(vec![(1, filing(1, 0)), (2, filing(1, 2))], &[]),
            (1, vec![1]),
            "no record: a first filing at ordinal 0 stays, another goes"
        );
    }

    /// **A plan's start names its plan and step** (doc 20 cut SM.11; the brief's 3.6): the
    /// parameter rule, word by word.
    #[test]
    fn sm11_a_plan_start_names_its_plan_and_step() {
        let params = |value: Value| value.as_object().cloned().unwrap_or_default();
        let act = |word: &str| start_word(word).unwrap_or_else(|| panic!("{word}"));
        let ok = |word: &str, value: Value| by_params(act(word), &params(value)).is_ok();
        // A plan's step: drive, scan, sound, mine, recover and plan_end, with plan and step.
        for word in ["drive", "scan", "sound", "mine", "recover", "plan_end"] {
            assert!(
                ok(word, json!({ "by": "plan", "plan": 1, "step": 0 })),
                "{word}"
            );
            assert!(
                !ok(word, json!({ "by": "plan", "plan": 1 })),
                "{word}: no step"
            );
            assert!(
                !ok(word, json!({ "by": "plan", "step": 0 })),
                "{word}: no plan"
            );
        }
        for word in ["launch", "dump", "unload", "mine_stop", "plan_file"] {
            assert!(
                !ok(word, json!({ "by": "plan", "plan": 1, "step": 0 })),
                "{word}"
            );
        }
        // The idle return: a recover only, with neither.
        assert!(ok("recover", json!({ "by": "idle" })));
        assert!(!ok("recover", json!({ "by": "idle", "plan": 1 })));
        assert!(!ok("scan", json!({ "by": "idle" })));
        // The controls.
        assert!(ok("plan_file", json!({})));
        assert!(!ok("plan_file", json!({ "plan": 1 })));
        assert!(ok("plan_clear", json!({ "plan": 1 })));
        assert!(ok("plan_strike", json!({ "plan": 1 })));
        assert!(!ok("plan_clear", json!({})));
        assert!(!ok("plan_end", json!({})), "an end is the plan's");
        // A hand start carries none; malformed values refuse.
        assert!(ok("scan", json!({})));
        assert!(!ok("scan", json!({ "plan": 1 })));
        assert!(!ok("drive", json!({ "by": "hand" })));
        assert!(!ok("drive", json!({ "by": "plan", "plan": 0, "step": 0 })));
        assert!(!ok("drive", json!({ "by": "plan", "plan": 1, "step": 65 })));
    }

    /// **An unload's start names how it was planned** (doc 20 cut SM.8; decision R15): `how` reads
    /// the three words and nothing else; `qty` reads whole tonnes `1..=99`; `unload` is an act
    /// word (the `Unload` kind) and its wire word round-trips.
    #[test]
    fn sm8_an_unload_start_names_how() {
        let parameters = |extra: Value| {
            let mut map = json!({ "tv": "900", "act": "unload", "ordinal": 4 })
                .as_object()
                .cloned()
                .unwrap_or_default();
            if let Some(more) = extra.as_object() {
                map.extend(more.clone());
            }
            map
        };
        assert_eq!(act_of_word("unload"), Some((ActKind::Unload, false)));
        assert_eq!(wire_word(ActKind::Unload, false), "unload");
        for (word, how) in [
            ("queued", UnloadHow::Queued),
            ("whole", UnloadHow::Whole),
            ("part", UnloadHow::Part),
        ] {
            assert_eq!(how_of(&parameters(json!({ "how": word }))), Some(how));
        }
        for bad in [json!("all"), json!(1), json!("Whole")] {
            assert_eq!(how_of(&parameters(json!({ "how": bad }))), None, "{bad}");
        }
        assert_eq!(how_of(&parameters(json!({}))), None);
        assert_eq!(qty_of(&parameters(json!({ "qty": 3 }))), Some(3));
        assert_eq!(qty_of(&parameters(json!({ "qty": 99 }))), Some(99));
        for bad in [json!(0), json!(100), json!("3"), json!(-1)] {
            assert_eq!(qty_of(&parameters(json!({ "qty": bad }))), None, "{bad}");
        }
    }

    /// **The unload weighs with the plan's table** (doc 20 cut SM.8; decision R16): the goods table
    /// a hold plan carries (`plan_goods` of the Dray-P's `stowage_plan`) is the compiled-in table
    /// the layer's store loads (`goods_table`, `TacsimStore::goods`), so the room `unload_room`
    /// computes through `hold_unreserved_with(plan_goods(..))` equals `hold_unreserved`'s on the
    /// shipped tables, and the preview, the offer, the prepare and the finish weigh alike. (No test
    /// seam sets the store's table apart from the compiled one, so the unequal-table refusal is
    /// asserted by construction only.)
    #[test]
    fn sm8_the_unload_weighs_with_the_plans_table() {
        let pack = crate::pack::load_pack(crate::definitions::content::DRAY_P_JSON)
            .unwrap_or_else(|e| panic!("{e:?}"));
        let id = CanonicalId::new(crate::definitions::content::DRAY_P)
            .unwrap_or_else(|_| panic!("an id"));
        let logistics = match pack.entry(&id) {
            Some(crate::pack::PackEntry::Unit(spec)) => spec
                .logistics
                .clone()
                .unwrap_or_else(|| panic!("logistics")),
            _ => panic!("the Dray-P unit"),
        };
        let plan =
            crate::port::market::stowage_plan(&logistics).unwrap_or_else(|| panic!("a plan"));
        let table = crate::port::market::plan_goods(&plan).unwrap_or_else(|| panic!("a table"));
        let store = crate::definitions::goods::goods_table().unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(*table, store, "the plan's table is the store's");
        let mut other = plan.clone();
        other.goods = "00".repeat(32);
        assert!(
            crate::port::market::plan_goods(&other).is_none(),
            "another digest reads dark"
        );
        let ore = CanonicalId::new("metal_ore").unwrap_or_else(|_| panic!("an id"));
        assert!(
            crate::port::capacity::compartment_for(
                table,
                &crate::port::market::plan_compartments(&plan),
                &ore
            )
            .is_some(),
            "the Dray-P hold has one ore compartment"
        );
    }

    /// **A load line names its goods as a list** (the lead's pre-commit round, item 5): one good
    /// reads as itself (byte-identical to the old join), two join with " and ", three or more
    /// with ", " and a last " and " — so goods v3's comma-bearing labels never read as more goods.
    #[test]
    fn sm8_a_load_line_names_its_goods_as_a_list() {
        let words = |items: &[&str]| -> Vec<String> {
            items.iter().map(|item| (*item).to_owned()).collect()
        };
        assert_eq!(and_list(&[]), "");
        assert_eq!(
            and_list(&words(&["8 t of ice ore, run-of-mine"])),
            "8 t of ice ore, run-of-mine"
        );
        assert_eq!(
            and_list(&words(&[
                "8 t of ice ore, run-of-mine",
                "4 t of metal ore, run-of-mine"
            ])),
            "8 t of ice ore, run-of-mine and 4 t of metal ore, run-of-mine"
        );
        assert_eq!(
            and_list(&words(&["1 t of a", "2 t of b", "3 t of c"])),
            "1 t of a, 2 t of b and 3 t of c"
        );
    }
}
