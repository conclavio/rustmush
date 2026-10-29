//! **The ship's epoch on the port voyage** (doc 23 cut S.2; §6 "What the host does at each
//! end", §7 "Records", §9's S.2 row, §10 tier 1; D-23-1, D-23-2, D-23-3, D-23-5, D-23-7, D-23-8;
//! the S.2 brief's lead rulings V1–V7 and the fix round's R1–R9).
//!
//! A ship's seal epoch opens when the launch arm's depart moves the hull off a port and closes
//! `returned_to_base` when it docks at a hangar port again; it stays open across every site it
//! settles on (D-23-3: a mining or field site is the field), and a lapsed burn departs nothing
//! and opens nothing. The epoch is seal and record (D-23-7): the ROM seal over the ship's
//! `TacticalUnit`, the ship's typed record ([`ShipEpochRecord`], `tacsim.ship_epoch`, one per
//! hull; the fix round's R1, amending V2) and a `mind_sealed`/`sortie_closed` pair in the hull's
//! own black box ([`crate::surface::black_box`]).
//!
//! **A pull-tick reconciliation, not a write in the launch's outbox** (V1): the launch's depart
//! is a direct tick write, and a black box line needs its owner participant's commit, so the two
//! cannot share one commit. The `tacsim.port` tick's **epoch arm** (agent = the hull, principal =
//! the epoch's warrant principal, V5) compares the hull's placement and journey with its record
//! (`epoch_due`) and submits at most one `tacsim_voyage_epoch` as the principal through
//! `Fire::as_principal`; the action's prepare re-reads the record, is idempotent by it (an open
//! with an epoch open or of the id it last wrote, or a close with none, is refused `Conflict`),
//! stages the record's move and appends the line through S.1's `black_box::append` — one bundle,
//! exactly a rig's shape. The sailing plan (`tacsim:voyage:<hull>`) is the tick's alone: the
//! launch writes its `departed`, which the arm reads, and nothing here writes it. The window
//! between the depart and the open's commit — one fire plus any deferrals — is accepted; the
//! id's and the line's instant is the journey's `departed().at`, never the fire's. A reboot
//! re-finds the state; a re-run fire mints nothing (the record is the cursor). The tick submits
//! nothing itself; the applier and the boot pass submit nothing.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, ActionTxn,
    AuthoritativeActionOutput, CanonicalId, Ctx, DefinitionDigest, DefinitionRef, GOD,
    IdempotencyKeyText, ObjId, PerceivedRef, ProjectedParameterKind, ProviderOfferDraft,
    RulesValue, StateFamilyHandle, World,
};
use mush_roll::commerce::ic_claims::IcCloseKind;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::arena::mind::SortieId;
use crate::definitions::UnitClass;
use crate::deliberation::order::CharacterRef;
use crate::port::coop::helm_holder;
use crate::port::def::PortDef;
use crate::port::families::PortHandles;
use crate::port::map::PortMap;
use crate::port::ship::{Berth, berth_of, hull_view, is_hangar_port};
use crate::port::voyage::{LAUNCH_GRACE_FIRES, VoyageState, lane_ports};
use crate::port_tick::{Fire, PortTickState, Step, SwapRefused, gone, port_error};
use crate::store::TacsimStore;
use crate::surface::black_box::{self, BlackBoxLine, BlackBoxParticipant, SealFacts};

/// The ship's epoch action (doc 23 cut S.2): submitted by the `tacsim.port` tick's epoch arm as
/// the epoch's warrant principal; no verb reaches it.
pub const EPOCH_ACTION: &str = "tacsim_voyage_epoch";
/// Its outcome.
pub const EPOCH_OUTCOME: &str = "tacsim_voyage_epoch_result";
/// **Its typed family** (`tacsim.ship_epoch`): one [`ShipEpochRecord`] per ship.
pub const EPOCH_FAMILY: &str = "tacsim.ship_epoch";
/// Its idempotency keys' prefix: `tacsim.port.epoch/<hull>/<at_ic>/<op>/<attempt>`, the attempt
/// the record's durable [`ShipEpochRecord::attempts`] plus one (the fix round's R8).
pub const EPOCH_KEY_PREFIX: &str = "tacsim.port.epoch/";
/// **Submissions one due open or close is given a boot** before the arm holds it and pages: a
/// submission that lands moves the record and the target is no longer due, so a second is made
/// only when the first did not land (dropped, refused, or its principal gone).
pub const MAX_EPOCH_ATTEMPTS: u8 = 3;
/// Fires the arm waits before it submits the same due open or close again (the launch's grace,
/// about two minutes).
pub const EPOCH_RETRY_FIRES: u64 = LAUNCH_GRACE_FIRES;

/// **Which end of a ship's epoch** is due.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EpochOp {
    /// The hull left a port: open it.
    Open,
    /// The hull is docked at a hangar port: close it.
    Close,
}

impl EpochOp {
    /// The word (`open`, `close`).
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            EpochOp::Open => "open",
            EpochOp::Close => "close",
        }
    }

    fn parse(word: &str) -> Option<EpochOp> {
        match word {
            "open" => Some(EpochOp::Open),
            "close" => Some(EpochOp::Close),
            _ => None,
        }
    }
}

/// **The epoch family's key** (doc 23 cut S.2): one record per ship, keyed by the hull.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EpochKey(pub ObjId);

impl EpochKey {
    /// The hull's id in decimal (`hull.0`, never `ObjId`'s `#`-prefixed Display).
    #[must_use]
    pub fn key_text(&self) -> String {
        self.0.0.to_string()
    }

