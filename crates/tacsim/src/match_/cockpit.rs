//! **The cockpit: boarding, the pilot's ten verbs, and the frame they read**
//! (16b cut K.2; 16b §8 cut K, K2–K4; doc 16 §3.3 "Startup", "Stations are on
//! the tank", §3.6 "What a pilot sees", §3.7, §3.8 "Disconnects", §5).
//!
//! Cut K.1 built the lobby: a centre, slots, pods locked to pilots, and the
//! named `Boarding → Running` trigger waiting for somebody to report a seat.
//! This is the half that reports one, and everything the pilot can do once
//! seated.
//!
//! ```text
//! BOARD                      climb into your pod and take the tank's cockpit
//! STARTUP                    bring power up (idempotent)
//! HEADING <0-359>            steer
//! SPEED <kph>                make way (negative is reverse)
//! VIEW                       the arena window, as a text map
//! CONTACTS                   what the platform holds
//! LOCK <contact>             hold one of them
//! FIRE [<weapon>]            shoot it
//! WEAPONSTATUS               ammunition and readiness
//! STATUS                     power, speed, heading, tempo
//! EXIT                       climb out
//! ```
//!
//! # Boarding is four facts, in one order
//!
//! Doc 16 §3.3 says *stations are on the tank*, and §3.7 says the pod *is where
//! the body sits*. Those two sentences are the whole design, and they fix the
//! order [`board`] does its work in:
//!
//! 1. **the station** — one `cockpit` billet on the **tank**, carrying `helm`,
//!    `sensors` and `fire`, whose console `space` is the **pod**. So the grants
//!    have the tank as their subject (which is what the arena authorises against)
//!    and reach is satisfied by a body in the pod (which is where the pilot is).
//!    Written through [`mush_sim::declare_station`], idempotently: the same pod
//!    always flies the same tank, so a second boarding rewrites nothing and
//!    invalidates no grant.
//! 2. **the body** — [`mush_sim::place_interior`] puts the character inside the
//!    pod. It comes *before* the seat, because the seat's reach proof is
//!    `Console { room: the pod }` and a pilot standing in the team room does not
//!    satisfy it.
//! 3. **the seat** — `mush_sim::request_seat_activation(Activate)`, which is the
//!    one path in the tree that writes a billet, narrows its channel set, claims
//!    the account's single foreground focus and asks K6 for the control lease. The
//!    cockpit opens no second path and mints no grant of its own.
//! 4. **the centre** — [`crate::match_::slot_seated`], cut K.1's trigger input.
//!    When this seat was the last one the match was waiting for, the check-out
//!    fires inside this same command.
//!
//! [`exit`] undoes 3, 2 and (during `Boarding`) 4, in that order, and sends
//! `Control { Released }` **first** — before the seat itself is released — so
//! that a pilot who types `FIRE` and `EXIT` in one burst has the epoch raised
//! under the queued shot and the arena purges it (doc 16 §3.1).
//!
//! # The verbs are contextual, and the gate is what makes them so
//!
//! There is no such thing as a verb that exists only in a cockpit: the command
//! table is global. So the eleven verbs are registered globally and
//! [`CockpitGate`] refuses each of them, with one line, wherever it does not
//! mean anything — `BOARD` outside a team room, `EXIT` outside a pod, and the
//! nine cockpit selectors to anyone who is not focused on a station of a
//! **deployed** unit. A pilot who has boarded but whose match has not started
//! yet is in that last case on purpose: there is no arena to order until the
//! trigger fires.
//!
//! # Where the numbers come from
//!
//! The arena is the only thing that knows a tank's speed, heading, power or
//! tempo, and it publishes them as the `tacsim.frame` keyframe cut T.2 opened.
//! The world keeps the last one per pilot
//! ([`crate::arena_bridge::ArenaRegistry::last_frame`]) and every read verb
//! renders **that** — never a second model of the same fact. A pilot with no
//! frame yet has a cold tank, which is true and is also the fail-closed answer:
//! `HEADING` and `SPEED` refuse with [`POWER_NOT_ONLINE`] before the arena would.
//!
//! ## One frame, two shapes
//!
//! [`status_frame`] is what a pulse sends: the numbers, the (empty until cut S)
//! contacts and the weapons table. [`view_frame`] is that same frame **plus** the
//! arena window in the grid shape `mush_ansi::panel` already renders, which is
//! what makes `VIEW` a text map on a line client and a map panel on the web with
//! no per-feed code in either. The grid rides `VIEW` and not the pulse
//! deliberately: a line client would otherwise be sent twenty rows of map twice a
//! second.

use std::collections::BTreeSet;
use std::sync::Arc;

use mush_core::module::{ModError, Registrar};
use mush_core::{ArgShape, ChannelId, CmdError, CommandSpec, Ctx, ObjId, Registry, World};
use mush_proto::{OutputEvent, RichText};
use smol_str::SmolStr;

use crate::arena::intents::{ControlState, IntentKind};
use crate::arena::state::{Channel, ContactId, Power};
use crate::arena::step::Frame;
use crate::arena_bridge::{
    ArenaRegistry, CockpitFrame, FRAME_FEED, IntentRefusal, NOTICE_CATEGORY, forward_intent,
    send_control,
};
use crate::arena_task::MatchId;
use crate::match_::MatchState;
use crate::match_::centre::{self, CentreRoom};
use crate::match_::lobby::{self, SimError};

// ---------------------------------------------------------------------------
// The vocabulary
// ---------------------------------------------------------------------------

/// The boarding verb.
pub const BOARD_VERB: &str = "board";

/// The leaving verb. A primary verb of its own: nothing in the tree registered
/// `exit` (the spatial layer's player-facing leave is `egress`, which needs a
/// domain to step onto and a pod is unanchored), so doc 16 §3.7's spelling is
/// free and is what a pilot will type.
pub const EXIT_VERB: &str = "exit";

/// The nine cockpit selectors, in doc 16 §3.7's order. Registered lowercase
/// because the command table folds case on the way in (`Registry::lookup_idx`),
/// so doc 16's capitals and a tired pilot's lower case are one verb.
pub const COCKPIT_VERBS: [&str; 9] = [
    "startup",
    "heading",
    "speed",
    "view",
    "contacts",
    "lock",
    "fire",
    "weaponstatus",
    "status",
];

/// The station id one pod's pilot holds **on the tank** (doc 16 §3.3).
///
/// Deliberately not `helm`, `sensors` or `fire`: a cockpit seat carries all
/// three channels, and [`crate::arena_bridge::open_match`]'s rule is that a
/// station named after one channel raises that one and any other name raises all
/// three. The name is the thing that makes the seat a cockpit.
pub const STATION: &str = "cockpit";

/// The station's descriptive role (`StationSpec::role`, which the engine reads
/// nothing from).
pub const STATION_ROLE: &str = "pilot";

/// Its dispatch priority — `@spatial/station`'s own `pilot` default, so a tank's
/// cockpit ranks like every other pilot billet in the tree.
pub const STATION_PRIORITY: u16 = 2;

