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
//! **Abandonment** (doc 36 cut GC.5; Q-36-52, the owner's ruling): a casualty loaded into a
//! carrier that is not its crew's own party's or a Friend's leaves everyone still aboard it
//! abandoned, until doc 42's cut CR.1 captures them instead (Q-38-22, Q-42-16). Their extraction starts the moment the load completes, as
//! if each had asked for it (`abandoned_loaded`), and it is never deferred for a recovery
//! flight ([`Extraction::abandoned`]). The hull stays with the carrier.
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
    let hold = crew_of(w, who)?.hold()?;
    refusal_for(w, &hold, gate)
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

/// **Read the boot's crew records** (`on_world_loaded`).
pub(crate) fn read(ctx: &mut Ctx<'_>, kv: &[(SmolStr, serde_json::Value)]) {
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
    let Some(cells) = mush_tactics::module::unit_of(w, unit) else {
        return CrewCondition::Fit;
    };
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
fn organizer_of(w: &World, site: &CanonicalId, unit: ObjId) -> Option<ObjId> {
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
        let home = assessment.as_ref().is_some_and(|c| {
            c.has_power
                && c.has_control
                && c.can_move
                && !matches!(
                    c.state,
                    crate::field_ops::OperationalState::Wreck
                        | crate::field_ops::OperationalState::Irrecoverable
                )
        });
        let location = if home {
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
/// each soul still aboard is recorded aboard it ([`check_in`]'s carrier branch).
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
/// Friend's; a load into anyone else's bay is GC.5's abandonment.
pub(crate) fn aboard_loaded(ctx: &mut Ctx<'_>, casualty: ObjId, carrier: ObjId) {
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

/// **The crew abandoned in a loaded casualty**: `{name}`, `{carrier}`, `{port}`, `{h}`.
pub const ABANDONED: &str = "The {name} is hauled into the {carrier}'s bay, a carrier not of your party or a Friend's, with you still in it: you are abandoned, and an extraction to {port} is on its way, about {h} IC hours.";

/// **A casualty loaded into a non-friend's bay** (doc 36 cut GC.5; Q-36-52, the owner's ruling):
/// every character whose record holds them in `casualty` — aboard it, or stranded in it — is
/// abandoned. Each is stranded on `site` in the hull and their extraction starts now, as if they
/// had asked for it, to the safe port they know, never deferred for a recovery flight; one already
/// on its way keeps its port and due instant and loses its deferral. The caller
/// (`crate::surface::tow`) has checked that the carrier is not of their party or a Friend's.
pub(crate) fn abandoned_loaded(
    ctx: &mut Ctx<'_>,
    site: &CanonicalId,
    casualty: ObjId,
    carrier: ObjId,
) {
    let now = ctx.w.world_ic_now().at;
    let delay = i64::from(super::config(ctx.w).extract_s);
    let held: Vec<CrewRecord> = ctx
        .w
        .module_state::<CrewBook>()
        .map(|book| {
            book.0
                .values()
                .filter(|record| held_in(record, casualty))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    for record in held {
        let who = record.character;
        let Some(port) = destination(ctx.w, who) else {
            tracing::warn!(
                who = who.0,
                "tacsim: an abandoned character knows no safe port"
            );
            continue;
        };
        let next = abandon(&record, site, (port.id, delay), now);
        let due = next.extraction.as_ref().map_or(now, |coming| coming.due_ic);
        let name = crate::ic_return::PORTS
            .iter()
            .find(|known| Some(known.id) == next.extraction.as_ref().map(|c| c.port.as_str()))
            .map_or(port.name, |known| known.name);
        put(ctx, next);
        ctx.out.sys_to_actor(
            who,
            "tacsim.field",
            ABANDONED
                .replace("{name}", &crate::port::coop::name_of(ctx.w, casualty))
                .replace("{carrier}", &crate::port::coop::name_of(ctx.w, carrier))
                .replace("{port}", name)
                .replace("{h}", &hours(due - now).to_string()),
        );
    }
}

/// **[`abandoned_loaded`]'s rule for one record**: stranded on `site` in the hull it was in, with
/// an extraction to `port` due `delay` from `now`, abandoned (an extraction already coming keeps
/// its port and due instant).
#[must_use]
pub(crate) fn abandon(
    record: &CrewRecord,
    site: &CanonicalId,
    (port, delay): (&str, i64),
    now: i64,
) -> CrewRecord {
    let unit = match &record.location {
        CrewLocation::Unit { unit } | CrewLocation::Stranded { unit, .. } => *unit,
        CrewLocation::Carrier { hull } | CrewLocation::Captured { carrier: hull, .. } => *hull,
        CrewLocation::Port { .. } | CrewLocation::Processing { .. } => ObjId(0),
    };
    let mut next = record.clone();
    next.location = CrewLocation::Stranded {
        site: site.clone(),
        unit,
    };
    next.extraction = Some(match &record.extraction {
        Some(coming) => Extraction {
            abandoned: true,
            ..coming.clone()
        },
        None => Extraction {
            port: SmolStr::new(port),
            requested_ic: now,
            due_ic: now.saturating_add(delay),
            abandoned: true,
            forced: false,
            capture: None,
        },
    });
    next.at_ic = now;
    next
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
fn destination(w: &World, who: ObjId) -> Option<&'static crate::ic_return::IcPort> {
    crate::ic_return::last_port(w, who).or_else(|| {
        crate::ic_return::PORTS
            .iter()
            .find(|port| port.id == FALLBACK_PORT)
    })
}

/// IC hours, rounded up.
fn hours(seconds: i64) -> i64 {
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
    record
        .extractable()
        .filter(|unit| inside(ctx.w, actor, *unit))
        .ok_or_else(|| CmdError::msg(NOT_STRANDED))?;
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
        EXTRACT_TAKEN
            .replace("{port}", port.name)
            .replace("{h}", &hours(delay).to_string()),
    );
    Ok(())
}

/// Whether a Friend's recovery flight stands on `site` for `organizer` (this module's head).
fn rescue_on_site(ctx: &Ctx<'_>, site: &CanonicalId, organizer: Option<ObjId>) -> bool {
    let Some(organizer) = organizer else {
        return false;
    };
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
            .is_some_and(|op| {
                op.roster.organizer == organizer
                    && op.roster.duty == crate::detachment::crew::CrewDuty::Recovery
            })
    })
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
        if let Some(site) = &site
            && !coming.abandoned
            && now < coming.due_ic.saturating_add(bound)
            && rescue_on_site(ctx, site, record.organizer)
        {
            continue;
        }
        land(ctx, record, &coming, site.as_ref(), unit);
    }
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
    if let Some(unit) = unit
        && let Some((seated, station)) = super::parties::seated(ctx.w, unit)
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

    /// **Doc 36 cut GC.5** (Q-36-52, the owner's ruling): a character inside a casualty loaded
    /// into a non-friend's bay is stranded in it with an abandoned extraction due after the
    /// delay; one already coming keeps its port and due instant and only loses its deferral; the
    /// flag round-trips and is omitted when false.
    #[test]
    fn gc5_a_non_friend_load_abandons_the_crew_to_extraction() {
        let site = CanonicalId::new("ahuna").expect("a site");
        let inside = CrewRecord {
            format: CREW_FORMAT,
            character: ObjId(7),
            location: CrewLocation::Unit { unit: ObjId(40) },
            condition: CrewCondition::Incapacitated,
            at_ic: 100,
            organizer: Some(ObjId(3)),
            extraction: None,
            treatment: None,
        };
        let left = abandon(&inside, &site, ("stickney", 3_600), 500);
        assert_eq!(
            left.location,
            CrewLocation::Stranded {
                site: site.clone(),
                unit: ObjId(40)
            }
        );
        let coming = left.extraction.clone().expect("the extraction starts");
        assert!(coming.abandoned, "no recovery flight defers it");
        assert_eq!((coming.requested_ic, coming.due_ic), (500, 4_100));
        assert_eq!(coming.port.as_str(), "stickney");
        let asked = CrewRecord {
            extraction: Some(Extraction {
                port: SmolStr::new("ceres"),
                requested_ic: 200,
                due_ic: 900,
                abandoned: false,
                forced: false,
                capture: None,
            }),
            ..inside
        };
        let kept = abandon(&asked, &site, ("stickney", 3_600), 500)
            .extraction
            .expect("still coming");
        assert_eq!(
            (kept.port.as_str(), kept.due_ic, kept.abandoned),
            ("ceres", 900, true)
        );
        let wire = serde_json::to_value(&asked).expect("encodes");
        assert!(
            wire["extraction"].get("abandoned").is_none(),
            "omitted when false"
        );
        let back: CrewRecord =
            serde_json::from_value(serde_json::to_value(&left).expect("encodes")).expect("decodes");
        assert_eq!(back, left);
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
            assert_eq!(
                line,
                HELD.replace("{carrier}", &crate::port::coop::name_of(&w, carrier))
            );
            assert!(line.starts_with("You are held aboard the "), "{line}");
        }
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