    /// The strict inverse of [`EpochKey::key_text`].
    #[must_use]
    pub fn parse(text: &str) -> Option<EpochKey> {
        crate::port::families::strict_decimal_u32(text).map(|hull| EpochKey(ObjId(hull)))
    }
}

/// **A ship's open epoch** (doc 23 cut S.2; §7; D-23-1, D-23-5): its id, the port it left and
/// that port's host room, the warrant principal, when it opened, whether its open line went
/// unwritten, and its seal's facts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShipEpoch {
    /// [`SortieId::ic`]`(hull, departed_at)` — the hull and the journey's `departed().at`.
    pub id: SortieId,
    /// The port entry the hull left (a hangar port; a site when a hull lifts from the field with
    /// no epoch open, the fix round's R6, paged).
    pub from: CanonicalId,
    /// That port's host room (`PortMap.host`).
    pub base: ObjId,
    /// The warrant principal (D-23-5): the filing's pilot, else the hull's helm.
    pub principal: ObjId,
    /// The IC instant it opened: the journey's `departed().at`, never the fire's.
    pub opened_ic: i64,
    /// **Whether its open line was never written** (cut S.1's F2 rule): the black box was dark
    /// or full, or the act was prepared off the composed path. Skipped when `false`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unrecorded: bool,
    /// **The seal's facts** (the lead's V6): the ROM seal over the ship's `TacticalUnit`; `None`
    /// when the hull had none (the line says `seal: absent`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seal: Option<SealFacts>,
}

/// **The last end a `tacsim_voyage_epoch` committed** (doc 23 cut S.2): which end, of which
/// epoch, at which instant, and whether its line was written.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpochLast {
    /// The end.
    pub op: EpochOp,
    /// The epoch.
    pub id: SortieId,
    /// The end's instant (the open's `departed().at`, the close's arrival).
    pub at_ic: i64,
    /// Whether its black box line was written.
    pub written: bool,
}

/// **A ship's epoch record** (doc 23 cut S.2, the fix round's R1 amending V2; `tacsim.ship_epoch`,
/// one per hull, replaced at every end): the open epoch, the last end committed, and how many
/// ends this hull's record has committed. **The ship's epoch is this record**; written only by
/// `tacsim_voyage_epoch`, in the bundle that carries its black box line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShipEpochRecord {
    /// The hull (the identity rule: its key's).
    pub hull: ObjId,
    /// The open epoch, from its open to its close.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<ShipEpoch>,
    /// The last end committed (R3: an open never re-mints its id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<EpochLast>,
    /// **Ends committed for this hull** (R8): the next submission's idempotency attempt is this
    /// plus one, durable across reboots, so no key is ever reused for a later end.
    pub attempts: u32,
}

impl ShipEpochRecord {
    /// A hull's record before its first end.
    #[must_use]
    pub fn fresh(hull: ObjId) -> ShipEpochRecord {
        ShipEpochRecord {
            hull,
            open: None,
            last: None,
            attempts: 0,
        }
    }

    /// **The record's own invariants**: every epoch it names is an IC epoch of its hull; the open
    /// one opened at its id's instant, into another object, under a principal; an open epoch is
    /// the last end's (an open of the same id); a record with a last end counted it.
    ///
    /// # Errors
    ///
    /// The first invariant broken, as a sentence.
    pub fn check(&self) -> Result<(), String> {
        let own = |id: SortieId| matches!(id, SortieId::Ic { hull, .. } if hull == self.hull);
        if self.hull.0 == 0 {
            return Err("a ship's epoch record names its hull".to_owned());
        }
        if let Some(open) = &self.open {
            if !matches!(open.id, SortieId::Ic { at_ic, .. } if at_ic == open.opened_ic)
                || !own(open.id)
            {
                return Err(
                    "a ship's epoch is an IC epoch of its own hull, opened when it left".to_owned(),
                );
            }
            if open.base == self.hull || open.base.0 == 0 || open.principal.0 == 0 {
                return Err("a ship's epoch left another object, under a principal".to_owned());
            }
            if self
                .last
                .as_ref()
                .is_none_or(|last| last.op != EpochOp::Open || last.id != open.id)
            {
                return Err("an open epoch is its record's last end".to_owned());
            }
        }
        if let Some(last) = &self.last {
            if !own(last.id) {
                return Err("a ship's last end is an IC epoch of its own hull".to_owned());
            }
            if self.attempts == 0 {
                return Err("a ship's record counts the ends it committed".to_owned());
            }
        }
        Ok(())
    }
}

/// **A hull's epoch record through an engine's typed read** (the offers' read, as the surface
/// finish's `DoneRig` facts read a rig's record).
pub(crate) fn record_in(engine: &mush_core::Engine, hull: ObjId) -> Option<ShipEpochRecord> {
    let handles = engine.world.module_state::<PortHandles>()?;
    engine
        .action_state()
        .get(&handles.families.epochs, &EpochKey(hull))
        .ok()
        .flatten()
        .cloned()
}

/// **A carrier's open epoch id, through an engine's typed read** (doc 23 cut S.2, V7): the id a
/// rig launched from `hull`'s bay names as its hangar's `sortie`.
pub(crate) fn open_id_in(engine: &mush_core::Engine, hull: ObjId) -> Option<SortieId> {
    record_in(engine, hull).and_then(|record| record.open.map(|open| open.id))
}

/// A hull's epoch record through the tick's own typed-state lens; `Err` when the lens refuses.
fn record_at(ctx: &Ctx<'_>, hull: ObjId) -> Result<Option<ShipEpochRecord>, ()> {
    let handles = ctx.w.module_state::<PortHandles>().ok_or(())?;
    ctx.action_state(&handles.families.epochs, &EpochKey(hull))
        .map(|found| found.cloned())
        .map_err(|_| ())
}