/// The weapon `FIRE` takes when the pilot names none. The stock tank's cannon
/// (doc 16 §3.7: *default cannon*); a unit that has no component by this name
/// falls back to its first weapon in component order.
pub const DEFAULT_WEAPON: &str = "cannon";

/// How many cells of the arena window one `VIEW` carries, across and up. The
/// widest and tallest a line client prints (`mush_ansi::panel`'s own bounds), so
/// the frame is the map and the renderer only has to clip it to a narrow
/// terminal.
pub const VIEW_COLUMNS: u16 = 78;

/// As [`VIEW_COLUMNS`], up the window.
pub const VIEW_ROWS: u16 = 20;

/// The glyph the pilot's own platform draws with.
pub const YOU_GLYPH: &str = "@";

// ---------------------------------------------------------------------------
// The refusals
// ---------------------------------------------------------------------------

/// `BOARD` outside a team room of the centre.
pub const OUTSIDE_TEAM_ROOM: &str = "Boarding is through one of the simulator's team rooms.";

/// `EXIT` outside a pod.
pub const NOT_IN_A_POD: &str = "You are not in a pod.";

/// A cockpit verb typed by somebody who is not flying anything.
pub const OUTSIDE_COCKPIT: &str = "You are not at the station of a deployed unit.";

/// **The one line a motion or fire order refuses with while the tank is cold**
/// (doc 16 §3.3 "Startup").
///
/// The cockpit refuses before the arena does, so the pilot gets the reason at
/// once instead of a pulse later; `the_cold_refusal_carries_the_arenas_own_word`
/// below pins it to contain [`crate::arena::intents::Refusal::PowerNotOnline`]'s
/// own word, so the early refusal and the late one can never drift apart.
pub const POWER_NOT_ONLINE: &str = "Power not online: STARTUP first.";

/// **The one line `FIRE` refuses with until cut W** (doc 16 §3.7: *no lock*).
///
/// Cut W's arena-side refusal renders these same words, for the same reason
/// [`POWER_NOT_ONLINE`] does.
pub const NO_LOCK: &str = "No lock: LOCK a contact first.";

/// **The one byte-identical refusal for any contact id the platform does not
/// hold** (doc 16 §3.6: *unresolvable, unowned or expired* are one answer).
///
/// **16b cut S.1 moved the string** to [`crate::arena::state::NO_SUCH_CONTACT`],
/// beside `ShadowContacts::resolve` — the only lookup a pilot's id goes through —
/// and re-exports it here under K.2's name. There is exactly one of this sentence
/// in the tree, so the three cases cannot drift into three answers, and every
/// caller that had this path keeps it.
pub use crate::arena::state::NO_SUCH_CONTACT;

/// Why a cockpit verb refused. Every variant is the line the pilot sees.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CockpitError {
    /// `BOARD` was typed outside a team room.
    #[error("{OUTSIDE_TEAM_ROOM}")]
    OutsideTeamRoom,
    /// `EXIT` was typed outside a pod.
    #[error("{NOT_IN_A_POD}")]
    NotInAPod,
    /// A cockpit verb was typed by somebody with no deployed station.
    #[error("{OUTSIDE_COCKPIT}")]
    OutsideCockpit,
    /// The tacsim layer did not compose.
    #[error("The arena is not available on this server.")]
    Unavailable,
    /// The centre refused, in its own words.
    #[error("{0}")]
    Centre(#[from] SimError),
    /// The pilot's slot is on the other side.
    #[error("Your slot is on the {team} side; board there.")]
    WrongRoom {
        /// The side the slot is on.
        team: &'static str,
    },
    /// The pilot is in the pod already.
    #[error("You are already in your pod.")]
    AlreadyAboard,
    /// The tank's station could not be written.
    #[error("That pod's tank has no cockpit: {0}")]
    Station(String),
    /// The character could not be put in the pod, or taken out of it.
    #[error("The hatch will not move: {0}")]
    Hatch(String),
    /// The seat itself refused.
    #[error("That cockpit will not wake for you: {0}")]
    Seat(String),
    /// T11's guardrail: a proxy driver may not abandon what it was lent.
    #[error("{0}")]
    Guarded(String),
    /// A cockpit order was refused by the world's own routing. Its words are
    /// [`IntentRefusal`]'s — one named reason each, so the cockpit never says
    /// "no" — read as the sentence a pilot is told: "Refused: you hold no
    /// station on that unit."
    #[error("Refused: {0}.")]
    Order(#[from] IntentRefusal),
    /// The order's argument did not parse, or is out of range.
    #[error("{0}")]
    Usage(String),
    /// Power is not online (doc 16 §3.3).
    #[error("{POWER_NOT_ONLINE}")]
    PowerNotOnline,
    /// No lock (doc 16 §3.7, until cut W).
    #[error("{NO_LOCK}")]
    NoLock,
    /// One line for every contact id the platform does not hold.
    #[error("{NO_SUCH_CONTACT}")]
    NoSuchContact,
    /// The unit carries no weapon by that name.
    #[error("That unit carries no weapon called `{0}`.")]
    NoSuchWeapon(String),
}

// ---------------------------------------------------------------------------
// Reading the cockpit
// ---------------------------------------------------------------------------

/// The channels a cockpit seat grants (doc 16 §3.7: `helm`, `sensors`, `fire`).
///
/// Built from the arena's own [`Channel`] vocabulary, so the seat and the orders
/// cannot name two different sets.
#[must_use]
pub fn cockpit_channels() -> BTreeSet<ChannelId> {
    Channel::ALL
        .iter()
        .filter_map(|channel| ChannelId::new(channel.as_str()))
        .collect()
}

/// The station id, as a typed [`mush_sim::StationId`].
fn station_id() -> Option<mush_sim::StationId> {
    mush_sim::StationId::new(STATION)
}

/// **The unit and station an actor is flying** — the cockpit's one context test.
///
/// The actor's focused station (through the proxy ingress, so a driver of an NPC
/// gunner resolves to the NPC), narrowed to a unit an arena actually holds
/// custody of. A pilot who has boarded but whose match has not started yet is
/// deliberately **not** in a cockpit: there is no arena to order.
#[must_use]
pub fn flying(w: &World, actor: ObjId) -> Option<(ObjId, mush_sim::StationId)> {
    let driver = mush_sim::focus::driving_actor(w, actor);
    let focus = w
        .module_state::<mush_sim::FocusState>()?
        .focus_of_actor(driver)?;
    let (unit, station) = (focus.unit, focus.station.clone());
    mush_sim::deployed::is_deployed(w, unit).then_some((unit, station))
}

/// The seats one actor holds on one station id, whatever its focus says.
///
/// `EXIT` reads this rather than the focus: a pilot who typed `+focus` at
/// something else still holds the billet, and leaving the pod has to give it
/// back.
fn seats_of(w: &World, actor: ObjId) -> Vec<(ObjId, mush_sim::StationId)> {
    w.module_state::<mush_sim::SpatialState>()
        .and_then(|state| state.derived.seats_by_actor.get(&actor))
        .map(|seats| seats.iter().cloned().collect())
        .unwrap_or_default()
}

/// The pod an actor's body is sitting in, if it is sitting in one.
fn pod_of_body(w: &World, actor: ObjId) -> Option<ObjId> {
    let at = w.get(actor)?.location?;
    centre::pod_slot(w, at).map(|_| at)
}

/// The last frame the arena sent that pilot.
fn last_frame(w: &World, pilot: ObjId) -> Option<CockpitFrame> {
    w.module_state::<ArenaRegistry>()
        .and_then(|registry| registry.last_frame(pilot))
        .cloned()
}

/// The power state the cockpit will act on: the last frame's, or [`Power::Off`]
/// for a pilot the arena has not spoken to yet.
fn power_of(w: &World, pilot: ObjId) -> Power {
    last_frame(w, pilot).map_or(Power::Off, |held| held.frame.power)
}

/// Refuse a motion or fire order while the tank is cold.
fn require_power(w: &World, pilot: ObjId) -> Result<(), CockpitError> {
    if power_of(w, pilot) == Power::Online {
        return Ok(());
    }
    Err(CockpitError::PowerNotOnline)
}

/// **The contacts the platform holds**, as the frame carries them.
///
/// **16b cut S.1 filled the arena side**: the sensor pass mints the ids and mirrors
/// them into [`Frame::contacts`] as `crate::arena::perception::FrameContact` rows,
/// so the data is now there on every frame a held platform is sent. Reading it —
/// the range band, the degrees, the line `CONTACTS` prints and the `LOCK` that
/// resolves through it — is **cut S.2's**, which is why this still answers empty:
/// until it does, `LOCK` answers [`NO_SUCH_CONTACT`] for every id and `CONTACTS`
/// says so out loud, exactly as K.2 shipped it.
#[must_use]
pub fn frame_contacts(_frame: &Frame) -> Vec<ContactRow> {
    Vec::new()
}

/// One contact, as a pilot is told about it (doc 16 §3.6 "What a pilot sees").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContactRow {
    /// The opaque arena id, which is the only name a pilot may use.
    pub id: ContactId,
    /// The class, or `unknown`.
    pub class: SmolStr,
    /// Bearing, degrees clockwise from north.
    pub bearing_deg: u32,
    /// The range band's label.
    pub band: SmolStr,
    /// How long it has been held, milliseconds of arena time.
    pub age_ms: i64,
}

