//! **Crew state and extraction** (doc 32 cut CF.5; Q-CF-38, Q-CF-39; doc 27 Q-H12, Q-F4, P-5,
//! FO-12; the rules applied in doc 32 §8).
//!
//! **The record** (Q-CF-38): two fields on a character, a **location** — aboard a unit, aboard
//! a carrier, stranded on a site, at a port — and a **condition** — fit, injured,
//! incapacitated — on the module's own lane (`tacsim:crew:<character>`, [`CrewRecord`]). It is
//! the field engine's record of a survivor: a character the field engine never touched has
//! none, and reads as nothing to report. It is set
//!
//! - **at every field check-in** (`check_in`): a demotion, a void and an exit at the ramp
//!   each derive it for the unit's pilot — physically inside the hull, the hull's own state
//!   decides (a hull that cannot return under its own power, or a wreck, strands its pilot on
//!   the site); at a remote console aboard a carrier, the carrier — unless the carrier's hull
//!   fell, which strands everyone aboard it (doc 36 cut GC.3, `crate::carrier_damage`); and
//! - **immediately on a control-section kill** (`on_frames`): a pilot inside a hull whose
//!   head, cockpit or bridge is destroyed is incapacitated the pulse the frame says so, and is
//!   told then.
//!
//! The condition reads the control sections (`crate::field_ops`'s anatomy: `head`, `cockpit`,
//! `bridge`): one destroyed incapacitates, one damaged injures, else fit. A pilot at a remote
//! console is fit. Doc 42 amends Q-CF-38's "no ejection, no capture": the location has five
//! variants (below), and ejection and capture write into it.
//!
//! **Custody** (doc 42 cut CR.0; Q-42-1, Q-42-2, Q-42-3): a character may be **held**
//! ([`CrewLocation::Captured`]: aboard a carrier that is not theirs, by a captor, since an IC
//! instant) or **in processing** at a port after a handover ([`CrewLocation::Processing`]). The
//! record is the authority: `check_in` leaves both alone, so a captor's carrier checking in from
//! a later field neither re-crews nor frees a captive, and [`crew_location`] prefers them over a
//! carrier row's `aboard`. A captive is a passenger who cannot leave and cannot operate: the
//! [`gate`] at the head of each verb Q-42-2 names refuses with one line ([`HELD`]), the
//! [`HeldGate`] hook refuses their exits, and mush-sim's `@station` and `+focus` ask the same
//! question through the carrier hooks (`crate::composition::sim_routes`). An ejectee waiting on a
//! forced extraction ([`Extraction::forced`]) is refused movement and operation too ([`PICKUP`]);
//! one in processing is refused only leaving the port ([`PROCESSING`]). What moves a character
//! into or out of custody is the later cuts' (CR.1 the load, CR.2 the handover, CR.3 the bound,
//! CR.4 the eject); this cut is the record, the gate and the lines.
//!
//! **Extraction** (Q-CF-39, P-5): a stranded or incapacitated character asks for it
//! (`+sortie/extract`); there is no timeout that extracts an unwilling pilot guarding the wreck.
//! After `[tacsim.field].extract_s` of IC time the character is at the **safe port they know**
//! — the port the IC-return stamp says they last stood in, else Stickney Yards on Phobos, the
//! guaranteed fallback doc 27 names — at its arrival room (`tick`). The hull, its cargo and its
//! epoch stay where they are; the seat is vacated. The delay is IC time, so a W-clock freeze
//! pauses it (no tick runs a frozen world). The completion is **deferred** while a Friend's
//! recovery flight — an open DT flight on the site of an activation on `recovery` duty under the
//! same organizer as the stranded character's own party — stands on the site, at most
//! `[tacsim.field].rescue_bound_s` past the due instant (the published bound).
//!
//! **Capture at a load** (doc 42 cut CR.1; Q-38-22, Q-42-4, Q-42-5, Q-42-16, Q-42-21, Q-42-26):
//! a casualty loaded into a carrier that is not its crew's own party's or a Friend's — a Neutral
//! salvor's as much as a Hostile's — takes everyone still aboard it captive
//! (`captured_loaded`): each leaves the seat, steps out into the carrier's bay room, and is
//! held there by the carrier's operator, the captor. A captor who was no enemy of the captive
//! pays [`STANDING_CAPTURE`] with each faction the captive openly stands in. A hired NPC keeps no
//! crew record and is never captured: their seat is vacated and they go back to their hire port's
//! pool room at once, and the roster's half — the contract and the reservation ended — is the
//! `tacsim_crew_captured` action the tow tick submits as the organizer until it lands
//! (`npc_exit_loaded`, `npc_exits`). GC.5's abandonment, which this replaces, left the
//! [`Extraction::abandoned`] flag on old records; it still reads.
//!
//! **Treatment** (doc 36 cut GC.13; Q-36-45): an injured or incapacitated character standing in
//! a room of an IC port (`crate::ic_return::port_of_room`) is under treatment there, and is fit
//! again after `[tacsim.field].treat_injured_s` or `treat_incapacitated_s` of IC time — scaled
//! by `treat_medical_permille` at a port with a medical room (`crate::port::medical`). No skill
//! check and no cost. The host's tick drives it as it drives extraction: the record carries the
//! due instant ([`Treatment`]), and a frozen world runs no tick, so a freeze pauses it. **The
//! rule for leaving:** a character the tick finds outside every port room — aboard a hull, out
//! on a site, in Limbo or the OOC area — loses the treatment and keeps their condition; the clock
//! starts over, in full, at the next port they stand in. A treatment begun at one port and found
//! at another starts over too. `+sortie/condition` is the crew status line, with the time left.

use std::collections::BTreeMap;

use mush_core::{CanonicalId, CmdError, Ctx, ObjId, ObjKind, World, WorldEvent, name_span};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::store::push_op;

/// [`CrewRecord::format`].
pub const CREW_FORMAT: u16 = 1;
/// The record's key prefix: `tacsim:crew:<character>`.
pub const CREW_PREFIX: &str = "tacsim:crew:";
/// The guaranteed fallback port (doc 27 Q-F4: "such as Phobos"): Stickney Yards.
pub const FALLBACK_PORT: &str = "stickney";
/// The sections whose loss takes the crew's control (`crate::field_ops`'s anatomy).
pub const CONTROL_SECTIONS: [&str; 3] = ["head", "cockpit", "bridge"];

/// `+sortie/extract` with nothing to extract from.
pub const NOT_STRANDED: &str = "You are not stranded: extraction is for a character left in a hull that cannot bring them home.";
/// Treatment begun (doc 36 cut GC.13): `{port}`, `{room}`, `{h}`.
pub const TREATMENT_BEGUN: &str = "You are under treatment at {port}{room}: fit again in about {h} IC h, if you stay at the port.";
/// Treatment done (doc 36 cut GC.13).
pub const FIT_AGAIN: &str = "Your treatment is done: you are fit again.";
/// Treatment broken off by leaving the port (doc 36 cut GC.13).
pub const TREATMENT_LEFT: &str =
    "You have left the port before your treatment was done; it starts over at the next port.";
/// [`TREATMENT_BEGUN`]'s `{room}` at a port with a medical room.
pub const MEDICAL_ROOM: &str = "'s medical room";
/// An extraction already on its way: `{port}`, `{h}`.
pub const ALREADY_COMING: &str =
    "Your extraction to {port} is already on its way, about {h} IC h out.";
/// The request taken: `{port}`, `{h}`.
pub const EXTRACT_TAKEN: &str = "Extraction requested. A recovery service will lift you to {port} in about {h} IC h; your hull, its cargo and its log stay where they are.";
/// The arrival: `{port}`, `{site}`.
pub const EXTRACTED: &str =
    "The recovery service sets you down at {port}. Your hull is still on {site}.";
/// Stranded at a check-in: `{unit}`, `{site}`.
pub const STRANDED: &str = "#{unit} cannot bring you home: you are stranded on {site}. +sortie/extract asks a recovery service to lift you out.";
/// Incapacitated on a control-section kill: `{section}`.
pub const INCAPACITATED: &str = "Your {section} is gone and you with it, for now: you are incapacitated. +sortie/extract asks a recovery service to lift you out.";
/// **The held refusal** (doc 42 Q-42-2, §7a): a captive's movement, operation, extraction
/// request or detachment chat. `{carrier}`.
pub const HELD: &str = "You are held aboard the {carrier}.";
/// **The pickup refusal** (doc 42 Q-42-12, Draft 0.3 42-5 (B)): an ejectee's movement or
/// operation while their forced extraction is on its way.
pub const PICKUP: &str = "You are waiting for your pickup.";
/// **The processing refusal** (doc 42 Q-42-7, Draft 0.3 42-2): leaving the port during the
/// processing hour. `{port}`, `{h}`.
pub const PROCESSING: &str = "You are being processed at {port}; about {h} IC h.";

/// `tacsim:crew:<character>`.
#[must_use]
pub fn crew_key(who: ObjId) -> SmolStr {
    SmolStr::new(format!("{CREW_PREFIX}{}", who.0))
}

/// **Where a character is**, as the field engine knows it (Q-CF-38).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "at", rename_all = "snake_case", deny_unknown_fields)]
pub enum CrewLocation {
    /// Inside a unit that can still bring them home.
    Unit {
        /// The unit.
        unit: ObjId,
    },
    /// Aboard a carrier (a remote console, or a hull back in the bay).
    Carrier {
        /// The carrier.
        hull: ObjId,
    },
    /// Inside a unit on a site that cannot bring them home.
    Stranded {
        /// The site.
        site: CanonicalId,
        /// The unit.
        unit: ObjId,
    },
    /// At a port, by extraction.
    Port {
        /// The port's venue id.
        port: SmolStr,
    },
    /// **Held** (doc 42 Q-42-1): aboard `carrier` — the hull they physically stand in, never the
    /// casualty they were taken in — in `captor`'s custody since `since_ic`, the bound's origin
    /// (Q-42-8). A new variant under the same tag, so [`CREW_FORMAT`] is unchanged.
    Captured {
        /// The carrier they ride in.
        carrier: ObjId,
        /// The captor: a character (Q-42-4).
        captor: ObjId,
        /// The IC instant of the capture.
        since_ic: i64,
    },
    /// **In processing** at a port after a handover (doc 42 Q-42-7, Draft 0.3 42-2): released
    /// at `release_due_ic`, the release fee's obligation `charge` once it is created (`None`
    /// first, so a pass that finds `None` creates it and one that finds it filled does not).
    Processing {
        /// The port's venue id.
        port: SmolStr,
        /// The IC instant of the release.
        release_due_ic: i64,
        /// The release fee's obligation, once created.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        charge: Option<mush_economy::ObligationId>,
    },
}

/// **How a character is** (Q-CF-38). Ordered by severity, fit first: a check-in keeps the worse
/// of what the record held and what the hull says (doc 36 cut GC.13, "the condition kept"; doc
/// 36a §11 row 4), and only a treatment at a port makes a character fit again (`treat`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrewCondition {
    /// Unhurt.
    Fit,
    /// Hurt: a control section damaged around them.
    Injured,
    /// Out of action: a control section destroyed around them.
    Incapacitated,
}

/// **An extraction in flight** (Q-CF-39).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Extraction {
    /// The port's venue id.
    pub port: SmolStr,
    /// The IC instant it was asked for.
    pub requested_ic: i64,
    /// The IC instant it lands.
    pub due_ic: i64,
    /// **Doc 36 cut GC.5** (Q-36-52): started by a non-friend's load, so no Friend's recovery
    /// flight defers it. Omitted when false, so an asked-for extraction reads as before.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub abandoned: bool,
    /// **Doc 42 Q-42-10** (cut CR.4 writes it): started by an ejection, not asked for. The body
    /// stays in the cockpit room until the landing moves it, so every load rule skips the record
    /// and the [`gate`] refuses movement and operation ([`PICKUP`]). Omitted when false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub forced: bool,
    /// **Doc 42 Q-42-12** (Draft 0.3 42-5 (B)): the eject's draw, kept so the landing survives a
    /// restart: when set, the landing captures into that carrier if it still stands on the site.
    /// Omitted when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture: Option<EjectCapture>,
}

/// **An ejection's capture, drawn at the eject** (doc 42 Q-42-12, Draft 0.3 42-5 (B)): the party
/// that held the cell, the carrier the captive will ride in, and the captor (its leader, Q-42-4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EjectCapture {
    /// The party that held the cell.
    pub party: crate::arena::party::PartyId,
    /// The party's carrier on the site.
    pub carrier: ObjId,
    /// The party's leader.
    pub captor: ObjId,
}

/// **A treatment under way** (doc 36 cut GC.13; Q-36-45).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Treatment {
    /// The port's venue id.
    pub port: SmolStr,
    /// The IC instant it began.
    pub started_ic: i64,
    /// The IC instant the character is fit again.
    pub due_ic: i64,
    /// Whether the port had a medical room when it began.
    pub medical: bool,
}

/// **One character's crew record** (`tacsim:crew:<character>`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrewRecord {
    /// [`CREW_FORMAT`].
    pub format: u16,
    /// The character.
    pub character: ObjId,
    /// Where they are.
    pub location: CrewLocation,
    /// How they are.
    pub condition: CrewCondition,
    /// The IC instant it was last set.
    pub at_ic: i64,
    /// The organizer of the party they were in (a Friend's recovery flight is one of theirs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organizer: Option<ObjId>,
    /// An extraction asked for and not yet landed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extraction: Option<Extraction>,
    /// A treatment under way at a port (doc 36 cut GC.13).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub treatment: Option<Treatment>,
}

impl CrewRecord {
    /// Whether the character may ask for extraction (Q-CF-39): stranded, or incapacitated in a
    /// unit.
    #[must_use]
    pub fn extractable(&self) -> Option<ObjId> {
        match &self.location {
            CrewLocation::Stranded { unit, .. } => Some(*unit),
            CrewLocation::Unit { unit } if self.condition == CrewCondition::Incapacitated => {
                Some(*unit)
            }
            // Doc 42: a captive asks only once the bound has passed (cut CR.3), and a character in
            // processing is released by the port, not lifted out.
            CrewLocation::Unit { .. }
            | CrewLocation::Carrier { .. }
            | CrewLocation::Port { .. }
            | CrewLocation::Captured { .. }
            | CrewLocation::Processing { .. } => None,
        }
    }

    /// **Whether the record is in custody** (doc 42 Q-42-3): held, or in processing after a
    /// handover. `check_in` and [`crew_location`] defer to such a record.
    #[must_use]
    pub const fn in_custody(&self) -> bool {
        matches!(
            self.location,
            CrewLocation::Captured { .. } | CrewLocation::Processing { .. }
        )
    }

    /// **Whether an ejection's forced extraction is still on its way** (doc 42 Q-42-12).
    #[must_use]
    pub fn awaiting_pickup(&self) -> bool {
        self.extraction.as_ref().is_some_and(|coming| coming.forced)
    }