/// **What is due on a hull's epoch now** ([`epoch_due`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EpochDue {
    /// Open or close.
    pub(crate) op: EpochOp,
    /// The epoch's id: minted from the journey for an open, the record's for a close.
    pub(crate) id: SortieId,
    /// The warrant principal (V5, R2): a live departure's pilot, else the live helm, for an open;
    /// the epoch's own when live, else the live helm, for a close.
    pub(crate) principal: ObjId,
    /// An open's journey `departed().at`; a close's instant (the journey's arrival there, else
    /// the IC now).
    pub(crate) at_ic: i64,
    /// An open's port entry left.
    pub(crate) from: Option<CanonicalId>,
    /// An open's port of departure's host room; a close's arrival port's host room.
    pub(crate) base: ObjId,
    /// An open's principal came from the sailing plan's `departed` (else the helm, paged once).
    pub(crate) recorded: bool,
    /// An open from a site, not a hangar port (R6, paged once).
    pub(crate) from_field: bool,
    /// **Doc 23 cut S.3** (A5; §2 "Recovery"): a close at a hangar port the hull's own journey
    /// did not bring it to — the journey book's journey for the hull has no arrival at this
    /// port's host — so the hull was brought in by other means than its own drive and the
    /// close is `RecoveredToBase`. `false` for an open.
    pub(crate) recovered: bool,
}

/// Whether `who` can warrant an act: a live object, never GOD (R2).
fn live(w: &World, who: ObjId) -> bool {
    who != GOD && who.0 != 0 && !gone(w, who)
}

/// The live helm of `hull`, if any (R2's fallback).
fn live_helm(w: &World, hull: ObjId) -> Option<ObjId> {
    helm_holder(w, hull).filter(|helm| live(w, *helm))
}

/// The lane port map and entry whose entry is `entry`.
fn port_of_entry<'w>(w: &'w World, entry: &CanonicalId) -> Option<(&'w PortMap, &'w PortDef)> {
    lane_ports(w).into_iter().find_map(|(def, map)| {
        let map = map?;
        (map.entry == *entry).then_some((map, def))
    })
}

/// **What is due on `hull`'s epoch now** (the lead's V1), from its typed record, its sailing
/// plan's departure, its placement and its journey — the one reading the arm, the offer and the
/// prepare share:
///
/// - **close** when the record holds an open epoch and the hull is docked at a hangar port
///   ([`Berth::hangar`], V3), into that port's host room, at the journey's arrival there;
/// - **open** when the record holds none and the hull left a port on a journey — under way, or
///   settled on a site's pad that journey brought it to — minting
///   [`SortieId::ic`]`(hull, departed().at)`, never the id the record last wrote (R3);
/// - otherwise nothing: docked at a hangar with no epoch, a site's pad with the epoch open
///   (D-23-3), a lapsed burn (no journey), an unreadable plan (paged by the port tick).
///
/// # Errors
///
/// A due end that cannot be written — no live principal (R2), or a port of departure that does
/// not resolve — as the sentence the arm pages once; the hull is then not due, so no arm is
/// denied it.
pub(crate) fn epoch_due(
    w: &World,
    hull: ObjId,
    record: Option<&ShipEpochRecord>,
) -> Result<Option<EpochDue>, &'static str> {
    let Some(store) = w.module_state::<TacsimStore>() else {
        return Ok(None);
    };
    let voyage = match store.voyage_state(hull) {
        VoyageState::Present(voyage) => Some(voyage),
        VoyageState::Absent => None,
        VoyageState::Unreadable => return Ok(None),
    };
    let journey = w
        .module_state::<mush_sim::JourneyBook>()
        .and_then(|book| book.get(hull));
    let berth = berth_of(w, hull);
    if let Some(open) = record.and_then(|record| record.open.as_ref()) {
        let Some((map, _)) = berth.hangar() else {
            return Ok(None);
        };
        let Some(host) = map.host.map(ObjId) else {
            return Err("the hangar port's host room is not made");
        };
        let principal = if live(w, open.principal) {
            open.principal
        } else {
            live_helm(w, hull).ok_or("the epoch's principal is gone and the hull has no helm")?
        };
        // Doc 23 cut S.3 (A5): the same test decides the instant and the kind — the journey's
        // arrival here is a return under the hull's own drive; no such arrival (another
        // journey's port, or none) is a hull brought in by other means, a recovery, at now.
        let arrived = journey
            .filter(|journey| journey.route().endpoints().1 == host)
            .and_then(mush_sim::CarrierJourney::arrival_at);
        let at_ic = arrived
            .unwrap_or_else(|| w.world_ic_now().at)
            .max(open.opened_ic);
        return Ok(Some(EpochDue {
            op: EpochOp::Close,
            id: open.id,
            principal,
            at_ic,
            from: None,
            base: host,
            recorded: true,
            from_field: false,
            recovered: arrived.is_none(),
        }));
    }
    let Some(journey) = journey else {
        return Ok(None);
    };
    let left = match berth {
        Berth::UnderWay => true,
        // D-23-3: a site's pad the journey brought the hull to; a hangar ends the passage.
        Berth::Docked { map, def } => {
            !is_hangar_port(def) && map.host == Some(journey.route().endpoints().1.0)
        }
        Berth::Elsewhere => false,
    };
    if !left {
        return Ok(None);
    }
    let departed_at = journey.departed().at;
    let id = SortieId::ic(hull, departed_at);
    // R3: the same id never opens twice — whether its epoch closed, or is the open one.
    if record
        .and_then(|record| record.last.as_ref())
        .is_some_and(|last| last.id == id)
    {
        return Ok(None);
    }
    let recorded = voyage
        .and_then(|voyage| voyage.departed.as_ref())
        .filter(|departed| departed.depart_ic == departed_at);
    let principal = match recorded.map(|departed| departed.pilot) {
        Some(pilot) if live(w, pilot) => pilot,
        Some(_) | None => {
            live_helm(w, hull).ok_or("the hull has no live helm to warrant its epoch")?
        }
    };
    let (map, def) = match recorded.and_then(|departed| port_of_entry(w, &departed.from)) {
        Some(found) => found,
        None => crate::port::voyage::entry_of_host(w, journey.route().endpoints().0)
            .ok_or("the port the hull left does not resolve")?,
    };
    let base = map
        .host
        .map(ObjId)
        .ok_or("the port the hull left has no host room")?;
    Ok(Some(EpochDue {
        op: EpochOp::Open,
        id,
        principal,
        at_ic: departed_at,
        from: Some(map.entry.clone()),
        base,
        recorded: recorded.is_some_and(|departed| departed.pilot == principal),
        from_field: !is_hangar_port(def),
        recovered: false,
    }))
}