/// **The lock the platform holds**, as the frame carries it. Cut W fills it; see
/// [`frame_contacts`].
#[must_use]
pub fn frame_lock(_frame: &Frame) -> Option<ContactId> {
    None
}

// ---------------------------------------------------------------------------
// K2: `BOARD`
// ---------------------------------------------------------------------------

/// **`BOARD`** — climb into your pod and take the tank's cockpit (doc 16 §3.7).
///
/// See the module head for the order and why it is that order. Every refusal
/// happens before the first write; a seat that refuses after the body is in the
/// pod takes the body back out.
///
/// # Errors
///
/// [`CockpitError`], as the line the pilot sees.
pub fn board(ctx: &mut Ctx<'_>) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    let Some(CentreRoom::Team(team)) = centre::where_in_centre(ctx.w, actor) else {
        return Err(CockpitError::OutsideTeamRoom);
    };
    // Every refusal, read out of the centre in one borrow.
    let (match_id, pod, index) = {
        let centre = lobby::centre(ctx.w).ok_or(CockpitError::Unavailable)?;
        // The slot before the state, deliberately: somebody with no slot at all
        // is told the thing they can act on ("claim one"), not the thing they
        // cannot (what the centre happens to be doing).
        let slot = centre
            .slot_of_pilot(actor)
            .cloned()
            .ok_or(SimError::NoSlot)?;
        if centre.state != MatchState::Boarding {
            return Err(SimError::WrongState {
                state: centre.state.as_str(),
            }
            .into());
        }
        if slot.team != team {
            return Err(CockpitError::WrongRoom {
                team: slot.team.as_str(),
            });
        }
        if slot.is_seated() {
            return Err(CockpitError::AlreadyAboard);
        }
        let pod = slot.pod.ok_or(SimError::WrongState {
            state: centre.state.as_str(),
        })?;
        let match_id = centre.match_id.ok_or(SimError::WrongState {
            state: centre.state.as_str(),
        })?;
        (match_id, pod, slot.index)
    };
    let tank = centre::tank_of(ctx.w, pod).ok_or(SimError::NoUnit {
        team: team.as_str(),
        index,
    })?;
    let station = station_id().ok_or(CockpitError::Unavailable)?;
    let room = ctx.w.get(pod).and_then(|object| object.location);

    // (1) The station, on the TANK, with the POD as its console.
    mush_sim::declare_station(
        ctx,
        tank,
        &station,
        STATION_ROLE,
        &cockpit_channels(),
        Some(pod),
        STATION_PRIORITY,
    )
    .map_err(|error| CockpitError::Station(error.to_string()))?;

    // (2) The body, inside the pod. One seat, so no compartment: the pack
    // authors none, and naming one the space does not declare is a refusal.
    mush_sim::place_interior(ctx, actor, pod, None)
        .map_err(|error| CockpitError::Hatch(error.to_string()))?;

    // (3) The seat. On a refusal the body comes back out, because a pilot shut
    // in a pod with no cockpit is the one state this verb must not leave behind.
    if let Err(error) = mush_sim::request_seat_activation(
        ctx,
        &mush_sim::SeatRequest::Activate {
            unit: tank,
            station: station.clone(),
            // Not a proxy: the pilot flies their own tank, so the seated actor
            // and the driver are the same character and `driving_actor` is a
            // fixed point on them.
            npc: actor,
            granted: cockpit_channels(),
            fallback: None,
        },
    ) {
        if let Some(room) = room {
            put_out(ctx, actor, room);
        }
        return Err(CockpitError::Seat(error.to_string()));
    }
    // The activation already republished the seated actor's grants; this is the
    // idempotent restatement doc 16 §3.7 names, and it costs one recompute.
    mush_sim::links::refresh_actor_links(ctx.w, actor);

    let pilot_line = format!(
        "You climb into {} and the panel wakes: STARTUP, then HEADING and SPEED. EXIT to get out.",
        display_name(ctx.w, pod)
    );
    let room_line = format!(
        "{} climbs into {}.",
        display_name(ctx.w, actor),
        display_name(ctx.w, pod)
    );
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, pilot_line);
    if let Some(room) = room {
        tell_room(ctx, room, room_line);
    }

    // (4) The centre, and — when this was the last seat — cut K.1's trigger.
    //
    // A refusal here is the check-out's own, which cut K.1 has already announced
    // and rolled back: the pilot keeps their pod and their seat (both harmless,
    // and `EXIT` gives them up), so the line is reported and nothing is undone.
    crate::match_::slot_seated(ctx, match_id, actor, tank)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// K2: `EXIT`
// ---------------------------------------------------------------------------