    /// **What holds the character, if anything** (doc 42 Q-42-2): the [`gate`]'s one read.
    #[must_use]
    pub fn hold(&self) -> Option<Hold> {
        match &self.location {
            CrewLocation::Captured { carrier, .. } => Some(Hold::Captured { carrier: *carrier }),
            CrewLocation::Processing {
                port,
                release_due_ic,
                ..
            } => Some(Hold::Processing {
                port: port.clone(),
                release_due_ic: *release_due_ic,
            }),
            _ if self.awaiting_pickup() => Some(Hold::Pickup),
            _ => None,
        }
    }
}

/// **What holds a character** (doc 42 Q-42-2; Draft 0.3 42-2 and 42-5): the three cases the
/// [`gate`] refuses on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hold {
    /// Held aboard `carrier` ([`CrewLocation::Captured`]): every gated verb refuses ([`HELD`]).
    Captured {
        /// The carrier.
        carrier: ObjId,
    },
    /// An ejectee waiting on a forced extraction: movement and operation refuse ([`PICKUP`]).
    Pickup,
    /// In processing at `port` until `release_due_ic`: only leaving the port refuses
    /// ([`PROCESSING`]).
    Processing {
        /// The port's venue id.
        port: SmolStr,
        /// The IC instant of the release.
        release_due_ic: i64,
    },
}

/// **What a gated verb does** (doc 42 Q-42-2's list, Draft 0.3 42-6): the [`gate`]'s argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    /// Moves the character: an exit or `go`, `+tv/bay in|out`, `+ship/board|leave`,
    /// `+tv/<n> board|leave`, `+sortie/seat`.
    Move,
    /// Operates a unit or commits one to an operation: `+tv/<n>` acts, `+ship/*`, `+helm/*`,
    /// `+tac/*`, `+field/*`, `+bay/*`, `+salvage/*`, `+sortie/activate|hire|board|contract|
    /// launch|recover|cancel-launch`, a cockpit keyword, `WILCO`/`NEGATIVE`, `@station/*`,
    /// `+focus`.
    Operate,
    /// `+sortie/extract`: refused to a captive until the bound (cut CR.3).
    Extract,
    /// `+sortie/chat`: the roster radio a captor would take.
    Chat,
}

/// **Whether `who` is held aboard a carrier** (doc 42 Q-42-2's `crew::held`): the carrier.
#[must_use]
pub fn held(w: &World, who: ObjId) -> Option<ObjId> {
    match crew_of(w, who)?.hold()? {
        Hold::Captured { carrier } => Some(carrier),
        Hold::Pickup | Hold::Processing { .. } => None,
    }
}

/// **The refusal `gate` gives `who`**, if any (doc 42 Q-42-2): [`HELD`] for a captive on every
/// gate; [`PICKUP`] for an ejectee on movement and operation; [`PROCESSING`] for a character in
/// processing on movement. `None` passes.
#[must_use]
pub fn refusal(w: &World, who: ObjId, gate: Gate) -> Option<String> {
    let record = crew_of(w, who)?;
    // Doc 42 cut CR.3 (Q-42-8): a captive may ask for extraction once the bound has passed; before
    // it, the refusal names the time left.
    if gate == Gate::Extract
        && let CrewLocation::Captured {
            carrier, since_ic, ..
        } = record.location
    {
        let due = since_ic.saturating_add(i64::from(super::config(w).captive_bound_s));
        let left = due - w.world_ic_now().at;
        return (left > 0).then(|| {
            HELD_BOUND
                .replace("{carrier}", &crate::port::coop::name_of(w, carrier))
                .replace("{h}", &hours(left).to_string())
        });
    }
    let hold = record.hold()?;
    refusal_for(w, &hold, gate)
}

/// **A captive's extraction before the bound** (doc 42 cut CR.3, Q-42-8): `{carrier}`, `{h}`.
pub const HELD_BOUND: &str = "You are held aboard the {carrier}. In about {h} IC h a recovery service may negotiate your release: +sortie/extract then.";
/// **An extraction asked for from captivity, or as a passenger** (doc 42 cut CR.3; Q-42-8,
/// Q-42-9): `{port}`, `{h}`.
pub const EXTRACT_RELEASE: &str = "Extraction requested. A recovery service negotiates your passage and sets you down at {port} in about {h} IC h.";

/// **Whether `who` may ask for extraction where their record stands** (doc 42 cut CR.3): a
/// stranded or incapacitated character inside their unit (Q-CF-39); a captive past the bound
/// (the gate has read it, Q-42-8); or a passenger released aboard a hull not their own (Q-42-9's
/// widening: `Carrier { hull }` on a hull other than the one they command).
fn may_extract(w: &World, who: ObjId, record: &CrewRecord) -> bool {
    match &record.location {
        CrewLocation::Captured { .. } => true,
        CrewLocation::Carrier { hull } => {
            crate::port::coop::helm_of(w, who).is_none_or(|helm| helm.hull != *hull)
                && inside(w, who, *hull)
        }
        _ => record
            .extractable()
            .is_some_and(|unit| inside(w, who, unit)),
    }
}

/// [`refusal`]'s rule for one [`Hold`].
fn refusal_for(w: &World, hold: &Hold, gate: Gate) -> Option<String> {
    match (hold, gate) {
        (Hold::Captured { carrier }, _) => {
            Some(HELD.replace("{carrier}", &crate::port::coop::name_of(w, *carrier)))
        }
        (Hold::Pickup, Gate::Move | Gate::Operate) => Some(PICKUP.to_owned()),
        (
            Hold::Processing {
                port,
                release_due_ic,
            },
            Gate::Move,
        ) => Some(processing_line(w, port, *release_due_ic)),
        (Hold::Pickup | Hold::Processing { .. }, _) => None,
    }
}

/// [`PROCESSING`], filled.
fn processing_line(w: &World, port: &str, release_due_ic: i64) -> String {
    PROCESSING.replace("{port}", port_name(port)).replace(
        "{h}",
        &hours(release_due_ic - w.world_ic_now().at).to_string(),
    )
}

/// **The held gate** (doc 42 Q-42-2): read at the head of each verb the question names, before
/// anything else the verb reads or writes.
///
/// # Errors
///
/// The one-line refusal ([`refusal`]).
pub fn gate(ctx: &Ctx<'_>, gate: Gate) -> Result<(), CmdError> {
    refusal(ctx.w, ctx.actor, gate).map_or(Ok(()), |line| Err(CmdError::msg(line)))
}

/// **The held gate on exits** (doc 42 Q-42-2: "`go` and exits"): a `MoveAttempt` veto. A captive
/// and an ejectee waiting on their pickup go nowhere; a character in processing goes anywhere in
/// the port, and nowhere else. The crew's own moves (an extraction's landing, a capture, a
/// handover) move the body directly and never ask.
pub struct HeldGate;

impl mush_core::EventHook for HeldGate {
    fn move_attempt(
        &self,
        ctx: &mush_core::HookReadContext<'_>,
        actor: ObjId,
        _exit: Option<ObjId>,
        dest: ObjId,
    ) -> Result<(), mush_proto::RichText> {
        let w = ctx.world();
        let Some(hold) = crew_of(w, actor).and_then(CrewRecord::hold) else {
            return Ok(());
        };
        if let Hold::Processing { port, .. } = &hold
            && crate::ic_return::port_of_room(w, dest).is_some_and(|here| here.id == port.as_str())
        {
            return Ok(());
        }
        refusal_for(w, &hold, Gate::Move)
            .map_or(Ok(()), |line| Err(mush_proto::RichText::tag("error", line)))
    }
}

/// **Every crew record, in RAM** (module state; loaded at boot, written through `put`).
#[derive(Clone, Debug, Default)]
pub struct CrewBook(BTreeMap<ObjId, CrewRecord>);

/// **A character's crew record** — the reader CF.7 calls.
#[must_use]
pub fn crew_of(w: &World, who: ObjId) -> Option<&CrewRecord> {
    w.module_state::<CrewBook>()?.0.get(&who)
}

/// **Where a character is** (Q-CF-38): aboard a carrier while a running field lists them on the
/// carrier's row (doc 32 cut CF.3's rows, `FieldUnit::aboard`), else their crew record's
/// location; `None` for a character the field engine never touched. A record in custody (doc 42
/// Q-42-3: held, or in processing) wins over the carrier's row.
#[must_use]
pub fn crew_location(w: &World, who: ObjId) -> Option<CrewLocation> {
    if let Some(record) = crew_of(w, who).filter(|record| record.in_custody()) {
        return Some(record.location.clone());
    }
    let aboard = w.module_state::<super::FieldBook>().and_then(|book| {
        book.running().find_map(|(_, hosted)| {
            hosted
                .live
                .units
                .iter()
                .find(|row| row.aboard.contains(&who))
                .map(|row| CrewLocation::Carrier { hull: row.unit })
        })
    });
    aboard.or_else(|| crew_of(w, who).map(|record| record.location.clone()))
}

/// **Read the boot's crew records** (`on_world_loaded`), and the NPC exits still owed (doc 42
/// cut CR.1).
pub(crate) fn read(ctx: &mut Ctx<'_>, kv: &[(SmolStr, serde_json::Value)]) {
    if let Some(exits) = ctx.w.module_state_mut::<NpcExits>() {
        for (key, value) in kv {
            let Some(npc) = key
                .as_str()
                .strip_prefix(NPC_EXIT_PREFIX)
                .and_then(|n| n.parse::<u32>().ok())
                .map(ObjId)
            else {
                continue;
            };
            match serde_json::from_value::<NpcExit>(value.clone()) {
                Ok(exit) if exit.npc == npc && exit.format == NPC_EXIT_FORMAT => {
                    exits.owed.insert(npc, exit);
                }
                _ => tracing::warn!(
                    npc = npc.0,
                    "tacsim: an owed NPC exit did not decode; it is ignored"
                ),
            }
        }
    }
    let Some(book) = ctx.w.module_state_mut::<CrewBook>() else {
        return;
    };
    for (key, value) in kv {
        let Some(who) = key
            .as_str()
            .strip_prefix(CREW_PREFIX)
            .and_then(|n| n.parse::<u32>().ok())
            .map(ObjId)
        else {
            continue;
        };
        match serde_json::from_value::<CrewRecord>(value.clone()) {
            Ok(record) if record.character == who && record.format == CREW_FORMAT => {
                book.0.insert(who, record);
            }
            _ => tracing::warn!(
                who = who.0,
                "tacsim: a crew record did not decode; it is ignored"
            ),
        }
    }
}

/// **Write a crew record** to RAM and the module's lane.
pub(crate) fn put(ctx: &mut Ctx<'_>, record: CrewRecord) {
    match serde_json::to_value(&record) {
        Ok(value) => push_op(
            ctx.out,
            mush_core::ModuleWrite::JsonSet {
                key: crew_key(record.character),
                value,
            },
        ),
        Err(error) => {
            tracing::error!(%error, "tacsim: a crew record did not encode");
            return;
        }
    }
    if let Some(book) = ctx.w.module_state_mut::<CrewBook>() {
        book.0.insert(record.character, record);
    }
}

/// **Whether `who` is physically inside `obj`**: the location chain, or the spatial graph.
#[must_use]
pub(crate) fn inside(w: &World, who: ObjId, obj: ObjId) -> bool {
    let mut here = w.get(who).and_then(|object| object.location);
    for _ in 0..16 {
        match here {
            Some(at) if at == obj => return true,
            Some(at) => here = w.get(at).and_then(|object| object.location),
            None => break,
        }
    }
    crate::detachment::planning::projection::physically_aboard(w, who, obj)
}

/// **The crew's condition from a unit's sections** (this module's head): by section id, its
/// condition word and its internals.
fn condition_of<'a>(
    sections: impl Iterator<Item = (&'a str, &'a str, Option<(i64, i64)>)>,
) -> CrewCondition {
    let mut worst = CrewCondition::Fit;
    for (id, condition, internals) in sections {
        if !CONTROL_SECTIONS.contains(&id) {
            continue;
        }
        let gone = matches!(condition, "destroyed" | "wreck" | "irrecoverable")
            || internals.is_some_and(|(now, _)| now <= 0);
        let hurt = internals.is_some_and(|(now, max)| now < max) || condition == "damaged";
        if gone {
            return CrewCondition::Incapacitated;
        }
        if hurt {
            worst = CrewCondition::Injured;
        }
    }
    worst
}

/// **The crew's condition in `unit`** as its checked-in record stands ([`condition_of`] over its
/// sections and their internals).
fn unit_condition(w: &World, unit: ObjId) -> CrewCondition {
    mush_tactics::module::unit_of(w, unit).map_or(CrewCondition::Fit, cells_condition)
}

/// **The crew's condition in a unit whose cells are `cells`** ([`condition_of`] over its
/// sections and their internals): the checked-in record's, or the arena's live row (doc 42 cut
/// CR.4's eject, Q-42-13).
#[must_use]
pub(crate) fn cells_condition(cells: &mush_tactics::TacticalUnit) -> CrewCondition {
    let internals_cell = mush_tactics::CellId::new(crate::definitions::unit::INTERNALS_CELL).ok();
    condition_of(cells.sections().map(|(id, section)| {
        let internals = internals_cell
            .as_ref()
            .and_then(|cell| {
                cells.cell(&mush_tactics::CellAddr::Section {
                    section: id.clone(),
                    cell: cell.clone(),
                })
            })
            .map(|cell| {
                (
                    cell.current.saturating_sub(cell.min),
                    cell.max.saturating_sub(cell.min),
                )
            });
        (id.as_str(), section.condition.as_str(), internals)
    }))
}

/// **Whether a unit whose cells are `cells` is doomed** (doc 42 cut CR.4; Q-42-11): it could not
/// bring its pilot home at a check-in — no power, no control, no motion, a wreck or irrecoverable
/// (`home`'s negation) — or a control section is already damaged, so the next hit
/// incapacitates. The arena reads it on its live row (`crate::arena::eject`), adding its own
/// wreck, motion-disabled and disabled limits.
#[must_use]
pub(crate) fn doomed(cells: &mush_tactics::TacticalUnit) -> bool {
    // A unit with no mobility profile is never doomed for want of motion (the withdraw's own
    // power test, `crate::arena::withdraw`).
    let unable = crate::field_ops::assess_tacsim_unit(cells, None)
        .ok()
        .is_none_or(|c| {
            !home(&crate::field_ops::FieldUnitAssessment {
                can_move: c.can_move || cells.mobility().is_none(),
                ..c
            })
        });
    unable || cells_condition(cells) != CrewCondition::Fit
}

/// Whether an assessed unit can bring its pilot home: power, control and motion, and neither a
/// wreck nor irrecoverable (`check_in`'s rule).
fn home(c: &crate::field_ops::FieldUnitAssessment) -> bool {
    c.has_power
        && c.has_control
        && c.can_move
        && !matches!(
            c.state,
            crate::field_ops::OperationalState::Wreck
                | crate::field_ops::OperationalState::Irrecoverable
        )
}

/// **Who is seated in `unit`**, if anyone (a reader for the gates).
#[must_use]
pub fn seated_in(w: &World, unit: ObjId) -> Option<ObjId> {
    super::parties::seated(w, unit).map(|(who, _)| who)
}