/// **Whether `hull`'s last passage ended at a hangar with no epoch closed for it** (the fix
/// round's R5): docked at a hangar port, no epoch open, the sailing plan's departure is the
/// journey's, and the record's last end is not that journey's close.
fn missed(w: &World, hull: ObjId, record: Option<&ShipEpochRecord>) -> bool {
    if record.is_some_and(|record| record.open.is_some()) || berth_of(w, hull).hangar().is_none() {
        return false;
    }
    let Some(journey) = w
        .module_state::<mush_sim::JourneyBook>()
        .and_then(|book| book.get(hull))
    else {
        return false;
    };
    let departed_at = journey.departed().at;
    let Some(store) = w.module_state::<TacsimStore>() else {
        return false;
    };
    let VoyageState::Present(voyage) = store.voyage_state(hull) else {
        return false;
    };
    if voyage
        .departed
        .as_ref()
        .is_none_or(|departed| departed.depart_ic != departed_at)
    {
        return false;
    }
    let closed = EpochLast {
        op: EpochOp::Close,
        id: SortieId::ic(hull, departed_at),
        at_ic: 0,
        written: false,
    };
    !record
        .and_then(|record| record.last.as_ref())
        .is_some_and(|last| last.op == closed.op && last.id == closed.id)
}

/// **The hulls whose epoch the arm may owe** (V1): every programme hull with a sailing plan, and
/// every programme hull the journey book carries. Other carriers are never a ship's epoch.
fn candidates(w: &World) -> BTreeSet<ObjId> {
    let mut hulls: BTreeSet<ObjId> = w
        .module_state::<TacsimStore>()
        .map(|store| store.voyages().map(|voyage| voyage.hull).collect())
        .unwrap_or_default();
    if let Some(book) = w.module_state::<mush_sim::JourneyBook>() {
        hulls.extend(
            book.journeys
                .keys()
                .copied()
                .filter(|hull| hull_view(w, *hull).is_some()),
        );
    }
    hulls
}

/// **One due open or close the arm submitted this boot** (RAM; doc 23 cut S.2): the target, the
/// submissions made this boot, the fire of the last — the retry spacing and the boot's hold
/// (R8: the key's attempt is the record's, never this).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EpochTry {
    op: EpochOp,
    id: SortieId,
    submissions: u8,
    last_fire: u64,
}

impl EpochTry {
    fn is_for(&self, due: &EpochDue) -> bool {
        self.op == due.op && self.id == due.id
    }
}

/// Why the arm paged a hull (once a boot each).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum EpochWarn {
    /// A due end that cannot be written ([`epoch_due`]'s error; R2).
    Unwritable,
    /// The record did not read through the tick's lens.
    Unreadable,
    /// An open warranted by the helm: the sailing plan records no live departure's pilot.
    Helm,
    /// An open over a hull with no `TacticalUnit` (the seal absent, V6).
    NoUnit,
    /// An open from a site, not a hangar port (R6).
    FromField,
    /// A passage ended at a hangar with no epoch closed for it (R5).
    Missed,
    /// [`MAX_EPOCH_ATTEMPTS`] submissions did not land.
    Exhausted,
    /// **Doc 23 cut S.3's fix round, R7**: a close read as a recovery ([`EpochDue::recovered`]) —
    /// docked at a hangar port the hull's own journey did not bring it to — paged so a misread
    /// own-drive docking shows in the pages.
    Recovered,
}

/// Page `why` for `hull` once a boot.
fn page(state: &mut PortTickState, hull: ObjId, warn: EpochWarn, why: &str) {
    if state.epoch_warned.insert((hull.0, warn)) {
        tracing::error!(
            hull = hull.0,
            ?warn,
            why,
            runbook = "doc 23 cut S.2: the ship's epoch (its tacsim.ship_epoch record, the \
                       log:hull.<hull> black box); a restart re-finds it",
            "tacsim: a ship's epoch needs staff"
        );
        state.epoch_pages = state.epoch_pages.saturating_add(1);
    }
}