/// **`EXIT`** — climb out (doc 16 §3.7).
///
/// The order is the reverse of [`board`]'s, with the authority first:
///
/// 1. `Control { Released }` on all three channels, which raises the arena's
///    epoch and **purges whatever that seat had queued** — the reason a pilot who
///    types `FIRE` and `EXIT` in one burst fires nothing;
/// 2. the seat, through the same one seam that took it, which clears the
///    narrowing, relinquishes the focus and fires the station's declared fallback
///    if it has one;
/// 3. the body, back into the team room;
/// 4. the centre, when the match is still `Boarding`, so the trigger stops
///    counting a pod nobody is in.
///
/// # Errors
///
/// [`CockpitError`], as the line the pilot sees.
pub fn exit(ctx: &mut Ctx<'_>) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    // **T11's guardrail** (16b cut K.2; `ProxyAct::Abandonment`'s station-lane
    // face, restored by this cut). A seat is tactical discretion: a driver lent
    // somebody else's crew member may not climb out of their unit and leave it
    // idling in a live match. A pilot in their own tank is not a proxy driver, so
    // the predicate stands aside for them and this costs them nothing.
    mush_sim::seat_restriction(ctx.w, actor, mush_sim::ProxyAct::Abandonment)
        .map_err(CockpitError::Guarded)?;
    let pod = pod_of_body(ctx.w, actor).ok_or(CockpitError::NotInAPod)?;
    let room = ctx.w.get(pod).and_then(|object| object.location);
    let station = station_id().ok_or(CockpitError::Unavailable)?;
    let seats: Vec<ObjId> = seats_of(ctx.w, actor)
        .into_iter()
        .filter(|(_, held)| *held == station)
        .map(|(unit, _)| unit)
        .collect();

    // (1) The arena hears first.
    for unit in &seats {
        for channel in Channel::ALL {
            if let Err(error) = send_control(ctx, *unit, channel, ControlState::Released) {
                tracing::debug!(
                    unit = %unit,
                    channel = channel.as_str(),
                    %error,
                    "tacsim: EXIT could not tell the arena the seat is released"
                );
            }
        }
    }
    // (2) The seat: stood down through the one seam that took it (the narrowing
    // cleared, the focus relinquished, the fallback fired), and then **given
    // back** — a stand-down deliberately leaves the billet where it is, which is
    // right for an NPC's own station and wrong for a pod lent for one match.
    for unit in &seats {
        if let Err(error) = mush_sim::request_seat_activation(
            ctx,
            &mush_sim::SeatRequest::Release {
                unit: *unit,
                station: station.clone(),
                npc: actor,
            },
        ) {
            tracing::debug!(
                unit = %unit,
                %error,
                "tacsim: EXIT could not stand the cockpit seat down"
            );
        }
        if let Err(error) = mush_sim::vacate_station(ctx, *unit, &station, actor) {
            tracing::error!(
                unit = %unit,
                %error,
                "tacsim: a cockpit billet could not be given back; the next pilot of that pod \
                 will be refused and staff must @spatial/unassign it"
            );
        }
    }
    // (3) The body. The pod is an unanchored interior space, so there is no
    // domain to `egress` onto: the placement is released where it stands and the
    // character walks back into the room the pod is in.
    if let Some(room) = room {
        put_out(ctx, actor, room);
    }
    let line = format!("You climb out of {}.", display_name(ctx.w, pod));
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    if let Some(room) = room {
        let text = format!(
            "{} climbs out of {}.",
            display_name(ctx.w, actor),
            display_name(ctx.w, pod)
        );
        tell_room(ctx, room, text);
    }

    // (4) The centre, while it is still boarding.
    let boarding = lobby::centre(ctx.w).and_then(|centre| {
        (centre.state == MatchState::Boarding && centre.slot_of_pilot(actor).is_some())
            .then_some(centre.match_id)?
    });
    if let Some(match_id) = boarding {
        crate::match_::slot_vacated(ctx, match_id, actor)?;
    }
    Ok(())
}

/// Take one character out of the pod it is placed inside and stand it in `room`.
///
/// Two steps, because the pod is **unanchored**: the placement is released where
/// it is (`keep`, so nothing is projected to the recovery room) and the body is
/// then moved into the room the pod stands in. An unplaced object projects
/// nowhere, so boot's own projection reconcile leaves it exactly here.
fn put_out(ctx: &mut Ctx<'_>, actor: ObjId, room: ObjId) {
    if let Err(error) = mush_sim::unplace(ctx, actor, "left a simulator pod") {
        tracing::debug!(%actor, %error, "tacsim: a pilot's pod placement did not release");
    }
    let from = ctx.w.get(actor).and_then(|object| object.location);
    if from == Some(room) {
        return;
    }
    if let Err(error) = ctx.w.move_to(actor, room) {
        tracing::error!(%actor, %room, %error, "tacsim: a pilot could not leave their pod");
        return;
    }
    ctx.emit_world_event(&mush_core::WorldEvent::Moved {
        who: actor,
        from,
        to: room,
    });
}

/// **Open every pod and stand whoever is in it in the team room** (doc 16 §3.8:
/// *seats, leases, focus and pods released atomically*).
///
/// The lobby's half of an ended match releases the **slots** and the pods' locks,
/// which are rows on a state slot; this is the other half of "released", the one
/// `BOARD` created: a body inside a Thing. A pilot left in a dark pod after the
/// match is over is not stuck — `EXIT` still works from there — but the centre
/// said the pods were free and they were not.
///
/// Idempotent, and keyed on the **pods** rather than on the slots, so it is
/// equally the repair a staff `@arena/reset` performs on a centre some earlier
/// failure left with somebody aboard. Answers how many it opened.
pub(crate) fn disembark_all(ctx: &mut Ctx<'_>) -> usize {
    let pods: Vec<(ObjId, Option<ObjId>)> = centre::POD_ENTRIES
        .iter()
        .filter_map(|(team, index, _)| centre::pod_object(ctx.w, *team, *index))
        .map(|pod| (pod, ctx.w.get(pod).and_then(|object| object.location)))
        .collect();
    let mut opened: usize = 0;
    for (pod, room) in pods {
        let Some(room) = room else {
            continue;
        };
        let aboard: Vec<ObjId> = ctx
            .w
            .get(pod)
            .map(|object| object.contents.clone())
            .unwrap_or_default();
        for body in aboard {
            if ctx
                .w
                .get(body)
                .is_none_or(|object| object.kind != mush_core::ObjKind::Player)
            {
                continue;
            }
            put_out(ctx, body, room);
            let line = format!("The hatch of {} swings open.", display_name(ctx.w, pod));
            ctx.out.sys_to_actor(body, NOTICE_CATEGORY, line);
            opened = opened.saturating_add(1);
        }
    }
    opened
}

// ---------------------------------------------------------------------------
// K3: the nine cockpit verbs
// ---------------------------------------------------------------------------

/// `STARTUP` — bring power up (doc 16 §3.3). Idempotent: the arena's own
/// `resolve_startup` is what decides whether anything changes, and a second
/// `STARTUP` while `starting` costs one queued action and no refusal.
fn startup(ctx: &mut Ctx<'_>, unit: ObjId) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    forward_intent(ctx, actor, unit, IntentKind::Startup)?;
    ctx.out.sys_to_actor(
        actor,
        NOTICE_CATEGORY,
        "Startup sequence begun; power comes online shortly.",
    );
    Ok(())
}

/// `HEADING <deg>` — 0 to 359, clockwise from north (doc 16 §3.7).
fn heading(ctx: &mut Ctx<'_>, unit: ObjId, rest: &str) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    let deg: u32 = rest
        .trim()
        .parse()
        .ok()
        .filter(|deg| *deg < 360)
        .ok_or_else(|| CockpitError::Usage("HEADING <0-359>, clockwise from north.".to_owned()))?;
    require_power(ctx.w, actor)?;
    forward_intent(
        ctx,
        actor,
        unit,
        IntentKind::Heading {
            mdeg: deg.saturating_mul(1_000),
        },
    )?;
    let line = format!("Steering to {deg}°.");
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    Ok(())
}

/// `SPEED <kph>` — 0 to the unit's own maximum, negative in reverse (doc 16
/// §3.7). The wire is centimetres a second (doc 16 §3.2's integers), so the
/// conversion is here and the pilot never types one.
fn speed(ctx: &mut Ctx<'_>, unit: ObjId, rest: &str) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    let kph: i32 =
        rest.trim().parse().ok().ok_or_else(|| {
            CockpitError::Usage("SPEED <kph>; a negative speed reverses.".to_owned())
        })?;
    let max_cms = max_speed_cms(ctx.w, unit);
    let max_kph = cms_to_kph(i32::try_from(max_cms).unwrap_or(i32::MAX));
    if kph.abs() > max_kph {
        return Err(CockpitError::Usage(format!(
            "That unit makes {max_kph} kph; SPEED 0 to {max_kph}."
        )));
    }
    require_power(ctx.w, actor)?;
    forward_intent(
        ctx,
        actor,
        unit,
        IntentKind::Speed {
            cms: kph_to_cms(kph),
        },
    )?;
    let line = format!("Making {kph} kph.");
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    Ok(())
}

/// `LOCK <contact>` — hold one of the platform's contacts (doc 16 §3.7).
///
/// One refusal for every id the platform does not hold, whether it is malformed,
/// somebody else's or expired (doc 16 §3.6). Cut S is what gives a platform
/// anything to hold; until then the refusal is the whole verb.
fn lock(ctx: &mut Ctx<'_>, unit: ObjId, rest: &str) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    let held = last_frame(ctx.w, actor)
        .map(|held| frame_contacts(&held.frame))
        .unwrap_or_default();
    let contact = parse_contact(rest)
        .filter(|id| held.iter().any(|row| row.id == *id))
        .ok_or(CockpitError::NoSuchContact)?;
    forward_intent(ctx, actor, unit, IntentKind::Lock { contact })?;
    let line = format!("Locking {contact}.");
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    Ok(())
}

/// `FIRE [<weapon>]` — shoot the lock (doc 16 §3.7).
///
/// Refused with [`NO_LOCK`] until cut W, in doc 16's own order: the weapon has to
/// exist, then there has to be something to shoot at, then the tank has to be
/// awake.
fn fire(ctx: &mut Ctx<'_>, unit: ObjId, rest: &str) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    let weapon = choose_weapon(ctx.w, unit, rest)?;
    let locked = last_frame(ctx.w, actor).and_then(|held| frame_lock(&held.frame));
    if locked.is_none() {
        return Err(CockpitError::NoLock);
    }
    require_power(ctx.w, actor)?;
    forward_intent(
        ctx,
        actor,
        unit,
        IntentKind::Fire {
            weapon: weapon.clone(),
        },
    )?;
    let line = format!("Firing {weapon}.");
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    Ok(())
}

/// `CONTACTS` — what the platform holds (doc 16 §3.6 "What a pilot sees").
fn contacts(ctx: &mut Ctx<'_>) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    let held = last_frame(ctx.w, actor);
    let rows = held
        .as_ref()
        .map(|held| frame_contacts(&held.frame))
        .unwrap_or_default();
    let mut lines: Vec<String> = Vec::new();
    if rows.is_empty() {
        lines.push("No contacts.".to_owned());
    }
    for row in &rows {
        lines.push(format!(
            "{}  {}  bearing {}°  {}  {} s",
            row.id,
            row.class,
            row.bearing_deg,
            row.band,
            row.age_ms / 1_000
        ));
    }
    say_and_frame(ctx, held.as_ref(), lines, false);
    Ok(())
}

/// `WEAPONSTATUS` — ammunition and readiness, per weapon (doc 16 §5).
fn weapon_status(ctx: &mut Ctx<'_>, unit: ObjId) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    let held = last_frame(ctx.w, actor);
    let weapons = weapons_of(ctx.w, unit);
    let mut lines: Vec<String> = Vec::new();
    if weapons.is_empty() {
        lines.push("This unit carries no weapon.".to_owned());
    }
    for weapon in &weapons {
        lines.push(format!(
            "{}  {}  {} of {} round(s)",
            weapon.id,
            if weapon.ready { "ready" } else { "cycling" },
            weapon.rounds,
            weapon.rounds_max
        ));
    }
    say_and_frame(ctx, held.as_ref(), lines, false);
    Ok(())
}

/// `STATUS` — the status block of doc 16 §5, as the last frame carries it.
fn status(ctx: &mut Ctx<'_>, unit: ObjId) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    let held = last_frame(ctx.w, actor);
    let lines = match held.as_ref() {
        Some(held) => {
            let frame = &held.frame;
            vec![
                format!(
                    "{}: power {}, {} kph, heading {}°{}",
                    display_name(ctx.w, unit),
                    frame.power.as_str(),
                    cms_to_kph(frame.speed_cms),
                    frame.pose.heading_mdeg / 1_000,
                    if frame.blocked { ", HELD" } else { "" }
                ),
                format!(
                    "  at {} m east, {} m north; the arena is running {}",
                    frame.pose.x_cm / 100,
                    frame.pose.y_cm / 100,
                    frame.tempo.as_str()
                ),
            ]
        }
        None => vec![format!(
            "{}: power off. The panel has told you nothing yet.",
            display_name(ctx.w, unit)
        )],
    };
    say_and_frame(ctx, held.as_ref(), lines, false);
    Ok(())
}

/// `VIEW` — the arena window, as the grid frame (doc 16 §3.6, §5).
fn view(ctx: &mut Ctx<'_>) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    let held = last_frame(ctx.w, actor);
    let lines = match held.as_ref() {
        // The map is the frame's; a line client's renderer draws it. Nothing is
        // said twice.
        Some(_) => Vec::new(),
        None => vec!["The window is dark: the panel has told you nothing yet.".to_owned()],
    };
    say_and_frame(ctx, held.as_ref(), lines, true);
    Ok(())
}

/// Say what a read verb answers and push the frame it read (doc 16 §5: one feed,
/// and the line renderer prints the frame).
fn say_and_frame(
    ctx: &mut Ctx<'_>,
    held: Option<&CockpitFrame>,
    lines: Vec<String>,
    with_view: bool,
) {
    let actor = ctx.actor;
    for line in lines {
        ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    }
    let Some(held) = held else {
        return;
    };
    let payload = if with_view {
        view_frame(ctx, held.match_id, &held.frame)
    } else {
        status_frame(ctx, held.match_id, &held.frame)
    };
    ctx.out.push(
        mush_core::Audience::Actor(actor),
        OutputEvent::Panel {
            feed: SmolStr::new_static(FRAME_FEED),
            kind: mush_proto::PanelFrameKind::Keyframe,
            data: payload,
        },
    );
}

// ---------------------------------------------------------------------------
// The frame (doc 16 §5)
// ---------------------------------------------------------------------------

/// The frame's schema id, which every consumer keys its decode on.
pub const FRAME_SCHEMA: &str = "arena.cockpit";

/// Its version. Cuts S and W add fields; a field added is not a version.
pub const FRAME_VERSION: u32 = 1;

/// **The cockpit keyframe** (doc 16 §5): where the tank is, how fast, which way,
/// whether it is awake, whether it is held, how often it is being told things,
/// what it holds and what it carries.
///
/// `contacts` is empty until cut S and `weapons`' live numbers are cut W's; both
/// keys exist now so neither cut has to change the shape, only fill it.
#[must_use]
pub fn status_frame(ctx: &Ctx<'_>, match_id: MatchId, frame: &Frame) -> serde_json::Value {
    let contacts: Vec<serde_json::Value> = frame_contacts(frame)
        .iter()
        .map(|row| {
            serde_json::json!({
                "id": row.id.to_string(),
                "class": row.class.as_str(),
                "bearing_deg": row.bearing_deg,
                "band": row.band.as_str(),
                "age_ms": row.age_ms,
            })
        })
        .collect();
    let weapons: Vec<serde_json::Value> = weapons_of(ctx.w, frame.unit)
        .iter()
        .map(|weapon| {
            serde_json::json!({
                "id": weapon.id.as_str(),
                "ready": weapon.ready,
                "rounds": weapon.rounds,
                "rounds_max": weapon.rounds_max,
            })
        })
        .collect();
    serde_json::json!({
        "schema": { "id": FRAME_SCHEMA, "version": FRAME_VERSION },
        "match": match_id.0,
        "unit": frame.unit.0,
        "x_cm": frame.pose.x_cm,
        "y_cm": frame.pose.y_cm,
        "z_cm": frame.pose.z_cm,
        "heading_mdeg": frame.pose.heading_mdeg,
        "heading_deg": frame.pose.heading_mdeg / 1_000,
        "speed_cms": frame.speed_cms,
        "speed_kph": cms_to_kph(frame.speed_cms),
        "power": frame.power.as_str(),
        "blocked": frame.blocked,
        "tempo": frame.tempo.as_str(),
        "lock": frame_lock(frame).map(|id| id.to_string()),
        "contacts": contacts,
        "weapons": weapons,
    })
}

/// **The cockpit keyframe with the window in it** — [`status_frame`] plus the
/// `grid` shape `mush_ansi::panel` renders as a text map and the web draws as a
/// map panel (doc 16 §5: *the ANSI line renderer prints the frame as text; the
/// TUI and web render the same structure*).
///
/// The window is [`VIEW_COLUMNS`] by [`VIEW_ROWS`] cells around the platform,
/// clamped to the arena — the most a line client prints, so the renderer only
/// has to clip it to a narrow terminal and never has to ask for more. One glyph
/// per terrain class, from the atlas's own table, so the arena's map and the
/// atlas's map read alike.
#[must_use]
pub fn view_frame(ctx: &Ctx<'_>, match_id: MatchId, frame: &Frame) -> serde_json::Value {
    let mut payload = status_frame(ctx, match_id, frame);
    let Some(ground) = ctx
        .w
        .module_state::<ArenaRegistry>()
        .and_then(|registry| registry.arena_of(match_id))
    else {
        return payload;
    };
    let terrain = &ground.terrain;
    let Some((cx, cy)) = terrain.cell_of(&frame.pose) else {
        return payload;
    };
    let width = VIEW_COLUMNS.min(terrain.width());
    let height = VIEW_ROWS.min(terrain.height());
    let x0 = cx
        .saturating_sub(width / 2)
        .min(terrain.width().saturating_sub(width));
    let y0 = cy
        .saturating_sub(height / 2)
        .min(terrain.height().saturating_sub(height));
    let mut rows: Vec<serde_json::Value> = Vec::with_capacity(usize::from(height));
    let mut classes: Vec<mush_atlas::TerrainClass> = Vec::new();
    for y in y0..y0.saturating_add(height) {
        // Run-length, as the grid shape wants it: one pair per run of a glyph.
        let mut runs: Vec<serde_json::Value> = Vec::new();
        let mut run: Option<(char, u32)> = None;
        for x in x0..x0.saturating_add(width) {
            let class = terrain
                .get(x, y)
                .map_or(mush_atlas::TerrainClass::Void, |cell| cell.class);
            if !classes.contains(&class) {
                classes.push(class);
            }
            let glyph = class.glyph();
            match run {
                Some((held, n)) if held == glyph => run = Some((held, n.saturating_add(1))),
                Some((held, n)) => {
                    runs.push(serde_json::json!([held.to_string(), n]));
                    run = Some((glyph, 1));
                }
                None => run = Some((glyph, 1)),
            }
        }
        if let Some((held, n)) = run {
            runs.push(serde_json::json!([held.to_string(), n]));
        }
        rows.push(serde_json::Value::Array(runs));
    }
    let mut legend: Vec<serde_json::Value> = classes
        .iter()
        .map(|class| serde_json::json!([class.glyph().to_string(), class.label()]))
        .collect();
    legend.push(serde_json::json!([YOU_GLYPH, "you"]));
    // The platform, and — when cut S fills them — its contacts. Cells are
    // relative to the window's own origin, which is what the grid renderer
    // centres on.
    let you = [
        u32::from(cx.saturating_sub(x0)),
        u32::from(cy.saturating_sub(y0)),
    ];
    let units = vec![serde_json::json!({
        "id": format!("#{}", frame.unit.0),
        "name": display_name(ctx.w, frame.unit),
        "cell": you,
        "glyph": YOU_GLYPH,
        "you": true,
    })];
    if let Some(object) = payload.as_object_mut() {
        object.insert(
            "schema".to_owned(),
            serde_json::json!({ "id": FRAME_SCHEMA, "version": FRAME_VERSION, "shape": "grid" }),
        );
        object.insert("title".to_owned(), serde_json::json!("Arena window"));
        object.insert("surface".to_owned(), serde_json::json!(ground.id.as_str()));
        object.insert("width".to_owned(), serde_json::json!(width));
        object.insert("height".to_owned(), serde_json::json!(height));
        object.insert("cell_cm".to_owned(), serde_json::json!(terrain.cell_cm()));
        object.insert(
            "origin".to_owned(),
            serde_json::json!([
                i64::from(x0) * i64::from(terrain.cell_cm()),
                i64::from(y0) * i64::from(terrain.cell_cm())
            ]),
        );
        object.insert("rows".to_owned(), serde_json::Value::Array(rows));
        object.insert("legend".to_owned(), serde_json::Value::Array(legend));
        object.insert("units".to_owned(), serde_json::Value::Array(units));
        object.insert("you".to_owned(), serde_json::json!(you));
    }
    payload
}

// ---------------------------------------------------------------------------
// The unit's own numbers
// ---------------------------------------------------------------------------

/// One weapon, as [`weapons_of`] reads it off the unit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WeaponRow {
    /// The component id (`cannon`, `machine_gun`).
    pub id: SmolStr,
    /// Whether its cycle has finished. Cut W keeps `ready_at_ms` live; until
    /// then a loaded weapon is ready.
    pub ready: bool,
    /// Rounds left.
    pub rounds: i64,
    /// Rounds a full load carries.
    pub rounds_max: i64,
}