/// The hull whose rooms `who` stands in, if any.
fn carrier_of(w: &World, who: ObjId) -> Option<ObjId> {
    let room = w.get(who)?.location?;
    crate::port::ship::hull_of_room(w, room).map(|(view, _)| view.hull())
}

/// **Where a soul aboard `hull` stands** after a field on `site`: aboard the carrier, or, when
/// its hull fell, stranded on the site in the wreck (doc 36 cut GC.3; Q-36-8's table).
fn aboard(w: &World, site: &CanonicalId, hull: ObjId) -> CrewLocation {
    let wrecked = crate::carrier_damage::damage_of(w, hull)
        .is_some_and(|damage| damage.has(crate::carrier_damage::CarrierLoss::CrewStranded));
    if wrecked {
        CrewLocation::Stranded {
            site: site.clone(),
            unit: hull,
        }
    } else {
        CrewLocation::Carrier { hull }
    }
}

/// The organizer of the party `unit` was in, while its field runs.
pub(crate) fn organizer_of(w: &World, site: &CanonicalId, unit: ObjId) -> Option<ObjId> {
    let hosted = w.module_state::<super::FieldBook>()?.hosted(site)?;
    let party = hosted.live.party_of(unit)?;
    hosted
        .live
        .parties
        .iter()
        .find(|row| row.party == party)
        .and_then(|row| match row.key {
            super::PartyKey::Operation { operation } => Some(operation.organizer),
            super::PartyKey::Unit { .. } => row.leader,
            // A fixture raider's hulls stand behind its NPC leader (doc 32 cut CF.6).
            super::PartyKey::Fixture { leader } => Some(leader),
        })
}

/// **Derive `pilot`'s crew state as `unit` checks in from a field on `site`** (this module's
/// head). The unit's cells are the checked-in record's.
///
/// **What a check-in never does** (doc 36a §11 row 4): it never heals. The condition written is
/// the worse of the record's and the hull's (doc 36 cut GC.13: the condition is kept until a
/// port treats it; a sound carrier's console, or a replacement hull, is no treatment). And a
/// soul abandoned in a casualty loaded into a captor's bay (doc 36 cut GC.5, Q-36-52: an
/// extraction marked `abandoned`) is not the captor's crew: the captor's carrier checking in
/// from a later field leaves their record alone, so the extraction lands them at a port as the
/// load promised. Doc 42 widens that guard ([`leaves_alone`], Q-42-3): a record in custody, and
/// an ejectee's forced extraction, are the record's to end, never a check-in's.
pub(crate) fn check_in(ctx: &mut Ctx<'_>, site: &CanonicalId, unit: ObjId, pilot: Option<ObjId>) {
    let Some(pilot) = pilot else {
        return;
    };
    if ctx
        .w
        .get(pilot)
        .is_none_or(|object| object.kind != ObjKind::Player)
    {
        return;
    }
    let prior = crew_of(ctx.w, pilot).cloned();
    // Doc 42 cut CR.3 (Q-42-15): a captor's carrier whose hull fell strands everyone aboard,
    // captives included; custody ends with the hull.
    if let Some(stranded) = prior
        .as_ref()
        .and_then(|record| wrecked_captive(record, site, unit, aboard(ctx.w, site, unit)))
    {
        put(ctx, stranded);
        let line = STRANDED
            .replace("{unit}", &unit.0.to_string())
            .replace("{site}", &super::admit::site_name(ctx, site));
        ctx.out.sys_to_actor(pilot, "tacsim.field", line);
        return;
    }
    if prior.as_ref().is_some_and(leaves_alone) {
        return;
    }
    let now = ctx.w.world_ic_now().at;
    let organizer = organizer_of(ctx.w, site, unit)
        .or_else(|| crew_of(ctx.w, pilot).and_then(|record| record.organizer));
    let (location, condition) = if super::carrier::is_carrier(ctx.w, unit) {
        // A carrier (doc 32 cut CF.3) is a hull its crew stand aboard, never a cockpit.
        if !inside(ctx.w, pilot, unit) {
            return;
        }
        (aboard(ctx.w, site, unit), unit_condition(ctx.w, unit))
    } else if inside(ctx.w, pilot, unit) {
        let Some(cells) = mush_tactics::module::unit_of(ctx.w, unit) else {
            return;
        };
        let assessment = crate::field_ops::assess_tacsim_unit(cells, None).ok();
        let condition = unit_condition(ctx.w, unit);
        let location = if assessment.as_ref().is_some_and(home) {
            CrewLocation::Unit { unit }
        } else {
            CrewLocation::Stranded {
                site: site.clone(),
                unit,
            }
        };
        (location, condition)
    } else if let Some(hull) = carrier_of(ctx.w, pilot) {
        (aboard(ctx.w, site, hull), CrewCondition::Fit)
    } else {
        return;
    };
    let stranded = matches!(location, CrewLocation::Stranded { .. });
    let (condition, extraction) = match prior {
        Some(record) => (record.condition.max(condition), record.extraction),
        None => (condition, None),
    };
    put(
        ctx,
        CrewRecord {
            format: CREW_FORMAT,
            character: pilot,
            location,
            condition,
            at_ic: now,
            organizer,
            extraction: if stranded { extraction } else { None },
            treatment: None,
        },
    );
    if stranded {
        let line = STRANDED
            .replace("{unit}", &unit.0.to_string())
            .replace("{site}", &super::admit::site_name(ctx, site));
        ctx.out.sys_to_actor(pilot, "tacsim.field", line);
    }
}

/// **`check_in`'s rule for a captive whose carrier checks in** (doc 42 cut CR.3, Q-42-15):
/// held aboard `unit` and the carrier's own check-in strands its souls (`at` is
/// [`aboard`]'s reading) — stranded in the wreck on `site`, the condition kept, custody gone.
/// Anyone else, and a captive of a sound carrier, is `None`.
#[must_use]
pub(crate) fn wrecked_captive(
    record: &CrewRecord,
    site: &CanonicalId,
    unit: ObjId,
    at: CrewLocation,
) -> Option<CrewRecord> {
    let CrewLocation::Captured { carrier, .. } = record.location else {
        return None;
    };
    if carrier != unit || !matches!(at, CrewLocation::Stranded { .. }) {
        return None;
    }
    let mut next = record.clone();
    next.location = CrewLocation::Stranded {
        site: site.clone(),
        unit,
    };
    next.extraction = None;
    Some(next)
}

/// **Whether a check-in leaves `record` alone** (doc 36a §11 row 4; doc 42 Q-42-3, Q-42-12): an
/// abandoned extraction (GC.5), an ejectee's forced one, or a character in custody — held, or in
/// processing. The prior condition is kept either way: a check-in never heals.
#[must_use]
pub(crate) fn leaves_alone(record: &CrewRecord) -> bool {
    record.in_custody()
        || record
            .extraction
            .as_ref()
            .is_some_and(|coming| coming.abandoned || coming.forced)
}

/// **Everyone a carrier's row listed aboard** (doc 32 cut CF.3's rows), as the carrier checks in:
/// each soul still aboard is recorded aboard it (`check_in`'s carrier branch).
pub(crate) fn check_in_aboard(
    ctx: &mut Ctx<'_>,
    site: &CanonicalId,
    carrier: ObjId,
    aboard: &[ObjId],
) {
    for who in aboard {
        check_in(ctx, site, carrier, Some(*who));
    }
}

/// **A casualty loaded into a friendly carrier's bay** (doc 36 cut GC.4; Q-36-52's friendly
/// half): every character whose record holds them in `casualty` — aboard it, or stranded in it —
/// is aboard `carrier` now, and any extraction they had asked for is cancelled. The caller
/// (`crate::surface::tow`) has checked that the carrier is of the casualty's own party or a
/// Friend's; a load into anyone else's bay is a capture ([`captured_loaded`]). `room`, when given,
/// is the carrier's bay room each soul steps out into first: a written-off lease rig goes to
/// Occator's shed, not the bay, and its crew stay with the carrier (doc 42 Draft 0.3, 42-8).
pub(crate) fn aboard_loaded(
    ctx: &mut Ctx<'_>,
    casualty: ObjId,
    carrier: ObjId,
    room: Option<ObjId>,
) {
    let now = ctx.w.world_ic_now().at;
    let souls: Vec<CrewRecord> = ctx
        .w
        .module_state::<CrewBook>()
        .map(|book| {
            book.0
                .values()
                .filter_map(|record| loaded_aboard(record, casualty, carrier, now))
                .collect()
        })
        .unwrap_or_default();
    let line = crate::surface::tow::CREW_ABOARD
        .replace("{name}", &crate::port::coop::name_of(ctx.w, casualty))
        .replace("{carrier}", &crate::port::coop::name_of(ctx.w, carrier));
    for record in souls {
        let who = record.character;
        if let Some(room) = room {
            vacate_seat(ctx, who, casualty);
            step_into(ctx, who, room);
        }
        put(ctx, record);
        ctx.out.sys_to_actor(who, "tacsim.field", line.clone());
    }
}

/// **[`aboard_loaded`]'s rule for one record**: a character aboard or stranded in `casualty` is
/// aboard `carrier` at `now`, with no extraction pending; anyone else is untouched (`None`).
#[must_use]
pub(crate) fn loaded_aboard(
    record: &CrewRecord,
    casualty: ObjId,
    carrier: ObjId,
    now: i64,
) -> Option<CrewRecord> {
    if !held_in(record, casualty) {
        return None;
    }
    let mut next = record.clone();
    next.location = CrewLocation::Carrier { hull: carrier };
    next.extraction = None;
    next.at_ic = now;
    Some(next)
}

/// **Whether a load of `casualty` carries `record`'s character** (GC.4, GC.5): aboard or stranded
/// in it. Doc 42 Draft 0.3 42-5 (B): an ejectee's forced extraction is skipped — the draw is
/// already decided, so the ejectee rides where the hull goes and lands where the extraction says
/// — and a record in custody names a carrier, never a casualty.
#[must_use]
pub(crate) fn held_in(record: &CrewRecord, casualty: ObjId) -> bool {
    if record.awaiting_pickup() {
        return false;
    }
    match &record.location {
        CrewLocation::Unit { unit } | CrewLocation::Stranded { unit, .. } => *unit == casualty,
        CrewLocation::Carrier { .. }
        | CrewLocation::Port { .. }
        | CrewLocation::Captured { .. }
        | CrewLocation::Processing { .. } => false,
    }
}

/// **The crew captured at a load** (doc 42 cut CR.1, Q-42-16): `{name}`, `{carrier}`,
/// `{captor}`.
pub const CAPTURED: &str = "The {name} is hauled into the {carrier}'s bay, a carrier not of your party or a Friend's, with you still in it: you are held aboard the {carrier}, in {captor}'s custody.";
/// **The captor's line at a load that took crew** (doc 42 cut CR.1): `{carrier}`, `{who}`.
pub const CAPTOR_TOLD: &str = "Aboard the {carrier} you now hold {who}. +ship/captives lists them; +ship/release <who> lets one go.";
/// **What a capture costs a captor who was no enemy of the captive** (doc 42 Q-42-21,
/// provisional): five points of the -100..100 scale with each faction the captive openly stands
/// in. Not voidable.
pub const STANDING_CAPTURE: i16 = -5;
/// **The capture receipts' cause prefix** (doc 42 Q-42-21): `capture:<site tag>:<ordinal>:<captive>`.
pub const CAPTURE_CAUSE: &str = "capture:";
/// **The standing a capture moved**, told to the captor: `{who}`, `{factions}`, `{delta}`.
pub const CAPTURE_STANDING: &str = "Carrying off {who}, who was no enemy of yours, moves your standing with {factions} by {delta}.";
/// The reason every capture receipt records: `{who}`, `{site}`.
const CAPTURE_REASON: &str = "Carried off {who}, who was no enemy, from {site}.";

/// **One completed non-friend load, as the capture reads it** (doc 42 cut CR.1): the site and
/// its last field's ordinal (the hostility read, Q-42-21), the casualty, the carrier, the
/// carrier's bay room the captives step out into, and the captor (Q-42-4: the carrier's
/// operator, the hull's custodian).
pub(crate) struct LoadCapture<'a> {
    /// The site.
    pub site: &'a CanonicalId,
    /// The site's last field ordinal (0: none).
    pub ordinal: u32,
    /// The casualty loaded.
    pub casualty: ObjId,
    /// The carrier it was loaded into.
    pub carrier: ObjId,
    /// The carrier's bay room.
    pub room: ObjId,
    /// The captor.
    pub captor: ObjId,
}