/// **The epoch arm's agents this fire** (V1, R2, R4): `(offered, withheld)` — one agent per
/// candidate hull whose open or close is due and not waiting out [`EPOCH_RETRY_FIRES`] (agent =
/// the hull, principal = [`EpochDue::principal`]), and every hull whose end is due (offered or
/// spacing out its retry), which the walk withholds from the launch, recall and order arms. A
/// target given [`MAX_EPOCH_ATTEMPTS`] this boot is held, paged, and no longer withheld (a stuck
/// epoch never wedges its hull). A hull whose end cannot be written is paged and not due. The
/// RAM of a target no longer due is dropped.
pub(crate) fn epoch_agents(
    ctx: &Ctx<'_>,
    state: &mut PortTickState,
) -> (Vec<(ObjId, ObjId)>, BTreeSet<ObjId>) {
    let mut offered = Vec::new();
    let mut withheld = BTreeSet::new();
    if state.epoch_action.is_none() {
        return (offered, withheld);
    }
    let w: &World = ctx.w;
    let mut due_now: BTreeMap<u32, EpochDue> = BTreeMap::new();
    for hull in candidates(w) {
        let record = match record_at(ctx, hull) {
            Ok(record) => record,
            Err(()) => {
                page(
                    state,
                    hull,
                    EpochWarn::Unreadable,
                    "the epoch record does not read",
                );
                continue;
            }
        };
        match epoch_due(w, hull, record.as_ref()) {
            Ok(Some(due)) => {
                if due.op == EpochOp::Open && !due.recorded {
                    page(
                        state,
                        hull,
                        EpochWarn::Helm,
                        "the sailing plan records no live departure's pilot for this journey; \
                         the helm warrants the epoch",
                    );
                }
                if due.op == EpochOp::Open && due.from_field {
                    page(
                        state,
                        hull,
                        EpochWarn::FromField,
                        "the hull left a site, not a hangar port, with no epoch open; the epoch \
                         opens from the field",
                    );
                }
                if due.op == EpochOp::Close && due.recovered {
                    page(
                        state,
                        hull,
                        EpochWarn::Recovered,
                        "the ship is docked at a hangar port its own journey did not bring it to; \
                         its epoch closes recovered_to_base",
                    );
                }
                if due.op == EpochOp::Open && mush_tactics::module::unit_of(w, hull).is_none() {
                    page(
                        state,
                        hull,
                        EpochWarn::NoUnit,
                        "the ship has no TacticalUnit to seal; its epoch opens with the seal \
                         absent",
                    );
                }
                due_now.insert(hull.0, due);
            }
            Ok(None) => {
                if missed(w, hull, record.as_ref()) {
                    page(
                        state,
                        hull,
                        EpochWarn::Missed,
                        "the passage ended at a hangar port with no epoch closed for it",
                    );
                }
            }
            Err(why) => page(state, hull, EpochWarn::Unwritable, why),
        }
    }
    state
        .epoch_tries
        .retain(|hull, tried| due_now.get(hull).is_some_and(|due| tried.is_for(due)));
    for (hull, due) in due_now {
        let ready = match state.epoch_tries.get(&hull) {
            None => true,
            Some(tried) if tried.submissions >= MAX_EPOCH_ATTEMPTS => {
                page(
                    state,
                    ObjId(hull),
                    EpochWarn::Exhausted,
                    "the ship's epoch was submitted and did not land; it is held until a restart",
                );
                continue;
            }
            Some(tried) => state.fires.saturating_sub(tried.last_fire) >= EPOCH_RETRY_FIRES,
        };
        withheld.insert(ObjId(hull));
        if ready {
            offered.push((ObjId(hull), due.principal));
        }
    }
    (offered, withheld)
}

/// Record one try at `due` on `hull` this fire.
fn tried(state: &mut PortTickState, hull: ObjId, due: &EpochDue) {
    let submissions = state
        .epoch_tries
        .get(&hull.0)
        .filter(|tried| tried.is_for(due))
        .map_or(0, |tried| tried.submissions)
        .saturating_add(1);
    state.epoch_tries.insert(
        hull.0,
        EpochTry {
            op: due.op,
            id: due.id,
            submissions,
            last_fire: state.fires,
        },
    );
}

/// **The epoch step for one hull** (V1): the due reading again, then ONE `tacsim_voyage_epoch`
/// submission as the principal (`Fire::as_principal`), keyed by the hull, the epoch's instant,
/// the end and the record's next attempt (R8). A deferral records nothing; a submission or a
/// refusal records a try (R2: never offered-and-refused every fire); a submission is verified by
/// pull — the next fire finds the record moved and the target no longer due.
pub(crate) fn epoch_step(
    ctx: &mut Ctx<'_>,
    state: &mut PortTickState,
    fire: &mut Fire,
    hull: ObjId,
    principal: ObjId,
) -> Step {
    let Some(action) = state.epoch_action.clone() else {
        return Step::Idle;
    };
    let Ok(record) = record_at(ctx, hull) else {
        return Step::Idle;
    };
    let due = match epoch_due(ctx.w, hull, record.as_ref()) {
        Ok(Some(due)) if due.principal == principal => due,
        Ok(_) | Err(_) => return Step::Idle,
    };
    let attempt = record
        .as_ref()
        .map_or(0, |record| record.attempts)
        .saturating_add(1);
    let at = match due.id {
        SortieId::Ic { at_ic, .. } => at_ic,
        SortieId::Packed(_) => {
            tried(state, hull, &due);
            return Step::Refused;
        }
    };
    let Ok(key) = IdempotencyKeyText::parse_canonical(format!(
        "{EPOCH_KEY_PREFIX}{}/{at}/{}/{attempt}",
        hull.0,
        due.op.word()
    )) else {
        tracing::error!(hull = hull.0, "tacsim: a ship's epoch key did not form");
        tried(state, hull, &due);
        return Step::Refused;
    };
    let parameters = json!({ "hull": hull.0.to_string(), "op": due.op.word() });
    let submitted = fire.as_principal(ctx, hull, principal, |ctx| {
        ctx.submit_authoritative(action, PerceivedRef::SelfActor, Vec::new(), parameters, key);
    });
    match submitted {
        Ok(()) => {
            tried(state, hull, &due);
            Step::Submitted
        }
        Err(SwapRefused::AlreadyActed | SwapRefused::Owed | SwapRefused::Pending) => Step::Deferred,
        Err(SwapRefused::God | SwapRefused::Gone) => {
            tried(state, hull, &due);
            Step::Refused
        }
    }
}