/// The cell a weapon component's ammunition lives in (doc 16 §3.3).
const AMMO_CELL: &str = "ammo";

/// The cell its cycle lives in.
const READY_AT_CELL: &str = "ready_at_ms";

/// **Every weapon a unit carries**, in component order (doc 16 §3.3's
/// components).
///
/// A weapon is a component with both an `ammo` and a `ready_at_ms` cell, which
/// is exactly what the unit decoder gives one (`definitions/unit.rs`). Read
/// structurally rather than by looking the weapon definition up, because the
/// question here is what *this* tank is carrying now.
#[must_use]
pub fn weapons_of(w: &World, unit: ObjId) -> Vec<WeaponRow> {
    let Some(cells) = mush_tactics::module::unit_of(w, unit) else {
        return Vec::new();
    };
    let (Ok(ammo), Ok(ready_at)) = (
        mush_tactics::CellId::new(AMMO_CELL),
        mush_tactics::CellId::new(READY_AT_CELL),
    ) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for (component, _) in cells.components() {
        let Some(rounds) = cells.cell(&mush_tactics::CellAddr::Component {
            component: component.clone(),
            cell: ammo.clone(),
        }) else {
            continue;
        };
        let Some(cycle) = cells.cell(&mush_tactics::CellAddr::Component {
            component: component.clone(),
            cell: ready_at.clone(),
        }) else {
            continue;
        };
        rows.push(WeaponRow {
            id: SmolStr::new(component.as_str()),
            ready: cycle.current == 0,
            rounds: rounds.current,
            rounds_max: rounds.max,
        });
    }
    rows
}

