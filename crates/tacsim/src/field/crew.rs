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
//! console is fit. No ejection or capture (Q-CF-38).
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
//! abandoned, until capture is built. Their extraction starts the moment the load completes, as
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
}

/// **How a character is** (Q-CF-38).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
            CrewLocation::Unit { .. }
            | CrewLocation::Carrier { .. }
            | CrewLocation::Port { .. } => None,
        }
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
/// location; `None` for a character the field engine never touched.
#[must_use]
pub fn crew_location(w: &World, who: ObjId) -> Option<CrewLocation> {
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
    let extraction = crew_of(ctx.w, pilot).and_then(|record| record.extraction.clone());
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
    let held = match &record.location {
        CrewLocation::Unit { unit } | CrewLocation::Stranded { unit, .. } => *unit == casualty,
        CrewLocation::Carrier { .. } | CrewLocation::Port { .. } => false,
    };
    if !held {
        return None;
    }
    let mut next = record.clone();
    next.location = CrewLocation::Carrier { hull: carrier };
    next.extraction = None;
    next.at_ic = now;
    Some(next)
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
                .filter(|record| match &record.location {
                    CrewLocation::Unit { unit } | CrewLocation::Stranded { unit, .. } => {
                        *unit == casualty
                    }
                    CrewLocation::Carrier { .. } | CrewLocation::Port { .. } => false,
                })
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
        CrewLocation::Carrier { hull } => *hull,
        CrewLocation::Port { .. } => ObjId(0),
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
        if condition != CrewCondition::Incapacitated
            || crew_of(ctx.w, *pilot).is_some_and(|record| {
                record.condition == CrewCondition::Incapacitated
                    && record.location == CrewLocation::Unit { unit: frame.unit }
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
            CrewLocation::Carrier { .. } | CrewLocation::Port { .. } => (None, None),
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
    };
    let mut line = format!("Crew: {condition}, {at}");
    if let Some(coming) = &record.extraction {
        line.push_str(&format!(
            "; extraction to {} in about {} IC h",
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
}