/// **How many times the epoch arm has paged this boot** (doc 23 cut S.2): a status read for tests.
#[must_use]
pub fn epoch_pages(w: &World) -> u32 {
    w.module_state::<PortTickState>()
        .map_or(0, |state| state.epoch_pages)
}

// ---- the action --------------------------------------------------------------------------------

/// **What the offer fixes for one hull's due open or close** (V1, V6): server facts read with the
/// World in hand — the epoch's id and instant, the port, and for an open the port left, the ROM
/// seal over the ship's `TacticalUnit`, that unit's definition digest (the prepare re-reads it)
/// and its class.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EpochOffer {
    op: EpochOp,
    id: SortieId,
    at_ic: i64,
    base: ObjId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    from: Option<CanonicalId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    seal: Option<SealFacts>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    unit_definition: Option<DefinitionDigest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    class: Option<UnitClass>,
    /// **Doc 23 cut S.3** (A5): a close into a hangar port the hull's own journey did not bring
    /// it to ([`EpochDue::recovered`]). Skipped when `false`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    recovered: bool,
}

/// The offer's facts for `due` on `hull` (an open's seal over the live unit, V6).
fn offer_of(w: &World, store: &TacsimStore, hull: ObjId, due: &EpochDue) -> EpochOffer {
    let mut offer = EpochOffer {
        op: due.op,
        id: due.id,
        at_ic: due.at_ic,
        base: due.base,
        from: due.from.clone(),
        seal: None,
        unit_definition: None,
        class: None,
        recovered: due.recovered,
    };
    if due.op == EpochOp::Open
        && let Some(unit) = mush_tactics::module::unit_of(w, hull)
    {
        offer.unit_definition = Some(unit.schema().digest());
        offer.class = Some(UnitClass::read(unit));
        let sealed = crate::match_::seal::seal_rom_for(
            hull,
            unit,
            &[],
            due.id,
            &crate::match_::seal::IcSealConfig {
                minds: &store.cfg.minds,
                reflex: &store.cfg.arena.reflex,
                principal: Some(CharacterRef::Character { id: due.principal }),
                impounded: crate::match_::seal::core_impounded(w, unit),
            },
        );
        match sealed {
            // Doc 17h cut M8a: a ROM seal is read under `doctrine`.
            Ok(seal) => {
                offer.seal = Some(SealFacts::of(
                    &seal,
                    crate::minds::licence_state::SealLicenceState::Doctrine,
                ));
            }
            Err(error) => tracing::error!(
                hull = hull.0,
                %error,
                "tacsim: a ship's ROM seal refused; its epoch opens with the seal absent"
            ),
        }
    }
    offer
}

/// **The ship's epoch provider** (doc 23 cut S.2): offered to a principal for the hulls whose
/// epoch it warrants and is due; its prepare moves the hull's `tacsim.ship_epoch` record and
/// appends the line through S.1's `black_box::append`, in one bundle.
pub(crate) struct EpochProvider {
    pub(crate) action: DefinitionRef,
    pub(crate) outcome: DefinitionRef,
    /// The `tacsim.ship_epoch` records.
    pub(crate) records: StateFamilyHandle<EpochKey, ShipEpochRecord>,
    pub(crate) black_box: mush_core::participant::ParticipantHandle<BlackBoxParticipant>,
}

fn invalid(message: &'static str) -> ActionError {
    port_error(ActionErrorCode::InvalidRequest, message)
}

/// The invocation's parameters: self-scoped, no targets, an object.
fn parameters_of(invocation: &ActionInvocation) -> Result<&Map<String, Value>, ActionError> {
    if invocation.subject() != &PerceivedRef::SelfActor || !invocation.targets().is_empty() {
        return Err(invalid(
            "a ship's epoch is self-scoped and takes no targets",
        ));
    }
    invocation
        .parameters()
        .as_object()
        .ok_or_else(|| invalid("a ship's epoch names a hull and an end"))
}

/// The `hull` and `op` parameters.
fn request(parameters: &Map<String, Value>) -> Result<(ObjId, EpochOp), ActionError> {
    let hull = parameters
        .get("hull")
        .and_then(Value::as_str)
        .and_then(crate::port::families::strict_decimal_u32)
        .filter(|hull| *hull >= 1)
        .map(ObjId)
        .ok_or_else(|| invalid("a ship's epoch names one hull"))?;
    let op = parameters
        .get("op")
        .and_then(Value::as_str)
        .and_then(EpochOp::parse)
        .ok_or_else(|| invalid("a ship's epoch opens or closes"))?;
    Ok((hull, op))
}