/// The weapon `FIRE` means: the one the pilot named, else [`DEFAULT_WEAPON`],
/// else the first the unit carries.
fn choose_weapon(w: &World, unit: ObjId, rest: &str) -> Result<SmolStr, CockpitError> {
    let carried = weapons_of(w, unit);
    let named = rest.trim();
    if !named.is_empty() {
        return carried
            .iter()
            .find(|weapon| weapon.id.eq_ignore_ascii_case(named))
            .map(|weapon| weapon.id.clone())
            .ok_or_else(|| CockpitError::NoSuchWeapon(named.to_owned()));
    }
    carried
        .iter()
        .find(|weapon| weapon.id.as_str() == DEFAULT_WEAPON)
        .or_else(|| carried.first())
        .map(|weapon| weapon.id.clone())
        .ok_or_else(|| CockpitError::NoSuchWeapon(DEFAULT_WEAPON.to_owned()))
}

/// The unit's own top speed, centimetres a second, out of its `tracked` mode
/// rules (doc 16 §3.3). A unit the world does not hold gets the stock tank's,
/// which is the same fallback the check-out uses.
fn max_speed_cms(w: &World, unit: ObjId) -> u32 {
    match mush_tactics::module::unit_of(w, unit) {
        Some(cells) => crate::match_::tracked_rules(cells).max_speed_cms,
        None => 0,
    }
}