/// **The crew captured in a casualty loaded into a non-friend's bay** (doc 42 cut CR.1; Q-38-22,
/// Q-42-1, Q-42-16): every character whose record holds them in the casualty — aboard it, or
/// stranded in it — leaves its seat, steps out into the carrier's bay room, and is held aboard
/// the carrier by the captor from now. The body moves first and the record after, so the
/// [`HeldGate`] never sees the move. Each is told; the captor is told once; and a captor who was
/// no enemy of a captive pays the standing (Q-42-21). The captives, in record order. The caller
/// (`crate::surface::tow`) has checked that the carrier is not of their party or a Friend's.
pub(crate) fn captured_loaded(ctx: &mut Ctx<'_>, load: &LoadCapture<'_>) -> Vec<ObjId> {
    let now = ctx.w.world_ic_now().at;
    let held: Vec<CrewRecord> = ctx
        .w
        .module_state::<CrewBook>()
        .map(|book| {
            book.0
                .values()
                .filter(|record| held_in(record, load.casualty))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let name = crate::port::coop::name_of(ctx.w, load.casualty);
    let carrier = crate::port::coop::name_of(ctx.w, load.carrier);
    let captor = crate::port::coop::name_of(ctx.w, load.captor);
    let mut taken = Vec::new();
    for record in held {
        let who = record.character;
        vacate_seat(ctx, who, load.casualty);
        step_into(ctx, who, load.room);
        put(ctx, capture(&record, load.carrier, load.captor, now));
        ctx.out.sys_to_actor(
            who,
            "tacsim.field",
            CAPTURED
                .replace("{name}", &name)
                .replace("{carrier}", &carrier)
                .replace("{captor}", &captor),
        );
        capture_standing(ctx, load, who);
        taken.push(who);
    }
    if !taken.is_empty() {
        let names: Vec<String> = taken
            .iter()
            .map(|who| crate::port::coop::name_of(ctx.w, *who))
            .collect();
        ctx.out.sys_to_actor(
            load.captor,
            "tacsim.field",
            CAPTOR_TOLD
                .replace("{carrier}", &carrier)
                .replace("{who}", &names.join(", ")),
        );
    }
    taken
}

/// **[`captured_loaded`]'s rule for one record**: held aboard `carrier` by `captor` since `now`,
/// the condition kept, and any extraction or treatment gone (a captive asks for extraction only
/// after the bound, cut CR.3).
#[must_use]
pub(crate) fn capture(record: &CrewRecord, carrier: ObjId, captor: ObjId, now: i64) -> CrewRecord {
    let mut next = record.clone();
    next.location = CrewLocation::Captured {
        carrier,
        captor,
        since_ic: now,
    };
    next.extraction = None;
    next.treatment = None;
    next.at_ic = now;
    next
}

/// **Vacate `who`'s seat in `unit`**, when they hold it (a load runs only outside a field, so no
/// arena is told).
fn vacate_seat(ctx: &mut Ctx<'_>, who: ObjId, unit: ObjId) {
    let Some((seated, station)) = super::parties::seated(ctx.w, unit) else {
        return;
    };
    if seated != who {
        return;
    }
    if let Some(station) = mush_sim::StationId::new(station.as_str())
        && let Err(error) = mush_sim::vacate_station(ctx, unit, &station, who)
    {
        tracing::warn!(
            who = who.0,
            unit = unit.0,
            ?error,
            "tacsim: a loaded casualty's seat did not vacate"
        );
    }
}

/// **`who` steps out into `room`** — a carrier's bay room — by mush-sim's interior placement
/// (the `+tv/<n> board` path), else a plain move. Never through the exit hooks: the crew's own
/// moves never ask the [`HeldGate`].
fn step_into(ctx: &mut Ctx<'_>, who: ObjId, room: ObjId) {
    let from = ctx.w.get(who).and_then(|object| object.location);
    if from == Some(room) {
        return;
    }
    if let Err(error) = mush_sim::place_interior(ctx, who, room, None) {
        tracing::warn!(who = who.0, room = room.0, %error, "tacsim: a loaded crew member's placement refused; moved instead");
        if ctx.w.move_to(who, room).is_err() {
            return;
        }
    }
    ctx.out.pin_deferred();
    ctx.emit_world_event(&WorldEvent::Moved {
        who,
        from,
        to: room,
    });
    ctx.show_room(who, room);
}

/// **The cause of the standing a capture writes** (doc 42 Q-42-21):
/// `capture:<site tag>:<ordinal>:<captive>`, the site tag GC.10's.
#[must_use]
pub fn capture_cause(site: &CanonicalId, ordinal: u32, captive: ObjId) -> String {
    let field = super::standing::cause_prefix(site, ordinal);
    let tail = field.strip_prefix("field:").unwrap_or(&field);
    format!("{CAPTURE_CAUSE}{tail}{}", captive.0)
}

/// **Whether `captor` held `captive` Hostile at the capture** (doc 42 Q-42-21): an organization
/// of either declared Hostile on one of the other's (doc 40 cut FR.1's stance table, read now),
/// or either stood behind fire in the site's last field (a GC.10 receipt of that field naming
/// them). A field's private marks end with it (Q-CF-8) and leave no record, so a mark that was
/// never followed by fire reads as no hostility.
#[must_use]
pub(crate) fn held_hostile(
    w: &World,
    site: &CanonicalId,
    ordinal: u32,
    captor: ObjId,
    captive: ObjId,
) -> bool {
    if declared_hostile(w, captor, captive) {
        return true;
    }
    if ordinal == 0 {
        return false;
    }
    mush_faction::standing_causes(w, &super::standing::cause_prefix(site, ordinal))
        .iter()
        .filter_map(|cause| mush_faction::standing_receipt(w, cause))
        .any(|receipt| receipt.subject == captor || receipt.subject == captive)
}

/// Whether an organization of `a` and one of `b` stand Hostile, either way (the stance table).
fn declared_hostile(w: &World, a: ObjId, b: ObjId) -> bool {
    use super::parties::Affiliation;
    let Some(table) = w
        .module_state::<mush_faction::FactionState>()
        .filter(|table| !table.stances.is_empty())
    else {
        return false;
    };
    let side = |affiliation: &Affiliation| match affiliation {
        Affiliation::Org(org) => mush_faction::StanceSide::Org(*org),
        Affiliation::Faction(faction) => mush_faction::StanceSide::Faction(*faction),
    };
    let (mine, theirs) = (
        super::parties::affiliations(w, a),
        super::parties::affiliations(w, b),
    );
    mine.iter().any(|x| {
        theirs.iter().any(|y| {
            table.stance(side(x), side(y)) == mush_faction::Stance::Hostile
                || table.stance(side(y), side(x)) == mush_faction::Stance::Hostile
        })
    })
}

/// **The capture's standing** (doc 42 Q-42-21): unless the captor held the captive Hostile, one
/// grant against the captor with each faction the captive openly stands in, [`STANDING_CAPTURE`]
/// each, not voidable. An identical retry writes nothing.
fn capture_standing(ctx: &mut Ctx<'_>, load: &LoadCapture<'_>, captive: ObjId) {
    if held_hostile(ctx.w, load.site, load.ordinal, load.captor, captive)
        || ctx.w.get(load.captor).is_none_or(|object| {
            !object.kind.is_actor() || object.flags.contains(mush_core::Flags::GOING)
        })
    {
        return;
    }
    let effects: Vec<mush_faction::StandingEffect> = mush_faction::open_factions(ctx.w, captive)
        .into_iter()
        .take(8)
        .map(|faction| mush_faction::StandingEffect {
            faction,
            delta: STANDING_CAPTURE,
        })
        .collect();
    if effects.is_empty() {
        return;
    }
    let who = crate::port::coop::name_of(ctx.w, captive);
    let reason = CAPTURE_REASON
        .replace("{who}", &who)
        .replace("{site}", &super::admit::site_name(ctx, load.site));
    let cause = capture_cause(load.site, load.ordinal, captive);
    match mush_faction::record_standing(ctx, &cause, load.captor, &effects, &reason) {
        Ok(true) => {
            let factions = ctx
                .w
                .module_state::<mush_faction::FactionState>()
                .map(|state| {
                    effects
                        .iter()
                        .map(|effect| {
                            state.factions.get(&effect.faction).map_or_else(
                                || format!("faction {}", effect.faction.0),
                                |faction| faction.name.to_string(),
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            ctx.out.sys_to_actor(
                load.captor,
                "tacsim.field",
                CAPTURE_STANDING
                    .replace("{who}", &who)
                    .replace("{factions}", &factions)
                    .replace("{delta}", &STANDING_CAPTURE.to_string()),
            );
        }
        Ok(false) => {}
        Err(error) => {
            tracing::warn!(cause, %error, "tacsim: a capture's standing was refused");
        }
    }
}

// ---- the NPC exit (doc 42 Q-42-5, Q-42-26) ------------------------------------------------

/// [`NpcExit::format`].
pub const NPC_EXIT_FORMAT: u16 = 1;
/// The owed NPC exits' key prefix: `tacsim:crew_exit:<npc>`.
pub const NPC_EXIT_PREFIX: &str = "tacsim:crew_exit:";
/// **A hired NPC's exit at a load**, told to the organizer (doc 42 Q-42-5): `{npc}`, `{name}`,
/// `{carrier}`, `{room}`.
pub const NPC_RETURNED: &str = "{npc} was flying the {name} when it was hauled into the {carrier}'s bay, a carrier not of your party or a Friend's: their contract with you ends, and they are back at {room}.";

/// `tacsim:crew_exit:<npc>`.
#[must_use]
pub fn npc_exit_key(npc: ObjId) -> SmolStr {
    SmolStr::new(format!("{NPC_EXIT_PREFIX}{}", npc.0))
}

/// **A hired NPC's exit owed to the roster** (doc 42 Q-42-26): the load did the world half — the
/// seat vacated, the NPC in their hire port's pool room, the organizer told — and the tow tick
/// submits `tacsim_crew_captured` as the organizer until the ledger shows the contract and the
/// reservation gone. Until then the contract stands, so the NPC is not on offer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NpcExit {
    /// [`NPC_EXIT_FORMAT`].
    pub format: u16,
    /// The NPC.
    pub npc: ObjId,
    /// The organizer: the contract's employer, the action's principal.
    pub organizer: ObjId,
    /// The IC instant of the load: the action's key.
    pub recovered_ic: i64,
}

/// **The owed NPC exits, in RAM** (module state; loaded at boot, written through
/// `owe_npc_exit`), and how many times each was submitted this boot.
#[derive(Clone, Debug, Default)]
pub struct NpcExits {
    owed: BTreeMap<ObjId, NpcExit>,
    tries: BTreeMap<ObjId, u32>,
}

/// **The exit owed for `npc`**, if any.
#[must_use]
pub fn npc_exit_of(w: &World, npc: ObjId) -> Option<&NpcExit> {
    w.module_state::<NpcExits>()?.owed.get(&npc)
}

/// **The exits owed to `organizer`'s roster**, in NPC order (the action's offer).
#[must_use]
pub fn npc_exits_for(w: &World, organizer: ObjId) -> Vec<NpcExit> {
    w.module_state::<NpcExits>()
        .map(|book| {
            book.owed
                .values()
                .filter(|exit| exit.organizer == organizer)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// Write `exit` to RAM and the module's lane.
fn owe_npc_exit(ctx: &mut Ctx<'_>, exit: NpcExit) {
    match serde_json::to_value(&exit) {
        Ok(value) => push_op(
            ctx.out,
            mush_core::ModuleWrite::JsonSet {
                key: npc_exit_key(exit.npc),
                value,
            },
        ),
        Err(error) => {
            tracing::error!(%error, "tacsim: an owed NPC exit did not encode");
            return;
        }
    }
    if let Some(book) = ctx.w.module_state_mut::<NpcExits>() {
        book.tries.remove(&exit.npc);
        book.owed.insert(exit.npc, exit);
    }
}

/// **The NPC pool room for `npc`** (doc 42 Q-42-5): the room the NPC was authored in, their home,
/// else the arrival room of the hire port the contract names.
fn pool_room(w: &World, npc: ObjId, port: &str) -> Option<ObjId> {
    w.get(npc)
        .and_then(|object| object.home)
        .filter(|home| {
            w.get(*home).is_some_and(|room| {
                room.kind == ObjKind::Room && !room.flags.contains(mush_core::Flags::GOING)
            })
        })
        .or_else(|| {
            crate::ic_return::PORTS
                .iter()
                .find(|known| known.id == port)
                .and_then(|known| crate::ic_return::arrival_room(w, known))
        })
}

/// **A hired NPC flying a casualty loaded into a non-friend's bay** (doc 42 cut CR.1; Q-42-5,
/// Q-42-26): the world half of the exit, at once. The NPC seated in `casualty` under a crew
/// contract leaves the seat and goes back to the hire port's pool room; the organizer is told;
/// the roster's half is owed ([`NpcExit`]) and the tow tick submits it ([`npc_exits`]). An NPC
/// with no contract, or nobody seated, is left alone. The NPC.
pub(crate) fn npc_exit_loaded(ctx: &mut Ctx<'_>, casualty: ObjId, carrier: ObjId) -> Option<ObjId> {
    let (npc, station) = super::parties::seated(ctx.w, casualty)?;
    if ctx
        .w
        .get(npc)
        .is_none_or(|object| object.kind != ObjKind::Npc)
    {
        return None;
    }
    let contracts = ctx
        .w
        .module_state::<crate::detachment::crew::CrewHandles>()?
        .contracts
        .clone();
    let contract = ctx.action_state(&contracts, &npc).ok().flatten()?.clone();
    let room = pool_room(ctx.w, npc, contract.terms.port.as_str())?;
    if let Some(station) = mush_sim::StationId::new(station.as_str())
        && let Err(error) = mush_sim::vacate_station(ctx, casualty, &station, npc)
    {
        tracing::warn!(
            npc = npc.0,
            unit = casualty.0,
            ?error,
            "tacsim: a captured hull's NPC seat did not vacate"
        );
    }
    let from = ctx.w.get(npc).and_then(|object| object.location);
    if from != Some(room) {
        if ctx.w.move_to(npc, room).is_err() {
            tracing::warn!(
                npc = npc.0,
                "tacsim: a captured hull's NPC could not go back to the pool"
            );
            return None;
        }
        let placed = ctx
            .w
            .module_state::<mush_sim::SpatialState>()
            .is_some_and(|state| state.is_placed(npc));
        if placed && let Err(error) = mush_sim::unplace(ctx, npc, "crew exit") {
            tracing::warn!(npc = npc.0, %error, "tacsim: a returned NPC's placement did not clear");
        }
        ctx.emit_world_event(&WorldEvent::Moved {
            who: npc,
            from,
            to: room,
        });
    }
    owe_npc_exit(
        ctx,
        NpcExit {
            format: NPC_EXIT_FORMAT,
            npc,
            organizer: contract.employer,
            recovered_ic: ctx.w.world_ic_now().at,
        },
    );
    let line = NPC_RETURNED
        .replace("{npc}", &crate::port::coop::name_of(ctx.w, npc))
        .replace("{name}", &crate::port::coop::name_of(ctx.w, casualty))
        .replace("{carrier}", &crate::port::coop::name_of(ctx.w, carrier))
        .replace("{room}", &crate::port::coop::name_of(ctx.w, room));
    ctx.out
        .sys_to_actor(contract.employer, "tacsim.field", line);
    Some(npc)
}

/// **The owed NPC exits' pass** (doc 42 Q-42-26; on the tow tick): an exit whose contract and
/// reservation the ledger no longer shows for its organizer has landed and is forgotten; every
/// other is submitted again as its organizer (`Fire::as_principal`).
pub(crate) fn npc_exits(ctx: &mut Ctx<'_>) {
    let owed: Vec<NpcExit> = ctx
        .w
        .module_state::<NpcExits>()
        .map(|book| book.owed.values().cloned().collect())
        .unwrap_or_default();
    if owed.is_empty() {
        return;
    }
    let Some(contracts) = ctx
        .w
        .module_state::<crate::detachment::crew::CrewHandles>()
        .map(|handles| handles.contracts.clone())
    else {
        return;
    };
    let reservations = ctx
        .w
        .module_state::<crate::detachment::activation::ActivationHandles>()
        .map(|handles| handles.reservations.clone());
    let mut fire = crate::port_tick::Fire::new(BTreeMap::new());
    for exit in owed {
        let Ok(contract) = ctx.action_state(&contracts, &exit.npc) else {
            continue;
        };
        let contracted = contract.is_some_and(|c| c.employer == exit.organizer);
        let reserved = match &reservations {
            Some(handle) => match ctx.action_state(handle, &exit.npc) {
                Ok(found) => found.is_some_and(|r| r.organizer == exit.organizer),
                Err(_) => continue,
            },
            None => false,
        };
        if !contracted && !reserved {
            push_op(
                ctx.out,
                mush_core::ModuleWrite::Del {
                    key: npc_exit_key(exit.npc),
                },
            );
            if let Some(book) = ctx.w.module_state_mut::<NpcExits>() {
                book.owed.remove(&exit.npc);
                book.tries.remove(&exit.npc);
            }
            tracing::info!(
                npc = exit.npc.0,
                "tacsim: a captured hull's NPC is off the roster"
            );
            continue;
        }
        let tries = ctx
            .w
            .module_state::<NpcExits>()
            .and_then(|book| book.tries.get(&exit.npc).copied())
            .unwrap_or(0);
        if crate::detachment::crew::submit_captured(ctx, &mut fire, &exit, tries)
            && let Some(book) = ctx.w.module_state_mut::<NpcExits>()
        {
            book.tries.insert(exit.npc, tries.saturating_add(1));
        }
    }
}

/// **A field's frames** (the host's branch, before the fire door): a pilot inside a hull whose
/// control section the frame shows destroyed is incapacitated now, and told once.
pub(crate) fn on_frames(ctx: &mut Ctx<'_>, frames: &[(ObjId, crate::arena::step::Frame)]) {
    let now = ctx.w.world_ic_now().at;
    for (pilot, frame) in frames {
        let condition = condition_of(frame.sections.iter().map(|section| {
            (
                section.id.as_str(),
                section.condition.as_str(),
                section.internals,
            )
        }));
        // Doc 42 Q-42-3: a captive and an ejectee sit in no seat; no frame reaches them.
        if condition != CrewCondition::Incapacitated
            || crew_of(ctx.w, *pilot).is_some_and(|record| {
                (record.condition == CrewCondition::Incapacitated
                    && record.location == CrewLocation::Unit { unit: frame.unit })
                    || record.in_custody()
                    || record.awaiting_pickup()
            })
            || !inside(ctx.w, *pilot, frame.unit)
        {
            continue;
        }
        let section = frame
            .sections
            .iter()
            .find(|section| CONTROL_SECTIONS.contains(&section.id.as_str()))
            .map_or("cockpit", |section| section.id.as_str())
            .to_owned();
        let site = super::field_of_unit(ctx.w, frame.unit);
        let organizer = site
            .as_ref()
            .and_then(|site| organizer_of(ctx.w, site, frame.unit));
        put(
            ctx,
            CrewRecord {
                format: CREW_FORMAT,
                character: *pilot,
                location: CrewLocation::Unit { unit: frame.unit },
                condition: CrewCondition::Incapacitated,
                at_ic: now,
                organizer,
                extraction: None,
                treatment: None,
            },
        );
        ctx.out.sys_to_actor(
            *pilot,
            "tacsim.field",
            INCAPACITATED.replace("{section}", &section),
        );
    }
}

/// The safe port `who` knows (this module's head): the IC-return stamp's, else the fallback.
pub(crate) fn destination(w: &World, who: ObjId) -> Option<&'static crate::ic_return::IcPort> {
    crate::ic_return::last_port(w, who).or_else(|| {
        crate::ic_return::PORTS
            .iter()
            .find(|port| port.id == FALLBACK_PORT)
    })
}

/// IC hours, rounded up.
pub(crate) fn hours(seconds: i64) -> i64 {
    seconds.max(0).saturating_add(3_599) / 3_600
}

/// **`+sortie/extract`** (Q-CF-39): ask for extraction (this module's head).
///
/// # Errors
///
/// The refusal line.
pub(crate) fn extract_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    // Doc 42 Q-42-2: a captive asks only after the bound (cut CR.3 opens it).
    gate(ctx, Gate::Extract)?;
    let actor = ctx.actor;
    let record = crew_of(ctx.w, actor)
        .cloned()
        .ok_or_else(|| CmdError::msg(NOT_STRANDED))?;
    let port =
        destination(ctx.w, actor).ok_or_else(|| CmdError::msg(crate::ic_return::NO_CORRIDOR))?;
    let now = ctx.w.world_ic_now().at;
    if let Some(coming) = &record.extraction {
        let name = crate::ic_return::PORTS
            .iter()
            .find(|known| known.id == coming.port.as_str())
            .map_or(coming.port.as_str(), |known| known.name);
        return Err(CmdError::msg(
            ALREADY_COMING
                .replace("{port}", name)
                .replace("{h}", &hours(coming.due_ic - now).to_string()),
        ));
    }
    if !may_extract(ctx.w, actor, &record) {
        return Err(CmdError::msg(NOT_STRANDED));
    }
    let negotiated = matches!(
        record.location,
        CrewLocation::Captured { .. } | CrewLocation::Carrier { .. }
    );
    if ctx.w.world_timeline().is_frozen() {
        return Err(CmdError::msg(
            "World time is frozen; ask again once it resumes.",
        ));
    }
    let delay = i64::from(super::config(ctx.w).extract_s);
    let mut next = record;
    next.extraction = Some(Extraction {
        port: SmolStr::new(port.id),
        requested_ic: now,
        due_ic: now.saturating_add(delay),
        abandoned: false,
        forced: false,
        capture: None,
    });
    put(ctx, next);
    ctx.out.sys_to_actor(
        actor,
        "tacsim.field",
        (if negotiated {
            EXTRACT_RELEASE
        } else {
            EXTRACT_TAKEN
        })
        .replace("{port}", port.name)
        .replace("{h}", &hours(delay).to_string()),
    );
    Ok(())
}

/// **Whether a rescue stands on `site` for `who`** (doc 42 Q-42-14, cut CR.4; this module's
/// head): an open DT flight in the field there, on any duty, or a live rig lease whose rig stands
/// on the site's ground, of the character's own party or a Friend's (`friendly`).
fn rescue_on_site(ctx: &Ctx<'_>, site: &CanonicalId, record: &CrewRecord) -> bool {
    let (who, organizer) = (record.character, record.organizer);
    // The hull the character is stranded in rescues nobody.
    let own = match record.location {
        CrewLocation::Stranded { unit, .. } | CrewLocation::Unit { unit } => Some(unit),
        _ => None,
    };
    let friend = |other: ObjId| friendly(ctx.w, who, organizer, other);
    let rig = ctx
        .w
        .module_state::<crate::TacsimStore>()
        .is_some_and(|store| {
            store.tv_leases_live().any(|lease| {
                Some(lease.tv) != own
                    && friend(lease.pilot)
                    && match crate::surface::act::where_is(ctx.w, lease.tv) {
                        crate::surface::act::Where::Ground { domain, .. } => {
                            crate::surface::act::site_of_domain(ctx.w, domain).as_ref()
                                == Some(site)
                        }
                        crate::surface::act::Where::Bay { .. }
                        | crate::surface::act::Where::Elsewhere => false,
                    }
            })
        });
    rig || flight_on_site(ctx, site, &friend)
}

/// **Whether `other` is of `who`'s own party or a Friend's** (Q-42-14): `who` themself or their
/// organizer, or a character sharing an organization either openly stands in (doc 36 cut GC.10's
/// friendship, `super::parties::affiliations`).
fn friendly(w: &World, who: ObjId, organizer: Option<ObjId>, other: ObjId) -> bool {
    if other == who || Some(other) == organizer {
        return true;
    }
    let theirs = super::parties::affiliations(w, other);
    [Some(who), organizer]
        .into_iter()
        .flatten()
        .any(|mine| !super::parties::affiliations(w, mine).is_disjoint(&theirs))
}

/// Whether an open DT flight in the field on `site` has an organizer `friend` accepts.
fn flight_on_site(ctx: &Ctx<'_>, site: &CanonicalId, friend: &dyn Fn(ObjId) -> bool) -> bool {
    let Some(h) = ctx
        .w
        .module_state::<crate::detachment::sortie::SortieHandles>()
    else {
        return false;
    };
    let Some(index) = ctx.action_state(&h.index, &()).ok().flatten() else {
        return false;
    };
    index.units.iter().any(|unit| {
        let Some(head) = ctx.action_state(&h.heads, unit).ok().flatten() else {
            return false;
        };
        let Some(flight) = ctx.action_state(&h.flights, &head.flight).ok().flatten() else {
            return false;
        };
        if flight.phase != crate::detachment::sortie::Phase::Field || &flight.site != site {
            return false;
        }
        ctx.action_state(&h.operations, &flight.operation)
            .ok()
            .flatten()
            .is_some_and(|op| friend(op.roster.organizer))
    })
}

/// **Whether an ejectee's capture is still on its way to `carrier`** (doc 42 cut CR.4; Q-42-12,
/// Draft 0.3 42-5): a forced extraction whose draw named it. A raider's lift waits on it.
#[must_use]
pub(crate) fn capture_pending(w: &World, carrier: ObjId) -> bool {
    w.module_state::<CrewBook>().is_some_and(|book| {
        book.0.values().any(|record| {
            record.extraction.as_ref().is_some_and(|coming| {
                coming.forced
                    && coming
                        .capture
                        .as_ref()
                        .is_some_and(|capture| capture.carrier == carrier)
            })
        })
    })
}

/// **An eject's capture landing** (doc 42 cut CR.4; Q-42-12, Draft 0.3 42-5 (B)): when the
/// carrier the draw named still stands on `site` and has not fallen, the ejectee steps aboard
/// into its bay room and is held there by the captor, both told. `false` when it is gone, and
/// the extraction then lands at a port.
fn eject_captured(
    ctx: &mut Ctx<'_>,
    record: &CrewRecord,
    capture: &EjectCapture,
    site: &CanonicalId,
) -> bool {
    let carrier = capture.carrier;
    let standing = crate::port::bay_work::site_of_carrier(ctx.w, carrier).as_ref() == Some(site)
        && !super::eject::fallen(ctx.w, carrier);
    let room = crate::port::ship::hull_view(ctx.w, carrier).and_then(|view| {
        crate::port::vehicle::bay_rooms(ctx.w, &view)
            .first()
            .copied()
    });
    let (true, Some(room)) = (standing, room) else {
        return false;
    };
    let who = record.character;
    let now = ctx.w.world_ic_now().at;
    if let CrewLocation::Stranded { unit, .. } = record.location {
        release_seat(ctx, who, unit);
    }
    step_into(ctx, who, room);
    put(ctx, self::capture(record, carrier, capture.captor, now));
    let carrier_name = crate::port::coop::name_of(ctx.w, carrier);
    ctx.out.sys_to_actor(
        who,
        "tacsim.field",
        super::eject::EJECT_CAPTURED
            .replace("{carrier}", &carrier_name)
            .replace(
                "{captor}",
                &crate::port::coop::name_of(ctx.w, capture.captor),
            ),
    );
    ctx.out.sys_to_actor(
        capture.captor,
        "tacsim.field",
        super::eject::EJECT_CAPTOR_TOLD
            .replace("{who}", &crate::port::coop::name_of(ctx.w, who))
            .replace("{carrier}", &carrier_name),
    );
    tracing::info!(
        who = who.0,
        carrier = carrier.0,
        "tacsim: an ejected pilot was captured"
    );
    true
}

/// **The host's tick, for crew** (this module's head): every extraction due lands, unless a
/// Friend's recovery flight holds it within the bound.
pub(crate) fn tick(ctx: &mut Ctx<'_>) {
    let now = ctx.w.world_ic_now().at;
    let bound = i64::from(super::config(ctx.w).rescue_bound_s);
    let due: Vec<CrewRecord> = ctx
        .w
        .module_state::<CrewBook>()
        .map(|book| {
            book.0
                .values()
                .filter(|record| {
                    record
                        .extraction
                        .as_ref()
                        .is_some_and(|coming| coming.due_ic <= now)
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    for record in due {
        let Some(coming) = record.extraction.clone() else {
            continue;
        };
        let (site, unit) = match &record.location {
            CrewLocation::Stranded { site, unit } => (Some(site.clone()), Some(*unit)),
            CrewLocation::Unit { unit } => (super::field_of_unit(ctx.w, *unit), Some(*unit)),
            // Doc 42: no seat is left to vacate aboard a carrier, in custody or at a port. A
            // captive's extraction after the bound (CR.3) and an eject's captured landing (CR.4)
            // are their cuts'; nothing writes either yet.
            CrewLocation::Carrier { .. }
            | CrewLocation::Port { .. }
            | CrewLocation::Captured { .. }
            | CrewLocation::Processing { .. } => (None, None),
        };
        // Doc 42 cut CR.4 (Q-42-12): an eject's drawn capture lands aboard the holder's carrier
        // while it still stands on the site; gone or fallen, the pilot lands at a port.
        if let (Some(capture), Some(site)) = (&coming.capture, &site)
            && eject_captured(ctx, &record, capture, site)
        {
            continue;
        }
        // Q-42-14: a rescue of the character's own party or a Friend's on the site defers an
        // asked-for or an uncaptured forced extraction, never an abandoned or a captured one.
        if let Some(site) = &site
            && !coming.abandoned
            && coming.capture.is_none()
            && now < coming.due_ic.saturating_add(bound)
            && rescue_on_site(ctx, site, &record)
        {
            continue;
        }
        land(ctx, record, &coming, site.as_ref(), unit);
    }
    // Doc 42 cut CR.2: the handover's catch-up, the fee's retries and the releases due.
    custody_pass(ctx, now);
    treat(ctx, now);
}

/// **IC seconds of treatment** for `condition` (doc 36 cut GC.13): the configured time, scaled
/// at a port with a medical room; nothing for a fit character.
#[must_use]
pub fn treatment_s(
    cfg: &crate::config::FieldConfig,
    condition: CrewCondition,
    medical: bool,
) -> i64 {
    let full = match condition {
        CrewCondition::Fit => return 0,
        CrewCondition::Injured => i64::from(cfg.treat_injured_s),
        CrewCondition::Incapacitated => i64::from(cfg.treat_incapacitated_s),
    };
    if medical {
        full.saturating_mul(i64::from(cfg.treat_medical_permille)) / 1_000
    } else {
        full
    }
}

/// The IC port `who` stands in, if any.
fn port_here(w: &World, who: ObjId) -> Option<&'static crate::ic_return::IcPort> {
    let room = w.get(who)?.location?;
    crate::ic_return::port_of_room(w, room)
}

/// **The host's tick, for treatment** (this module's head): every injured or incapacitated
/// character at a port is under treatment there, fit again once it is due; one who left loses it.
fn treat(ctx: &mut Ctx<'_>, now: i64) {
    let cfg = super::config(ctx.w);
    let hurt: Vec<CrewRecord> = ctx
        .w
        .module_state::<CrewBook>()
        .map(|book| {
            book.0
                .values()
                .filter(|record| record.condition != CrewCondition::Fit)
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    for record in hurt {
        let who = record.character;
        match (port_here(ctx.w, who), &record.treatment) {
            (None, None) => {}
            (None, Some(_)) => {
                put(
                    ctx,
                    CrewRecord {
                        treatment: None,
                        ..record
                    },
                );
                ctx.out
                    .sys_to_actor(who, "tacsim.field", TREATMENT_LEFT.to_owned());
            }
            (Some(port), Some(under)) if under.port.as_str() == port.id => {
                if now < under.due_ic {
                    continue;
                }
                put(
                    ctx,
                    CrewRecord {
                        condition: CrewCondition::Fit,
                        treatment: None,
                        at_ic: now,
                        ..record
                    },
                );
                ctx.out
                    .sys_to_actor(who, "tacsim.field", FIT_AGAIN.to_owned());
                tracing::info!(
                    who = who.0,
                    port = port.id,
                    "tacsim: a character is fit again"
                );
            }
            (Some(port), _) => {
                let medical = crate::port::medical::has_medical_room(ctx.w, port);
                let span = treatment_s(&cfg, record.condition, medical);
                let line = TREATMENT_BEGUN
                    .replace("{port}", port.name)
                    .replace("{room}", if medical { MEDICAL_ROOM } else { "" })
                    .replace("{h}", &hours(span).to_string());
                put(
                    ctx,
                    CrewRecord {
                        treatment: Some(Treatment {
                            port: SmolStr::new(port.id),
                            started_ic: now,
                            due_ic: now.saturating_add(span),
                            medical,
                        }),
                        ..record
                    },
                );
                ctx.out.sys_to_actor(who, "tacsim.field", line);
            }
        }
    }
}

/// The name of the port `id`, else the id.
fn port_name(id: &str) -> &str {
    crate::ic_return::PORTS
        .iter()
        .find(|known| known.id == id)
        .map_or(id, |known| known.name)
}

/// **The crew status line** (doc 36 cut GC.13; `+sortie/condition`): where and how `who` is, and
/// what is under way — an extraction or a treatment, with the IC time left.
#[must_use]
pub fn status_line(ctx: &Ctx<'_>, who: ObjId) -> String {
    let Some(record) = crew_of(ctx.w, who) else {
        return "Crew: fit; the field has nothing on you.".to_owned();
    };
    let now = ctx.w.world_ic_now().at;
    let condition = match record.condition {
        CrewCondition::Fit => "fit",
        CrewCondition::Injured => "injured",
        CrewCondition::Incapacitated => "incapacitated",
    };
    let at = match crew_location(ctx.w, who).unwrap_or_else(|| record.location.clone()) {
        CrewLocation::Unit { unit } => format!("aboard #{}", unit.0),
        CrewLocation::Carrier { hull } => format!("aboard the carrier #{}", hull.0),
        CrewLocation::Stranded { site, unit } => format!(
            "stranded on {} in #{}",
            super::admit::site_name(ctx, &site),
            unit.0
        ),
        CrewLocation::Port { port } => format!("at {}", port_name(port.as_str())),
        // Doc 42 cut CR.0 (Q-42-1, Draft 0.3 42-2).
        CrewLocation::Captured { carrier, .. } => format!(
            "held aboard the {}",
            crate::port::coop::name_of(ctx.w, carrier)
        ),
        CrewLocation::Processing {
            port,
            release_due_ic,
            charge,
        } => format!(
            "in processing at {}, released in about {} IC h{}",
            port_name(port.as_str()),
            hours(release_due_ic - now),
            if charge.is_some() {
                ", the release fee charged"
            } else {
                ""
            }
        ),
    };
    let mut line = format!("Crew: {condition}, {at}");
    if let Some(coming) = &record.extraction {
        // Doc 42 Q-42-12: an ejectee's draw is kept on the record; the line names the pickup, not
        // the draw, which the landing tells.
        let what = if coming.forced {
            "pickup after your ejection, to"
        } else {
            "extraction to"
        };
        line.push_str(&format!(
            "; {what} {} in about {} IC h",
            port_name(coming.port.as_str()),
            hours(coming.due_ic - now)
        ));
    }
    if let Some(under) = &record.treatment {
        line.push_str(&format!(
            "; under treatment at {}{}, fit again in about {} IC h",
            port_name(under.port.as_str()),
            if under.medical { MEDICAL_ROOM } else { "" },
            hours(under.due_ic - now)
        ));
    } else if record.condition != CrewCondition::Fit {
        line.push_str("; treatment begins at the next port you stand in");
    }
    line.push('.');
    line
}

/// **Land one extraction**: the seat vacated, the character at the port's arrival room.
fn land(
    ctx: &mut Ctx<'_>,
    record: CrewRecord,
    coming: &Extraction,
    site: Option<&CanonicalId>,
    unit: Option<ObjId>,
) {
    let who = record.character;
    let Some(port) = crate::ic_return::PORTS
        .iter()
        .find(|known| known.id == coming.port.as_str())
        .or_else(|| destination(ctx.w, who))
    else {
        return;
    };
    let Some(dest) = crate::ic_return::arrival_room(ctx.w, port) else {
        tracing::warn!(
            who = who.0,
            port = port.id,
            "tacsim: an extraction's port has no arrival room; it waits"
        );
        return;
    };
    // The seat: out of the hull, and out of any field still running around it.
    if let Some(unit) = unit {
        release_seat(ctx, who, unit);
    }
    land_at(ctx, record, port, dest, site);
}

/// **Release `who`'s seat in `unit`**, when they hold it: out of the hull, and out of any field
/// still running around it (an extraction's landing, and doc 42 cut CR.4's eject).
pub(crate) fn release_seat(ctx: &mut Ctx<'_>, who: ObjId, unit: ObjId) {
    if let Some((seated, station)) = super::parties::seated(ctx.w, unit)
        && seated == who
    {
        // Doc 32 §15 (the review's CF-5a): the arena is told the seat is released, on every
        // channel, so it stops building frames for an absent pilot and the hull is the mind's;
        // `send_control(Released)` unbinds the pilot and clears the cockpit with it.
        if super::field_of_unit(ctx.w, unit).is_some() {
            crate::arena_bridge::control_all_channels(
                ctx,
                unit,
                crate::arena::intents::ControlState::Released,
                who,
            );
            // A control the registry could not route (no task holds the unit) unbinds nothing.
            let unbound = ctx
                .w
                .module_state_mut::<crate::arena_bridge::ArenaRegistry>()
                .is_some_and(|registry| registry.unbind_pilot(who));
            if unbound {
                crate::arena_bridge::clear_cockpit(ctx, who);
            }
        }
        if let Some(station) = mush_sim::StationId::new(station.as_str())
            && let Err(error) = mush_sim::vacate_station(ctx, unit, &station, who)
        {
            tracing::warn!(
                who = who.0,
                unit = unit.0,
                ?error,
                "tacsim: an extraction could not vacate the seat"
            );
        }
    }
}

/// **Land `record` at `port`'s arrival room `dest`** ([`land`]'s second half).
fn land_at(
    ctx: &mut Ctx<'_>,
    record: CrewRecord,
    port: &'static crate::ic_return::IcPort,
    dest: ObjId,
    site: Option<&CanonicalId>,
) {
    let who = record.character;
    let from = ctx.w.get(who).and_then(|object| object.location);
    if ctx.w.move_to(who, dest).is_err() {
        tracing::warn!(
            who = who.0,
            "tacsim: an extraction's move refused; it waits"
        );
        return;
    }
    let placed = ctx
        .w
        .module_state::<mush_sim::SpatialState>()
        .is_some_and(|state| state.is_placed(who));
    if placed && let Err(error) = mush_sim::unplace(ctx, who, "extraction") {
        tracing::warn!(who = who.0, %error, "tacsim: an extracted character's placement did not clear");
    }
    if let Some(left) = from {
        let text = mush_proto::RichText::new()
            .span(name_span(ctx.w, who))
            .text(" is lifted out.");
        ctx.out.to_room_from(
            left,
            vec![who],
            who,
            mush_proto::OutputEvent::Narrative {
                kind: mush_proto::NarrativeKind::Emit,
                actor: None,
                text,
            },
        );
    }
    ctx.show_room(who, dest);
    ctx.emit_world_event(&WorldEvent::Moved {
        who,
        from,
        to: dest,
    });
    if let Err(error) = crate::ic_return::stamp(ctx.w, who, port) {
        tracing::warn!(who = who.0, %error, "tacsim: the extraction's last-port stamp did not write");
    }
    let site_name = site.map_or_else(
        || "the ground".to_owned(),
        |site| super::admit::site_name(ctx, site),
    );
    let now = ctx.w.world_ic_now().at;
    put(
        ctx,
        CrewRecord {
            location: CrewLocation::Port {
                port: SmolStr::new(port.id),
            },
            extraction: None,
            at_ic: now,
            ..record
        },
    );
    ctx.out.sys_to_actor(
        who,
        "tacsim.field",
        EXTRACTED
            .replace("{port}", port.name)
            .replace("{site}", &site_name),
    );
    tracing::info!(
        who = who.0,
        port = port.id,
        "tacsim: a stranded character was extracted"
    );
}

// ---- the captor's verbs (doc 42 cut CR.1; Q-42-9) -----------------------------------------

/// `+ship/captives` with nobody held.
pub const NO_CAPTIVES: &str = "You hold nobody aboard.";
/// `+ship/captives`' row: `{name}`, `{n}`, `{carrier}`, `{condition}`, `{h}`.
pub const CAPTIVE_ROW: &str =
    "{name} (#{n}): held aboard the {carrier}, {condition}, for {h} IC h.";
/// `+ship/release` naming nobody held.
pub const NOT_HELD: &str = "You hold nobody aboard by that name. +ship/captives lists them.";
/// The release, told to the captor: `{who}`.
pub const RELEASED: &str = "You release {who}.";
/// The release docked at an IC port, told to the released: `{captor}`, `{port}`.
pub const RELEASED_AT_PORT: &str =
    "{captor} releases you at {port}: you are set down in its arrivals, free.";
/// The release anywhere else, told to the released: `{captor}`, `{carrier}`.
pub const RELEASED_ABOARD: &str = "{captor} releases you aboard the {carrier}: you are a passenger now, free to leave at the next port.";

/// **The captives `actor` holds**: held by them, or held aboard the hull they command.
fn captives_of(w: &World, actor: ObjId) -> Vec<CrewRecord> {
    let helm = crate::port::coop::helm_of(w, actor).map(|helm| helm.hull);
    w.module_state::<CrewBook>()
        .map(|book| {
            book.0
                .values()
                .filter(|record| match &record.location {
                    CrewLocation::Captured {
                        carrier, captor, ..
                    } => *captor == actor || Some(*carrier) == helm,
                    _ => false,
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// **`+ship/captives`** (doc 42 Q-42-9): the captives the actor holds, each with the carrier,
/// the condition and the IC hours held.
///
/// # Errors
///
/// None today; the signature is the verb's.
pub(crate) fn captives_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let held = captives_of(ctx.w, actor);
    if held.is_empty() {
        ctx.out
            .sys_to_actor(actor, "tacsim.field", NO_CAPTIVES.to_owned());
        return Ok(());
    }
    let now = ctx.w.world_ic_now().at;
    for record in held {
        let CrewLocation::Captured {
            carrier, since_ic, ..
        } = record.location
        else {
            continue;
        };
        let condition = match record.condition {
            CrewCondition::Fit => "fit",
            CrewCondition::Injured => "injured",
            CrewCondition::Incapacitated => "incapacitated",
        };
        let line = CAPTIVE_ROW
            .replace(
                "{name}",
                &crate::port::coop::name_of(ctx.w, record.character),
            )
            .replace("{n}", &record.character.0.to_string())
            .replace("{carrier}", &crate::port::coop::name_of(ctx.w, carrier))
            .replace("{condition}", condition)
            .replace("{h}", &((now - since_ic).max(0) / 3_600).to_string());
        ctx.out.sys_to_actor(actor, "tacsim.field", line);
    }
    Ok(())
}

/// **`+ship/release <who>`** (doc 42 Q-42-9): the captor lets one captive go, at will and for
/// nothing. With the carrier docked at an IC port the captive is set down in its arrival room, at
/// the port ([`CrewLocation::Port`]); anywhere else they stay where they stand, in the carrier's
/// bay room, a passenger aboard it ([`CrewLocation::Carrier`]). Their condition stays.
///
/// # Errors
///
/// [`NOT_HELD`] for anyone the actor does not hold.
pub(crate) fn release_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let raw = ctx.args.raw().trim().to_owned();
    let held = captives_of(ctx.w, actor);
    let wanted = raw.strip_prefix('#').and_then(|n| n.parse::<u32>().ok());
    let mut named = held.into_iter().filter(|record| {
        wanted.map_or_else(
            || {
                !raw.is_empty()
                    && crate::port::coop::name_of(ctx.w, record.character)
                        .to_ascii_lowercase()
                        .starts_with(&raw.to_ascii_lowercase())
            },
            |n| record.character.0 == n,
        )
    });
    let (Some(record), None) = (named.next(), named.next()) else {
        return Err(CmdError::msg(NOT_HELD));
    };
    let CrewLocation::Captured { carrier, .. } = record.location else {
        return Err(CmdError::msg(NOT_HELD));
    };
    let who = record.character;
    let now = ctx.w.world_ic_now().at;
    let port = match crate::port::ship::berth_of(ctx.w, carrier) {
        crate::port::ship::Berth::Docked { def, .. } => crate::ic_return::PORTS
            .iter()
            .find(|known| known.id == def.venue.as_str()),
        crate::port::ship::Berth::UnderWay | crate::port::ship::Berth::Elsewhere => None,
    };
    let landing =
        port.and_then(|port| crate::ic_return::arrival_room(ctx.w, port).map(|room| (port, room)));
    let captor = crate::port::coop::name_of(ctx.w, actor);
    let line = if let Some((port, room)) = landing {
        set_down(ctx, who, room);
        if let Err(error) = crate::ic_return::stamp(ctx.w, who, port) {
            tracing::warn!(who = who.0, %error, "tacsim: a released captive's last-port stamp did not write");
        }
        put(
            ctx,
            CrewRecord {
                location: CrewLocation::Port {
                    port: SmolStr::new(port.id),
                },
                at_ic: now,
                ..record
            },
        );
        RELEASED_AT_PORT
            .replace("{captor}", &captor)
            .replace("{port}", port.name)
    } else {
        put(
            ctx,
            CrewRecord {
                location: CrewLocation::Carrier { hull: carrier },
                at_ic: now,
                ..record
            },
        );
        RELEASED_ABOARD
            .replace("{captor}", &captor)
            .replace("{carrier}", &crate::port::coop::name_of(ctx.w, carrier))
    };
    ctx.out.sys_to_actor(who, "tacsim.field", line);
    let done = RELEASED.replace("{who}", &crate::port::coop::name_of(ctx.w, who));
    ctx.out.sys_to_actor(actor, "tacsim.field", done);
    Ok(())
}

/// **Set `who` down in `dest`** (a release at a port): out of any placement, a plain move, the
/// room told, and the arrival shown.
fn set_down(ctx: &mut Ctx<'_>, who: ObjId, dest: ObjId) {
    let from = ctx.w.get(who).and_then(|object| object.location);
    if ctx.w.move_to(who, dest).is_err() {
        tracing::warn!(who = who.0, "tacsim: a released captive's move refused");
        return;
    }
    let placed = ctx
        .w
        .module_state::<mush_sim::SpatialState>()
        .is_some_and(|state| state.is_placed(who));
    if placed && let Err(error) = mush_sim::unplace(ctx, who, "release") {
        tracing::warn!(who = who.0, %error, "tacsim: a released captive's placement did not clear");
    }
    ctx.show_room(who, dest);
    ctx.emit_world_event(&WorldEvent::Moved {
        who,
        from,
        to: dest,
    });
}

// ---- the port handover and release (doc 42 cut CR.2; Q-42-6, Q-42-7, Q-42-19) --------------

/// **The handover**, told to the captive: `{carrier}`, `{port}`, `{authority}`, `{h}`, `{fee}`.
pub const HANDED_OVER: &str = "The {carrier} docks at {port} and hands you over to the {authority}: you are in processing, released in about {h} IC h. The release fee is {fee} cr, levied on your account; whatever it cannot cover stays with you as a debt (+debts).";
/// **The handover**, told to the captor: `{who}`, `{authority}`, `{port}`.
pub const HANDED_OVER_CAPTOR: &str =
    "At {port} you hand {who} over to the {authority}. You are paid nothing for it.";
/// **The release after processing**, told to the released: `{authority}`, `{port}`.
pub const PROCESSED: &str = "The {authority} releases you at {port}: you are free.";

/// **The IC port a hull is docked at, and its authority's facts** (doc 42 Q-42-6): the port, the
/// entry's id and definition and map. `None` anywhere but a docked berth at an IC port whose entry
/// authors an authority.
fn authority_berth(
    w: &World,
    hull: ObjId,
) -> Option<(
    &'static crate::ic_return::IcPort,
    &crate::port::def::PortDef,
    &crate::port::map::PortMap,
)> {
    let crate::port::ship::Berth::Docked { map, def } = crate::port::ship::berth_of(w, hull) else {
        return None;
    };
    def.authority.as_ref()?;
    let port = crate::ic_return::PORTS
        .iter()
        .find(|known| known.id == def.venue.as_str())?;
    Some((port, def, map))
}

/// The authority's display name at `def`'s port.
fn authority_name(def: &crate::port::def::PortDef) -> String {
    def.authority.as_ref().map_or_else(String::new, |id| {
        def.orgs
            .iter()
            .find(|org| &org.id == id)
            .map_or_else(|| id.to_string(), |org| org.name.clone())
    })
}

/// **Hand every captive aboard `hull` over at the IC port it docked at** (doc 42 cut CR.2;
/// Q-42-6, Draft 0.3 42-2, 42-3): the docking hook's step, run before any hull hook of the same
/// arrival, and again on the crew tick, so a lost callback is caught. Each record still
/// `Captured { carrier: hull }` — and only such a record, so a duplicate callback, a replayed
/// arrival or a restart finds nobody twice — has its body set down in the port's medical room
/// (else its arrival room), and is written `Processing { charge: None }` first; the release fee's
/// obligation is created after and fills `charge` ([`levy_release`]). Nothing at a site's pad or
/// at a port with no authority.
pub(crate) fn hand_over(ctx: &mut Ctx<'_>, hull: ObjId) {
    let held: Vec<CrewRecord> = ctx
        .w
        .module_state::<CrewBook>()
        .map(|book| {
            book.0
                .values()
                .filter(|record| {
                    matches!(record.location, CrewLocation::Captured { carrier, .. } if carrier == hull)
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    if held.is_empty() {
        return;
    }
    let Some((port, room, authority, fee)) =
        authority_berth(ctx.w, hull).map(|(port, def, map)| {
            let room = crate::port::medical::room(ctx.w, &map.entry, def)
                .ok()
                .flatten()
                .or_else(|| crate::ic_return::arrival_room(ctx.w, port));
            (
                port,
                room,
                authority_name(def),
                def.release_fee_cr.unwrap_or(0),
            )
        })
    else {
        return;
    };
    let Some(room) = room else {
        tracing::warn!(
            hull = hull.0,
            port = port.id,
            "tacsim: a captive's handover port has no room to set them down in; it waits"
        );
        return;
    };
    let carrier = crate::port::coop::name_of(ctx.w, hull);
    let now = ctx.w.world_ic_now().at;
    let due = now.saturating_add(i64::from(super::config(ctx.w).release_s));
    for record in held {
        let who = record.character;
        let CrewLocation::Captured { captor, .. } = record.location else {
            continue;
        };
        set_down(ctx, who, room);
        put(
            ctx,
            CrewRecord {
                location: CrewLocation::Processing {
                    port: SmolStr::new(port.id),
                    release_due_ic: due,
                    charge: None,
                },
                // Doc 42 cut CR.3: an extraction asked for after the bound is overtaken by the
                // handover.
                extraction: None,
                at_ic: now,
                ..record
            },
        );
        ctx.out.sys_to_actor(
            who,
            "tacsim.field",
            HANDED_OVER
                .replace("{carrier}", &carrier)
                .replace("{port}", port.name)
                .replace("{authority}", &authority)
                .replace("{h}", &hours(due - now).to_string())
                .replace("{fee}", &fee.to_string()),
        );
        ctx.out.sys_to_actor(
            captor,
            "tacsim.field",
            HANDED_OVER_CAPTOR
                .replace("{who}", &crate::port::coop::name_of(ctx.w, who))
                .replace("{authority}", &authority)
                .replace("{port}", port.name),
        );
        levy_release(ctx, who);
    }
}

/// **The release fee's inspection key** (doc 42 Q-42-7): `release:<captive>:<release_due_ic>`,
/// one per handover.
#[must_use]
pub fn release_key(who: ObjId, release_due_ic: i64) -> String {
    format!("release:{}:{release_due_ic}", who.0)
}

/// **Levy the release fee for a record in processing with no charge yet** (doc 42 Q-42-7, Draft
/// 0.3 42-7): one `create_sanction_obligation`, one period due now, debiting the captive's own
/// account under the port clerk's `port.release_fee` grant on the authority's root scope, paying
/// the authority's account. An obligation already carrying the key (a crash between the economy's
/// write and the record's) is adopted rather than levied twice. A refusal leaves `charge` empty
/// for the crew tick to try again; the release never waits on it (FO-12).
fn levy_release(ctx: &mut Ctx<'_>, who: ObjId) {
    let Some(record) = crew_of(ctx.w, who).cloned() else {
        return;
    };
    let CrewLocation::Processing {
        port,
        release_due_ic,
        charge: None,
    } = &record.location
    else {
        return;
    };
    let key = release_key(who, *release_due_ic);
    let found = ctx
        .w
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|economy| {
            economy.snapshot().obligations().find_map(|o| {
                matches!(&o.cause, mush_economy::ObligationCause::Sanction { inspection } if *inspection == key)
                    .then_some(o.id)
            })
        });
    let charge = match found {
        Some(id) => Some(id),
        None => create_release_obligation(ctx, who, port.as_str(), &key),
    };
    let Some(charge) = charge else {
        return;
    };
    let mut next = record.clone();
    if let CrewLocation::Processing { charge: slot, .. } = &mut next.location {
        *slot = Some(charge);
    }
    put(ctx, next);
}

/// The economy's half of [`levy_release`]: the sanction's facts read off the port's entry and map.
fn create_release_obligation(
    ctx: &mut Ctx<'_>,
    who: ObjId,
    port: &str,
    key: &str,
) -> Option<mush_economy::ObligationId> {
    let (def, map) = crate::port::market::ports(ctx.w)
        .into_iter()
        .find(|(_, def, _)| def.venue.as_str() == port)
        .and_then(|(_, def, map)| Some((def.clone(), map?.clone())))?;
    let authority = def.authority.as_ref()?;
    let row = map.orgs.get(authority)?;
    let credit = mush_economy::AccountId(row.account?);
    let clerk = ObjId(map.clerk.as_ref()?.thing);
    let debit = crate::port::credit::own_account(ctx.w, who)?.id;
    let scale = crate::port::apply::credit_scale(ctx.w);
    let minor = crate::port::apply::minor_of(def.release_fee_cr?, scale)?;
    let capability = mush_org::CapabilityId::new(crate::port::vendor::RELEASE_FEE_CAPABILITY)?;
    let spec = mush_economy::SanctionSpec {
        debit_account: debit,
        credit_account: credit,
        amount: mush_economy::Amount::from_minor(minor),
        due_ic: ctx.w.world_ic_now().at,
        grace: mush_economy::GracePolicy::new(1),
        inspection: SmolStr::new(key),
        capability,
        scope: mush_org::OrgScopeId(row.root_scope),
        levied_by: clerk,
    };
    match mush_economy::create_sanction_obligation(ctx, spec) {
        Ok(id) => Some(id),
        Err(error) => {
            tracing::warn!(who = who.0, port, %error, "tacsim: a release fee could not be levied; the crew tick tries again");
            None
        }
    }
}

/// **The crew tick's custody pass** (doc 42 cut CR.2): every captive whose carrier now stands
/// docked at an IC port with an authority is handed over (the docking hook's catch-up); every
/// record in processing with no charge is levied again; every one whose release is due is
/// released ([`release_processed`]).
fn custody_pass(ctx: &mut Ctx<'_>, now: i64) {
    let (carriers, uncharged, due): (
        std::collections::BTreeSet<ObjId>,
        Vec<ObjId>,
        Vec<CrewRecord>,
    ) = {
        let Some(book) = ctx.w.module_state::<CrewBook>() else {
            return;
        };
        let mut carriers = std::collections::BTreeSet::new();
        let mut uncharged = Vec::new();
        let mut due = Vec::new();
        for record in book.0.values() {
            match &record.location {
                CrewLocation::Captured { carrier, .. } => {
                    carriers.insert(*carrier);
                }
                CrewLocation::Processing {
                    release_due_ic,
                    charge,
                    ..
                } => {
                    if *release_due_ic <= now {
                        due.push(record.clone());
                    } else if charge.is_none() {
                        uncharged.push(record.character);
                    }
                }
                _ => {}
            }
        }
        (carriers, uncharged, due)
    };
    for carrier in carriers {
        if authority_berth(ctx.w, carrier).is_some() {
            hand_over(ctx, carrier);
        }
    }
    for who in uncharged {
        levy_release(ctx, who);
    }
    for record in due {
        release_processed(ctx, record, now);
    }
}

/// **The release after processing** (doc 42 Q-42-7): the character at the port — set down in its
/// arrival room unless they already stand in one of its rooms — stamped as the port they know,
/// `Port { port }`, told. An uncharged fee is levied first, once more; the release never waits
/// on it.
fn release_processed(ctx: &mut Ctx<'_>, record: CrewRecord, now: i64) {
    let who = record.character;
    let CrewLocation::Processing { port, charge, .. } = &record.location else {
        return;
    };
    if charge.is_none() {
        levy_release(ctx, who);
    }
    let Some(known) = crate::ic_return::PORTS
        .iter()
        .find(|known| known.id == port.as_str())
    else {
        return;
    };
    if port_here(ctx.w, who).is_none_or(|here| here.id != known.id)
        && let Some(room) = crate::ic_return::arrival_room(ctx.w, known)
    {
        set_down(ctx, who, room);
    }
    if let Err(error) = crate::ic_return::stamp(ctx.w, who, known) {
        tracing::warn!(who = who.0, %error, "tacsim: a released captive's last-port stamp did not write");
    }
    let record = crew_of(ctx.w, who).cloned().unwrap_or(record);
    put(
        ctx,
        CrewRecord {
            location: CrewLocation::Port {
                port: SmolStr::new(known.id),
            },
            at_ic: now,
            ..record
        },
    );
    let authority = crate::port::market::ports(ctx.w)
        .into_iter()
        .find(|(_, def, _)| def.venue.as_str() == known.id)
        .map_or_else(String::new, |(_, def, _)| authority_name(def));
    ctx.out.sys_to_actor(
        who,
        "tacsim.field",
        PROCESSED
            .replace("{authority}", &authority)
            .replace("{port}", known.name),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_control_sections_decide_the_condition() {
        let fit = [
            ("hull", "destroyed", Some((0, 10))),
            ("cockpit", "intact", Some((5, 5))),
        ];
        assert_eq!(condition_of(fit.into_iter()), CrewCondition::Fit);
        let hurt = [("cockpit", "intact", Some((3, 5)))];
        assert_eq!(condition_of(hurt.into_iter()), CrewCondition::Injured);
        let gone = [("head", "destroyed", Some((0, 5)))];
        assert_eq!(condition_of(gone.into_iter()), CrewCondition::Incapacitated);
    }

    #[test]
    fn a_medical_room_treats_faster() {
        let cfg = crate::config::FieldConfig::default();
        assert_eq!(treatment_s(&cfg, CrewCondition::Fit, true), 0);
        assert_eq!(treatment_s(&cfg, CrewCondition::Injured, false), 86_400);
        assert_eq!(treatment_s(&cfg, CrewCondition::Injured, true), 43_200);
        assert_eq!(
            treatment_s(&cfg, CrewCondition::Incapacitated, false),
            259_200
        );
        assert_eq!(
            treatment_s(&cfg, CrewCondition::Incapacitated, true),
            129_600
        );
    }

    #[test]
    fn only_a_stranded_or_incapacitated_character_may_ask() {
        let mut record = CrewRecord {
            format: CREW_FORMAT,
            character: ObjId(7),
            location: CrewLocation::Unit { unit: ObjId(9) },
            condition: CrewCondition::Injured,
            at_ic: 0,
            organizer: None,
            extraction: None,
            treatment: None,
        };
        assert_eq!(record.extractable(), None);
        record.condition = CrewCondition::Incapacitated;
        assert_eq!(record.extractable(), Some(ObjId(9)));
        record.location = CrewLocation::Stranded {
            site: CanonicalId::new("ahuna_port").expect("canonical"),
            unit: ObjId(9),
        };
        record.condition = CrewCondition::Fit;
        assert_eq!(record.extractable(), Some(ObjId(9)));
        record.location = CrewLocation::Carrier { hull: ObjId(3) };
        assert_eq!(record.extractable(), None);
        let back: CrewRecord =
            serde_json::from_value(serde_json::to_value(&record).expect("encodes"))
                .expect("decodes");
        assert_eq!(back, record);
    }

    /// **Doc 36 cut GC.4** (Q-36-52's friendly half): a character stranded in a casualty, with
    /// an extraction asked for, is aboard the carrier once the casualty is loaded into it, and the
    /// extraction is gone; a character in another hull, or at a port, is untouched.
    #[test]
    fn gc4_a_friendly_load_sets_the_crew_aboard_and_cancels_the_extraction() {
        let site = CanonicalId::new("ahuna").expect("a site");
        let stranded = CrewRecord {
            format: CREW_FORMAT,
            character: ObjId(7),
            location: CrewLocation::Stranded {
                site,
                unit: ObjId(40),
            },
            condition: CrewCondition::Injured,
            at_ic: 100,
            organizer: Some(ObjId(3)),
            extraction: Some(Extraction {
                port: SmolStr::new("stickney"),
                requested_ic: 150,
                due_ic: 21_750,
                abandoned: false,
                forced: false,
                capture: None,
            }),
            treatment: None,
        };
        let aboard = loaded_aboard(&stranded, ObjId(40), ObjId(90), 500).expect("aboard");
        assert_eq!(aboard.location, CrewLocation::Carrier { hull: ObjId(90) });
        assert!(aboard.extraction.is_none(), "the extraction is cancelled");
        assert_eq!(
            aboard.condition,
            CrewCondition::Injured,
            "the hurt stay hurt"
        );
        assert_eq!(aboard.at_ic, 500);
        let in_unit = CrewRecord {
            location: CrewLocation::Unit { unit: ObjId(40) },
            extraction: None,
            ..stranded.clone()
        };
        assert!(loaded_aboard(&in_unit, ObjId(40), ObjId(90), 500).is_some());
        assert!(loaded_aboard(&stranded, ObjId(41), ObjId(90), 500).is_none());
        let home = CrewRecord {
            location: CrewLocation::Port {
                port: SmolStr::new("stickney"),
            },
            ..stranded
        };
        assert!(loaded_aboard(&home, ObjId(40), ObjId(90), 500).is_none());
    }

    /// **Doc 42 cut CR.1** (Q-38-22, Q-42-1, Q-42-16): a character inside a casualty loaded into a
    /// non-friend's bay is held aboard the carrier by the captor from the load's instant; the
    /// condition stays, an extraction asked for is gone; the record round-trips under the same
    /// format, and the cause the standing writes is the GC.10 site tag's.
    #[test]
    fn cr1_a_non_friend_load_captures_the_crew() {
        let site = CanonicalId::new("ahuna").expect("a site");
        let inside = CrewRecord {
            format: CREW_FORMAT,
            character: ObjId(7),
            location: CrewLocation::Stranded {
                site: site.clone(),
                unit: ObjId(40),
            },
            condition: CrewCondition::Incapacitated,
            at_ic: 100,
            organizer: Some(ObjId(3)),
            extraction: Some(Extraction {
                port: SmolStr::new("ceres"),
                requested_ic: 200,
                due_ic: 900,
                abandoned: false,
                forced: false,
                capture: None,
            }),
            treatment: None,
        };
        assert!(held_in(&inside, ObjId(40)), "the load carries them");
        let held = capture(&inside, ObjId(90), ObjId(12), 500);
        assert_eq!(
            held.location,
            CrewLocation::Captured {
                carrier: ObjId(90),
                captor: ObjId(12),
                since_ic: 500
            }
        );
        assert_eq!(
            held.condition,
            CrewCondition::Incapacitated,
            "the hurt stay hurt"
        );
        assert!(held.extraction.is_none(), "no extraction while held");
        assert_eq!(held.at_ic, 500);
        assert!(
            held.in_custody() && leaves_alone(&held),
            "a check-in leaves them alone"
        );
        assert!(
            !held_in(&held, ObjId(40)),
            "a second load carries them nowhere"
        );
        let back: CrewRecord =
            serde_json::from_value(serde_json::to_value(&held).expect("encodes")).expect("decodes");
        assert_eq!(back, held);
        let field = super::super::standing::cause_prefix(&site, 3);
        let cause = capture_cause(&site, 3, ObjId(7));
        assert_eq!(
            cause.strip_prefix(CAPTURE_CAUSE),
            field
                .strip_prefix("field:")
                .map(|tag| format!("{tag}7"))
                .as_deref()
        );
        let exit = NpcExit {
            format: NPC_EXIT_FORMAT,
            npc: ObjId(55),
            organizer: ObjId(3),
            recovered_ic: 500,
        };
        let back: NpcExit =
            serde_json::from_value(serde_json::to_value(&exit).expect("encodes")).expect("decodes");
        assert_eq!(back, exit);
        assert_eq!(npc_exit_key(ObjId(55)).as_str(), "tacsim:crew_exit:55");
    }

    /// **Doc 42 cut CR.2** (Q-42-7): the release fee's inspection key is one per handover, and
    /// `[tacsim.field].release_s` is banded an IC minute to an IC day.
    #[test]
    fn cr2_the_release_key_and_the_processing_band() {
        assert_eq!(release_key(ObjId(7), 4_100), "release:7:4100");
        let mut cfg = crate::config::FieldConfig::default();
        assert_eq!(cfg.release_s, 3_600);
        assert!(cfg.validate().is_ok());
        cfg.release_s = 59;
        assert!(cfg.validate().is_err_and(|e| e.contains("release_s")));
        cfg.release_s = 86_401;
        assert!(cfg.validate().is_err());
    }

    /// **Doc 42 cut CR.3** (Q-42-15): a captive of a carrier whose own check-in strands its souls
    /// is stranded in the wreck, custody gone; a sound carrier's check-in, or another unit's,
    /// leaves the captive alone.
    #[test]
    fn cr3_a_captors_wreck_strands_its_captives() {
        let site = CanonicalId::new("ahuna").expect("a site");
        let held = record_at(
            ObjId(7),
            CrewLocation::Captured {
                carrier: ObjId(90),
                captor: ObjId(12),
                since_ic: 100,
            },
        );
        let wreck = CrewLocation::Stranded {
            site: site.clone(),
            unit: ObjId(90),
        };
        let left = wrecked_captive(&held, &site, ObjId(90), wreck.clone()).expect("stranded");
        assert_eq!(left.location, wreck);
        assert!(!left.in_custody() && left.extractable() == Some(ObjId(90)));
        assert_eq!(left.condition, held.condition, "the condition kept");
        assert!(
            wrecked_captive(
                &held,
                &site,
                ObjId(90),
                CrewLocation::Carrier { hull: ObjId(90) }
            )
            .is_none(),
            "a sound carrier keeps its captive"
        );
        assert!(wrecked_captive(&held, &site, ObjId(91), wreck).is_none());
    }

    /// A record for the doc 42 tests: `who`, injured, at `location`.
    fn record_at(who: ObjId, location: CrewLocation) -> CrewRecord {
        CrewRecord {
            format: CREW_FORMAT,
            character: who,
            location,
            condition: CrewCondition::Injured,
            at_ic: 100,
            organizer: Some(ObjId(3)),
            extraction: None,
            treatment: None,
        }
    }

    /// **Doc 42 cut CR.0** (Q-42-1, Draft 0.3 42-2 and 42-5): the two new locations and the two
    /// new extraction fields decode under `CREW_FORMAT` 1; a record written before them decodes as
    /// before; the new fields are omitted when empty, so an old record re-encodes unchanged.
    #[test]
    fn cr0_the_custody_record_decodes_under_the_same_format() {
        let captured: CrewRecord = serde_json::from_value(serde_json::json!({
            "format": 1, "character": 7,
            "location": {"at": "captured", "carrier": 90, "captor": 12, "since_ic": 500},
            "condition": "injured", "at_ic": 500
        }))
        .expect("a captured record decodes");
        assert_eq!(
            captured.location,
            CrewLocation::Captured {
                carrier: ObjId(90),
                captor: ObjId(12),
                since_ic: 500
            }
        );
        assert!(captured.in_custody());
        assert_eq!(
            captured.extractable(),
            None,
            "a captive asks after the bound"
        );
        let processing: CrewRecord = serde_json::from_value(serde_json::json!({
            "format": 1, "character": 7,
            "location": {"at": "processing", "port": "ceres", "release_due_ic": 4_100},
            "condition": "fit", "at_ic": 500
        }))
        .expect("a processing record decodes");
        assert_eq!(
            processing.location,
            CrewLocation::Processing {
                port: SmolStr::new("ceres"),
                release_due_ic: 4_100,
                charge: None
            }
        );
        assert_eq!(processing.extractable(), None);
        let charged = CrewRecord {
            location: CrewLocation::Processing {
                port: SmolStr::new("ceres"),
                release_due_ic: 4_100,
                charge: Some(mush_economy::ObligationId(31)),
            },
            ..processing.clone()
        };
        let wire = serde_json::to_value(&charged).expect("encodes");
        assert_eq!(wire["location"]["charge"], 31);
        assert!(
            serde_json::to_value(&processing).expect("encodes")["location"]
                .get("charge")
                .is_none(),
            "an uncreated charge is omitted"
        );
        let ejected: CrewRecord = serde_json::from_value(serde_json::json!({
            "format": 1, "character": 7,
            "location": {"at": "stranded", "site": "ahuna", "unit": 40},
            "condition": "injured", "at_ic": 500,
            "extraction": {"port": "stickney", "requested_ic": 500, "due_ic": 22_100,
                "forced": true, "capture": {"party": "c", "carrier": 90, "captor": 12}}
        }))
        .expect("an ejectee's record decodes");
        let coming = ejected.extraction.clone().expect("the pickup");
        assert!(coming.forced && !coming.abandoned);
        assert_eq!(
            coming
                .capture
                .as_ref()
                .map(|c| (c.party.as_str(), c.carrier, c.captor)),
            Some(("c", ObjId(90), ObjId(12)))
        );
        for record in [captured, processing, charged, ejected] {
            let back: CrewRecord =
                serde_json::from_value(serde_json::to_value(&record).expect("encodes"))
                    .expect("decodes");
            assert_eq!(back, record);
        }
        // A record from before doc 42 decodes, and re-encodes with none of the new fields.
        let old = serde_json::json!({
            "format": 1, "character": 7,
            "location": {"at": "stranded", "site": "ahuna", "unit": 40},
            "condition": "fit", "at_ic": 100,
            "extraction": {"port": "stickney", "requested_ic": 150, "due_ic": 21_750}
        });
        let decoded: CrewRecord = serde_json::from_value(old.clone()).expect("an old record");
        assert_eq!(serde_json::to_value(&decoded).expect("encodes"), old);
    }

    /// **Doc 42 cut CR.0** (Q-42-3, Draft 0.3 42-5 (B)): a check-in leaves custody, an abandoned
    /// extraction and an ejectee's forced one alone; no load carries a captive, a soul in
    /// processing or an ejectee; a Friend's load still carries the stranded.
    #[test]
    fn cr0_check_in_and_the_loads_leave_custody_alone() {
        let site = CanonicalId::new("ahuna").expect("a site");
        let stranded = record_at(
            ObjId(7),
            CrewLocation::Stranded {
                site,
                unit: ObjId(40),
            },
        );
        assert!(!leaves_alone(&stranded));
        assert!(held_in(&stranded, ObjId(40)));
        let mut ejectee = stranded.clone();
        ejectee.extraction = Some(Extraction {
            port: SmolStr::new("stickney"),
            requested_ic: 100,
            due_ic: 21_700,
            abandoned: false,
            forced: true,
            capture: None,
        });
        assert!(leaves_alone(&ejectee), "the eject's draw is the record's");
        assert!(
            !held_in(&ejectee, ObjId(40)),
            "the ejectee rides where the hull goes"
        );
        assert!(loaded_aboard(&ejectee, ObjId(40), ObjId(90), 500).is_none());
        let captive = record_at(
            ObjId(7),
            CrewLocation::Captured {
                carrier: ObjId(40),
                captor: ObjId(12),
                since_ic: 100,
            },
        );
        assert!(leaves_alone(&captive));
        assert!(
            !held_in(&captive, ObjId(40)),
            "a captive's carrier is never a casualty"
        );
        let processing = record_at(
            ObjId(7),
            CrewLocation::Processing {
                port: SmolStr::new("ceres"),
                release_due_ic: 4_100,
                charge: None,
            },
        );
        assert!(leaves_alone(&processing));
        assert!(!held_in(&processing, ObjId(40)));
    }

    /// **Doc 42 cut CR.0** (Q-42-2, Draft 0.3 42-2, 42-5 (B), 42-6): the gate's refusals. A
    /// captive is refused every gate with the held line; an ejectee movement and operation; a
    /// character in processing only movement; a passenger nothing.
    #[test]
    fn cr0_the_held_gate_refuses_by_hold() {
        let mut w = World::new_minimal();
        let who = w
            .create(
                ObjKind::Player,
                "Pilot",
                mush_core::GOD,
                Some(mush_core::LIMBO),
            )
            .expect("a pilot");
        let carrier = w
            .create(
                ObjKind::Thing,
                "Kestrel",
                mush_core::GOD,
                Some(mush_core::LIMBO),
            )
            .expect("a carrier");
        let now = w.world_ic_now().at;
        let mut book = CrewBook::default();
        let gates = [Gate::Move, Gate::Operate, Gate::Extract, Gate::Chat];
        w.module_state_insert(book.clone());
        assert!(
            gates.iter().all(|gate| refusal(&w, who, *gate).is_none()),
            "no record"
        );
        book.0.insert(
            who,
            record_at(
                who,
                CrewLocation::Captured {
                    carrier,
                    captor: ObjId(12),
                    since_ic: now,
                },
            ),
        );
        w.module_state_insert(book.clone());
        assert_eq!(held(&w, who), Some(carrier));
        for gate in gates {
            let line = refusal(&w, who, gate).expect("a captive is refused");
            assert!(line.starts_with("You are held aboard the "), "{line}");
            if gate == Gate::Extract {
                // Doc 42 cut CR.3 (Q-42-8): the time left to the bound.
                assert!(line.contains("In about 168 IC h"), "{line}");
            } else {
                assert_eq!(
                    line,
                    HELD.replace("{carrier}", &crate::port::coop::name_of(&w, carrier))
                );
            }
        }
        // Doc 42 cut CR.3: past the bound the captive may ask; nothing else opens.
        let saved = book.clone();
        book.0.insert(
            who,
            record_at(
                who,
                CrewLocation::Captured {
                    carrier,
                    captor: ObjId(12),
                    since_ic: now - 604_800,
                },
            ),
        );
        w.module_state_insert(book.clone());
        assert!(refusal(&w, who, Gate::Extract).is_none());
        assert!(refusal(&w, who, Gate::Move).is_some());
        book = saved;
        w.module_state_insert(book.clone());
        let mut ejectee = record_at(
            who,
            CrewLocation::Stranded {
                site: CanonicalId::new("ahuna").expect("a site"),
                unit: ObjId(40),
            },
        );
        ejectee.extraction = Some(Extraction {
            port: SmolStr::new("stickney"),
            requested_ic: now,
            due_ic: now + 21_600,
            abandoned: false,
            forced: true,
            capture: None,
        });
        book.0.insert(who, ejectee);
        w.module_state_insert(book.clone());
        assert_eq!(held(&w, who), None);
        assert_eq!(refusal(&w, who, Gate::Move).as_deref(), Some(PICKUP));
        assert_eq!(refusal(&w, who, Gate::Operate).as_deref(), Some(PICKUP));
        assert_eq!(refusal(&w, who, Gate::Chat), None);
        book.0.insert(
            who,
            record_at(
                who,
                CrewLocation::Processing {
                    port: SmolStr::new("ceres"),
                    release_due_ic: now + 3_600,
                    charge: None,
                },
            ),
        );
        w.module_state_insert(book.clone());
        let line = refusal(&w, who, Gate::Move).expect("leaving the port is refused");
        assert_eq!(
            line,
            PROCESSING
                .replace("{port}", port_name("ceres"))
                .replace("{h}", "1")
        );
        for gate in [Gate::Operate, Gate::Extract, Gate::Chat] {
            assert_eq!(refusal(&w, who, gate), None, "{gate:?}");
        }
        book.0
            .insert(who, record_at(who, CrewLocation::Carrier { hull: carrier }));
        w.module_state_insert(book);
        assert!(
            gates.iter().all(|gate| refusal(&w, who, *gate).is_none()),
            "a passenger"
        );
    }
}