impl EpochProvider {
    fn output(
        &self,
        invocation: &ActionInvocation,
        (hull, op, written): (ObjId, EpochOp, bool),
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({
                "actor": invocation.actor().0,
                "hull": hull.0,
                "op": op.word(),
                "written": written,
            }),
        )
        .map_err(|_| {
            port_error(
                ActionErrorCode::BoundsExceeded,
                "a ship's epoch outcome exceeds canonical bounds",
            )
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

impl ActionProvider for EpochProvider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        request(parameters_of(invocation)?).map(|_| ())
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        if !crate::surface::act::self_query(action, &self.action, query) {
            return Ok(None);
        }
        let w: &World = &engine.world;
        let actor = query.actor();
        let Some(store) = w.module_state::<TacsimStore>() else {
            return Ok(None);
        };
        let mut hulls: BTreeMap<String, EpochOffer> = BTreeMap::new();
        for hull in candidates(w) {
            let record = engine
                .action_state()
                .get(&self.records, &EpochKey(hull))
                .ok()
                .flatten();
            if let Ok(Some(due)) = epoch_due(w, hull, record)
                && due.principal == actor
            {
                hulls.insert(hull.0.to_string(), offer_of(w, store, hull, &due));
            }
        }
        if hulls.is_empty() {
            return Ok(None);
        }
        let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
        let fixed = json!({
            "now": w.world_ic_now().at,
            "hulls": serde_json::to_value(&hulls)
                .map_err(|_| integrity("a ship's epoch facts do not encode"))?,
        });
        let parameters = vec![
            crate::surface::act::parameter(
                "hull",
                true,
                ProjectedParameterKind::Choice {
                    choices: hulls.keys().cloned().collect(),
                },
            )?,
            crate::surface::act::parameter(
                "op",
                true,
                ProjectedParameterKind::Choice {
                    choices: vec!["open".to_owned(), "close".to_owned()],
                },
            )?,
        ];
        crate::surface::act::offer_draft(
            action,
            engine,
            fixed,
            parameters,
            (
                "tacsim-voyage-epoch",
                "Record a ship's passage",
                "Open or close a ship's seal epoch when it leaves a port or reaches a hangar.",
            ),
        )
        .map(Some)
    }

    /// The tick's offer is never a player's. The record's move is the typed write and the line
    /// rides it through S.1's `black_box::append`: off the composed path (a typed-only
    /// preparation) the record still moves and the open says `unrecorded` (cut S.1's F2 rule).
    fn prepare_authoritative_with_context(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        context: &mut mush_core::PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let parameters = parameters_of(invocation)?;
        let (hull, op) = request(parameters)?;
        let actor = invocation.actor();
        let offer: EpochOffer = parameters
            .get("hulls")
            .and_then(|hulls| hulls.get(hull.0.to_string()))
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .ok_or_else(|| {
                port_error(
                    ActionErrorCode::Unauthorized,
                    "that ship's epoch is not yours to record",
                )
            })?;
        if offer.op != op {
            return Err(invalid("a ship's epoch end is the one its offer fixed"));
        }
        let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
        let conflict = |message: &'static str| port_error(ActionErrorCode::Conflict, message);
        let key = EpochKey(hull);
        let prior = txn
            .get(&self.records, &key)
            .map_err(|_| integrity("a ship's epoch records are unavailable"))?
            .cloned();
        let existed = prior.is_some();
        let mut record = prior.unwrap_or_else(|| ShipEpochRecord::fresh(hull));
        record
            .check()
            .map_err(|_| integrity("a ship's epoch record cannot move until it is repaired"))?;
        // Idempotent by the record (V1, R3): an open with an epoch open or of the id the record
        // last wrote, a close with none of that id, moves nothing.
        match op {
            EpochOp::Open if record.open.is_some() => {
                return Err(conflict("that ship's epoch is already open"));
            }
            EpochOp::Open if record.last.as_ref().is_some_and(|last| last.id == offer.id) => {
                return Err(conflict("that ship's epoch has already been written"));
            }
            EpochOp::Close if record.open.as_ref().is_none_or(|open| open.id != offer.id) => {
                return Err(conflict("that ship has no such epoch open"));
            }
            EpochOp::Open | EpochOp::Close => {}
        }
        // On the composed path the World is read again: the reading the offer was made from
        // must still hold (the same end of the same epoch, under the same principal and port),
        // and an open's seal is over the unit the World still holds (V6).
        if let Ok(world) = context.world() {
            epoch_due(world, hull, Some(&record))
                .ok()
                .flatten()
                .filter(|due| {
                    due.op == op
                        && due.id == offer.id
                        && due.principal == actor
                        && due.base == offer.base
                        && due.recovered == offer.recovered
                })
                .ok_or_else(|| conflict("that ship's epoch has already moved"))?;
            if op == EpochOp::Open
                && mush_tactics::module::unit_of(world, hull).map(|unit| unit.schema().digest())
                    != offer.unit_definition
            {
                return Err(conflict("that ship's unit changed since its seal"));
            }
        }
        let written = match op {
            EpochOp::Open => {
                if offer.seal.is_none() {
                    tracing::error!(
                        hull = hull.0,
                        "tacsim: a ship left its port with nothing sealed (no TacticalUnit, or \
                         its seal refused); its epoch opens with the seal absent"
                    );
                }
                let from = offer
                    .from
                    .clone()
                    .ok_or_else(|| invalid("a ship's open names the port it left"))?;
                let mut epoch = ShipEpoch {
                    id: offer.id,
                    from,
                    base: offer.base,
                    principal: actor,
                    opened_ic: offer.at_ic,
                    unrecorded: false,
                    seal: offer.seal.clone(),
                };
                let line = BlackBoxLine::ship_sealed(hull, &epoch, offer.class);
                let written = black_box::append(context, Some(&self.black_box), hull, line)?;
                epoch.unrecorded = !written;
                record.open = Some(epoch);
                written
            }
            EpochOp::Close => {
                let Some(epoch) = record.open.take() else {
                    return Err(conflict("that ship has no such epoch open"));
                };
                // `destroyed` is reserved: there is no wreck path for a ship yet. **Doc 23 cut
                // S.3** (A5): a hull docked at a hangar port its own journey did not bring it to
                // was brought in by other means than its own drive (§2 "Recovery") and closes
                // `RecoveredToBase`; the line names no recoverer (A3: a staff placement is a GOD
                // write that leaves no actor on the hull). There is no staff `close` for a ship:
                // a ship's hangar is a port, never destroyed or closed, and D-23-4's abandonment
                // is the rig's; `@tacsim/port clear` stands down a filed clearance before any
                // epoch and is not an end of one.
                let base = offer.base;
                let line = BlackBoxLine::ship_closed(
                    hull,
                    &epoch,
                    offer.at_ic,
                    if offer.recovered {
                        IcCloseKind::RecoveredToBase { base }
                    } else {
                        IcCloseKind::ReturnedToBase { base }
                    },
                );
                black_box::append(context, Some(&self.black_box), hull, line)?
            }
        };
        record.last = Some(EpochLast {
            op,
            id: offer.id,
            at_ic: offer.at_ic,
            written,
        });
        record.attempts = record.attempts.saturating_add(1);
        record
            .check()
            .map_err(|_| integrity("a ship's epoch record is out of bounds"))?;
        let staged = if existed {
            txn.replace(&self.records, &key, record)
        } else {
            txn.insert(&self.records, &key, record)
        };
        staged.map_err(|_| integrity("a ship's epoch record did not stage"))?;
        self.output(invocation, (hull, op, written))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The end words and the key** (doc 23 cut S.2): `open`/`close` round-trip, nothing else
    /// parses, and an epoch key forms under the idempotency grammar.
    #[test]
    fn s2_an_epoch_end_is_open_or_close() {
        for op in [EpochOp::Open, EpochOp::Close] {
            assert_eq!(EpochOp::parse(op.word()), Some(op));
            assert_eq!(serde_json::to_value(op).expect("encodes"), json!(op.word()));
        }
        assert_eq!(EpochOp::parse("Open"), None);
        assert!(
            IdempotencyKeyText::parse_canonical(format!("{EPOCH_KEY_PREFIX}5011/864000/open/1"))
                .is_ok()
        );
        assert_eq!(EpochKey::parse("5011"), Some(EpochKey(ObjId(5011))));
        assert_eq!(EpochKey::parse("05011"), None);
    }

    /// **The record** (the fix round's R1, R3, R8): strict, the open epoch the last end's, every
    /// id its hull's, the count of ends kept; a fresh record holds nothing.
    #[test]
    fn s2_a_ship_epoch_record_is_strict() {
        let hull = ObjId(5011);
        let fresh = ShipEpochRecord::fresh(hull);
        fresh.check().unwrap();
        assert_eq!(
            serde_json::to_value(&fresh).unwrap(),
            json!({"hull": 5011, "attempts": 0})
        );
        let epoch = ShipEpoch {
            id: SortieId::ic(hull, 900),
            from: CanonicalId::new("ceres_port").unwrap(),
            base: ObjId(7),
            principal: ObjId(10),
            opened_ic: 900,
            unrecorded: false,
            seal: None,
        };
        let open = ShipEpochRecord {
            hull,
            open: Some(epoch.clone()),
            last: Some(EpochLast {
                op: EpochOp::Open,
                id: epoch.id,
                at_ic: 900,
                written: true,
            }),
            attempts: 1,
        };
        open.check().unwrap();
        let wire = serde_json::to_value(&open).unwrap();
        assert_eq!(
            serde_json::from_value::<ShipEpochRecord>(wire.clone()).unwrap(),
            open
        );
        let mut extra = wire;
        extra["sortie"] = json!(1);
        assert!(
            serde_json::from_value::<ShipEpochRecord>(extra).is_err(),
            "strict"
        );
        let broken = |edit: &dyn Fn(&mut ShipEpochRecord)| {
            let mut record = open.clone();
            edit(&mut record);
            record.check().is_err()
        };
        assert!(broken(&|r| r.attempts = 0), "a last end is counted");
        assert!(
            broken(&|r| r.last = None),
            "an open epoch is the last end's"
        );
        assert!(
            broken(&|r| {
                if let Some(last) = r.last.as_mut() {
                    last.op = EpochOp::Close;
                }
            }),
            "an open epoch's last end is its open"
        );
        assert!(
            broken(&|r| {
                if let Some(e) = r.open.as_mut() {
                    e.id = SortieId::ic(ObjId(5012), 900);
                }
            }),
            "another hull's epoch"
        );
        assert!(
            broken(&|r| {
                if let Some(e) = r.open.as_mut() {
                    e.opened_ic = 901;
                }
            }),
            "opened when it left"
        );
        assert!(
            broken(&|r| {
                if let Some(e) = r.open.as_mut() {
                    e.base = hull;
                }
            }),
            "into another object"
        );
        assert!(
            broken(&|r| {
                if let Some(last) = r.last.as_mut() {
                    last.id = SortieId::in_match(0, hull);
                }
            }),
            "an arena id"
        );
        let closed = ShipEpochRecord {
            open: None,
            last: Some(EpochLast {
                op: EpochOp::Close,
                id: epoch.id,
                at_ic: 5_000,
                written: true,
            }),
            attempts: 2,
            ..open.clone()
        };
        closed.check().unwrap();
    }
}