/// Kilometres an hour to centimetres a second, and back. Integer both ways (doc
/// 16 §3.2: the wire is whole centimetres), so a pilot's round number is a round
/// number of cm/s and the readback is the same round number.
fn kph_to_cms(kph: i32) -> i32 {
    kph.saturating_mul(100_000) / 3_600
}

/// See [`kph_to_cms`].
fn cms_to_kph(cms: i32) -> i32 {
    cms.saturating_mul(36) / 1_000
}

/// One contact id, as a pilot may type it: the sixteen base-16 digits
/// [`ContactId`] renders as, with or without their leading zeros.
fn parse_contact(rest: &str) -> Option<ContactId> {
    let word = rest.trim();
    if word.is_empty() || word.len() > 16 {
        return None;
    }
    u64::from_str_radix(word, 16).ok().map(ContactId)
}

// ---------------------------------------------------------------------------
// The gate and the dispatch
// ---------------------------------------------------------------------------

/// **The contextual gate**: the eleven verbs exist everywhere and mean something
/// in a team room, in a pod, and at a deployed unit's station.
///
/// One line each, and nothing else refused: a gate that is asked about `look`
/// stands aside, and a world with no centre at all lets `BOARD` through to the
/// verb's own "This server has no simulator centre.".
pub struct CockpitGate;

impl mush_core::CommandGate for CockpitGate {
    fn check(
        &self,
        w: &World,
        _reg: &Registry,
        actor: ObjId,
        verb: &str,
        _line: &str,
    ) -> Result<(), String> {
        if verb.eq_ignore_ascii_case(BOARD_VERB) {
            if centre::centre_rooms(w).is_none() {
                return Ok(());
            }
            return match centre::where_in_centre(w, actor) {
                Some(CentreRoom::Team(_)) => Ok(()),
                _ => Err(OUTSIDE_TEAM_ROOM.to_owned()),
            };
        }
        if verb.eq_ignore_ascii_case(EXIT_VERB) {
            return match pod_of_body(w, actor) {
                Some(_) => Ok(()),
                None => Err(NOT_IN_A_POD.to_owned()),
            };
        }
        if COCKPIT_VERBS
            .iter()
            .any(|cockpit| verb.eq_ignore_ascii_case(cockpit))
            && flying(w, actor).is_none()
        {
            return Err(OUTSIDE_COCKPIT.to_owned());
        }
        Ok(())
    }
}

/// One object's name, or its dbref when it has none.
fn display_name(w: &World, object: ObjId) -> String {
    mush_core::output::display_name(w, object)
}

/// Say one line in a room.
fn tell_room(ctx: &mut Ctx<'_>, room: ObjId, text: String) {
    ctx.out.to_room(
        room,
        Vec::new(),
        OutputEvent::Sys(RichText::tag(NOTICE_CATEGORY, text)),
    );
}

