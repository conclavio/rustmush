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

/// **How long an unload's hopper markers stay in the hopper's ring, real seconds** (doc 20 cut
/// SM.12c, decision (c); the owner's O-22 default, provisional): the finish moves the hopper's goods
/// by `InventoryFamilies::transfer_many_marked_keeping` at this horizon instead of the ring's real
/// day (`mush_inventory::record::MARKER_RETENTION_SECS`). The unload's durable replay guard once it
/// lands is the rig record's ordinal, written in the same bundle, so the hopper's marker is needed
/// only across the finish's own re-drives — eight attempts and the 60-fire back-offs — which one
/// real hour covers many times over; a hopper writes at most about a dozen markers an hour at rate
/// 24. The market's markers keep the day.
pub(crate) const UNLOAD_MARKER_KEEP_S: i64 = 3_600;

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
    /// An actual accepted physical-crew selection. The source re-reads it and
    /// fences current control/permission before admission; this is no permit.
    pub crew: Option<Box<crate::detachment::sortie::SurfaceCrew>>,
    /// Expected delivered sequence for an NPC's queued action. The source
    /// re-reads the order; this number conveys no permission by itself.
    pub field_order: Option<u64>,
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
/// (`crate::port::coop::helm_of`), by slot. **Doc 20 cut SM.12b**: a yard's dispatcher's rigs are
/// its yard's, as house views (`crate::port::yard::house_rigs`), slot order — it operates them
/// with no helm.
#[must_use]
pub(crate) fn operator_rigs(w: &World, actor: ObjId) -> Vec<RigAt> {
    let Some(helm) = crate::port::coop::helm_of(w, actor) else {
        return crate::port::yard::house_rigs(w, actor)
            .into_iter()
            .map(|record| RigAt {
                crew: None,
                field_order: None,
                tv: record.tv,
                slot: record.slot,
                mark: record.mark.clone(),
                hull: record.hull,
                record,
            })
            .collect();
    };
    let Some(store) = w.module_state::<TacsimStore>() else {
        return Vec::new();
    };
    let mut rigs: Vec<RigAt> = store
        .tv_leases_of(actor)
        .filter(|record| record.live() && record.hull == helm.hull)
        .map(|record| RigAt {
            crew: None,
            field_order: None,
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
            crew: None,
            field_order: None,
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

impl RigAt {
    pub(crate) fn operator_present(&self, w: &World, actor: ObjId) -> bool {
        if let Some(crew) = self.crew.as_deref() {
            crew.operator == actor
                && crate::detachment::planning::projection::physically_aboard(w, actor, self.tv)
        } else {
            crate::port::voyage::aboard(w, self.hull, actor)
        }
    }
    pub(crate) fn mining_principal(&self, actor: ObjId) -> ObjId {
        self.crew.as_deref().map_or(actor, |crew| crew.principal)
    }
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

mod commands;
mod fixed;
mod plans;
mod providers;
mod refusal;

#[allow(unused_imports)]
use self::commands::*;
#[cfg(test)]
pub(crate) use self::commands::{ActOrder, PlanOrder, tenure_verdict};
pub(crate) use self::commands::{
    ActOrders, Authored, PlanWords, TenureBar, aboard_site, act_cmd, authored_of, claim_live,
    drill_of, drive_cmd, end_why, excursion_counts, file_compiled, leg_home, licence_end_line,
    licence_verdict, load_words, mine_cmd, mine_plan, mine_stop_cmd, mine_tenure, parked_short,
    pass_why, plan_ask, plan_clear_cmd, plan_cmd, plan_show_cmd, prune_mine_orders, rig_drill_id,
    site_def_of, unload_cmd, wait_why, want_cold,
};
pub use self::commands::{
    mine_order_for_tests, plan_counts_for_tests, plan_order_for_tests, unload_order_for_tests,
};
#[allow(unused_imports)]
use self::fixed::*;
pub(crate) use self::fixed::{
    ByParams, ByWord, offer_draft, offer_draft_with_revisions, parameter, queued_ring_full,
    read_record, self_query, write_record,
};
#[allow(unused_imports)]
use self::plans::*;
pub(crate) use self::plans::{
    ActPlan, LeaseBar, UnloadRoom, battery_of, doing_words, drilling_words, drive_plan, dump_plan,
    launch_plan, lease_gate, percent, recover_plan, rig_battery_id, rig_rates, rig_scanner_id,
    scan_plan, scanner_of, sound_plan, stand_of, unload_plan, unload_room, world_rigs,
};
#[allow(unused_imports)]
use self::providers::*;
pub(crate) use self::providers::{
    SurfaceActProvider, SurfaceDoneProvider, read_cursor, write_cursor,
};
pub use self::refusal::Refusal;
pub(crate) use self::refusal::code_of_word;
#[allow(unused_imports)]
use self::refusal::*;
#[cfg(test)]
pub(crate) use self::refusal::{refusal_samples, refusal_words};

#[cfg(test)]
mod tests;