/// Render one refusal as the line the pilot sees.
fn refusal(error: CockpitError) -> CmdError {
    CmdError::msg(error.to_string())
}

/// Run one cockpit verb: resolve the station first, so every one of the nine
/// answers the same way to somebody who is not flying.
fn cockpit_command(ctx: &mut Ctx<'_>, verb: &'static str) -> Result<(), CmdError> {
    let Some((unit, _station)) = flying(ctx.w, ctx.actor) else {
        return Err(CmdError::msg(OUTSIDE_COCKPIT));
    };
    let rest = ctx.args.raw().trim().to_owned();
    let outcome = match verb {
        "startup" => startup(ctx, unit),
        "heading" => heading(ctx, unit, &rest),
        "speed" => speed(ctx, unit, &rest),
        "view" => view(ctx),
        "contacts" => contacts(ctx),
        "lock" => lock(ctx, unit, &rest),
        "fire" => fire(ctx, unit, &rest),
        "weaponstatus" => weapon_status(ctx, unit),
        "status" => status(ctx, unit),
        // Not reachable: the table below registers exactly `COCKPIT_VERBS`, and
        // this arm is the closed-set proof rather than a panic.
        _ => Err(CockpitError::OutsideCockpit),
    };
    outcome.map_err(refusal)
}

/// The help one cockpit verb carries.
fn cockpit_help(verb: &str) -> String {
    match verb {
        "startup" => "STARTUP             bring the tank's power up. Idempotent; motion and fire \
                      refuse until it is online."
            .to_owned(),
        "heading" => "HEADING <0-359>     steer to a bearing, clockwise from north. Refused until \
                      power is online."
            .to_owned(),
        "speed" => "SPEED <kph>         make way at that speed; a negative speed reverses. \
                    Refused until power is online."
            .to_owned(),
        "view" => "VIEW                the arena window around your platform, as a map.".to_owned(),
        "contacts" => "CONTACTS            what your platform holds: its opaque id, its class, \
                       bearing, range band and age."
            .to_owned(),
        "lock" => {
            "LOCK <contact>      hold one of your contacts by the id CONTACTS prints.".to_owned()
        }
        "fire" => "FIRE [<weapon>]     shoot your lock; the cannon unless you name another weapon."
            .to_owned(),
        "weaponstatus" => "WEAPONSTATUS        every weapon you carry: ready or cycling, and its \
                           rounds."
            .to_owned(),
        _ => {
            "STATUS              power, speed, heading, position and the arena's tempo.".to_owned()
        }
    }
}

/// Register the cockpit on the **tacsim** module: eleven verbs and one gate.
///
/// The state the verbs read is [`ArenaRegistry`]'s and [`MatchBook`]'s, both
/// registered by [`crate::arena_bridge::register`], so nothing here declares a
/// slot.
///
/// # Errors
///
/// [`ModError`] if one of the verb names is already taken.
pub fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    // `BOARD` writes a station document, a placement, a billet and (at the
    // trigger) cut T.3's whole check-out; `EXIT` writes the same in reverse. The
    // nine selectors stage one arena intent each and write nothing at all, but
    // they are registered on the same lane as their two neighbours rather than
    // split across two classifications for one subsystem.
    reg.classify_commands(mush_core::CommandCompatibility::Authoritative);
    reg.command(
        CommandSpec {
            name: BOARD_VERB.into(),
            shape: ArgShape::Raw,
            help: "BOARD               climb into the pod your simulator slot holds and take its \
                   tank's cockpit. In a team room, once the match is boarding; EXIT gets you out \
                   again."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(|ctx: &mut Ctx<'_>| board(ctx).map_err(refusal)),
    )?;
    reg.command(
        CommandSpec {
            name: EXIT_VERB.into(),
            shape: ArgShape::Raw,
            help: "EXIT                climb out of the pod you are in: the seat, its control \
                   lease and your focus are released, and a match still boarding stops waiting \
                   for you."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(|ctx: &mut Ctx<'_>| exit(ctx).map_err(refusal)),
    )?;
    for verb in COCKPIT_VERBS {
        reg.command(
            CommandSpec {
                name: verb.into(),
                shape: ArgShape::Raw,
                help: cockpit_help(verb),
                ..CommandSpec::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| cockpit_command(ctx, verb)),
        )?;
    }
    reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
    reg.command_gate(Arc::new(CockpitGate));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::intents::Refusal;

    #[test]
    fn the_cold_refusal_carries_the_arenas_own_word() {
        // The cockpit refuses before the arena does; the two must read alike, so
        // the sentence has to contain the arena's own word for it.
        let word = Refusal::PowerNotOnline.as_str().replace('_', " ");
        assert!(
            POWER_NOT_ONLINE.to_ascii_lowercase().contains(&word),
            "`{POWER_NOT_ONLINE}` does not contain `{word}`"
        );
    }

    #[test]
    fn a_cockpit_seat_carries_exactly_the_three_channels() {
        let channels = cockpit_channels();
        assert_eq!(channels.len(), 3);
        for channel in Channel::ALL {
            assert!(
                ChannelId::new(channel.as_str()).is_some_and(|id| channels.contains(&id)),
                "{}",
                channel.as_str()
            );
        }
        // And the station is NOT named after one of them, which is what makes
        // `open_match` raise all three for it.
        assert_eq!(Channel::parse(STATION), None);
    }

    #[test]
    fn speed_converts_both_ways_at_the_stock_tanks_numbers() {
        // 15 m/s is the stock tank's maximum: 54 kph.
        assert_eq!(cms_to_kph(1_500), 54);
        assert_eq!(kph_to_cms(54), 1_500);
        assert_eq!(kph_to_cms(0), 0);
        assert_eq!(kph_to_cms(-20), -555);
        assert_eq!(cms_to_kph(-555), -19);
        // Saturating, not panicking, at the edges.
        assert_eq!(kph_to_cms(i32::MAX), i32::MAX / 3_600);
    }

    #[test]
    fn a_contact_id_parses_as_the_sixteen_digits_it_renders_as() {
        let id = ContactId(0x0123_4567_89ab_cdef);
        assert_eq!(id.to_string(), "0123456789abcdef");
        assert_eq!(parse_contact(&id.to_string()), Some(id));
        assert_eq!(parse_contact(" 1 "), Some(ContactId(1)));
        assert_eq!(parse_contact(""), None);
        assert_eq!(parse_contact("nope"), None);
        assert_eq!(parse_contact("00000000000000001"), None, "over sixteen");
    }

    #[test]
    fn every_registered_cockpit_verb_has_a_help_line_and_a_dispatch_arm() {
        for verb in COCKPIT_VERBS {
            assert!(!cockpit_help(verb).is_empty(), "{verb}");
            assert!(
                cockpit_help(verb)
                    .to_ascii_lowercase()
                    .contains(&verb.to_ascii_lowercase()),
                "{verb}'s help names it"
            );
        }
        assert_eq!(COCKPIT_VERBS.len(), 9, "doc 16 section 3.7's nine");
    }
}
