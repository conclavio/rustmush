//! **One picture of the site** (doc 32 cut CF.4; Q-CF-48, Q-CF-49, Q-CF-50, with Q-CF-26 and
//! Q-CF-28): the campaign layer, built once, and the field layer, read once, which every
//! rendering of a site draws.
//!
//! **The campaign layer** ([`CampaignLayer`], [`campaign_layer`]) is what a viewer knows of a
//! site whether or not it is promoted: the blocks' letters, the live claims' edges, the public
//! pits, the viewer's own readings, the carrier's pad, the presence picture (Q-CF-15) and the
//! salvage the viewer sees now or remembers (Q-CF-28). `+tv/map` and the quarry screens draw it
//! into the `site.map` grid ([`super::site_map_frame`] maps it into frame cells); the cockpit
//! frame of a unit in an IC field carries it as its `campaign` block ([`campaign_value`], site
//! cells), which the TUI's and the web's cockpit maps and the pixel cockpit's map window draw
//! under the contacts. Two renderings of one builder, so the two cannot disagree about a cell.
//!
//! **The field layer** is the kernel's: contacts coloured by relation, the lock, the own unit.
//! The cockpit frame is it; `site.map` reads the viewer's own held cockpit frame
//! ([`field_layer`]) when it is the field on the very site the map shows, and draws its contacts
//! with the cockpit's own glyphs (`x` hostile, `+` own or friend, `?` neither). A viewer with no
//! seat in the field holds no contacts, and the map says so rather than draw anyone's.
//!
//! **Ground objects in a field** (Q-CF-26): remains and crates are static obstacles over their
//! footprints that hide what is behind them (`crate::field::ground`), and sensor returns: a unit
//! sees one in line of sight within [`crate::surface::presence::SIGHT_CELLS`]
//! ([`crate::field::known::sensed`]), and the frame carries it in `ground` beside the contacts.
//! Not lockable: click-to-lock is the field layer's contacts only (Q-CF-50).
//!
//! **The rig's cab is a cockpit** (Q-CF-49): a rig admitted to a field flies the kernel's
//! cockpit frame like any hull, and [`enrich_cockpit`] adds the quarry screen's rig state — the
//! same row `site.rigs` carries — as its `rig` panel.
//!
//! D-21-14 throughout: every glyph and row is decided here, on the server; a client draws what
//! the frame says and invents nothing.

use std::collections::BTreeSet;

use mush_core::{CanonicalId, ObjId, World};
use serde_json::{Map, Value, json};

use super::{
    ClaimMark, OreFamily, PAD_CELL, Scanned, Sounded, SurfaceReads, Word, bearing_words,
    claim_edges, claimed_blocks, pit_layer, pocket_cell, rank, reading_glyph, viewer_readings,
};
use crate::surface::address;
use crate::surface::chart::{self, SitePeek};
use crate::surface::excursion::cells_apart;
use crate::surface::presence::{Flight, SIGHT_CELLS, Seen, seen_from_heard};
use crate::surface::site_record::{FieldId, GroundKind, KnownWhat, SiteRecord, object_words};

/// **The glyph of a wreck, a casualty or remains** on every map of the site.
pub const SALVAGE_GLYPH: char = '%';
/// **The glyph of a crate** (dropped cargo).
pub const CRATE_GLYPH: char = '&';
/// The legend's pair for [`SALVAGE_GLYPH`].
pub const SALVAGE_LABEL: &str = "wreck or remains";
/// The legend's pair for [`CRATE_GLYPH`].
pub const CRATE_LABEL: &str = "crate";

/// **One thing on the ground as the viewer knows it** (Q-CF-28).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Salvage {
    /// What it is (or was, when remembered).
    pub what: KnownWhat,
    /// Its site cell.
    pub cell: (i64, i64),
    /// Seen now (in sight, or held by a sensor in a field); else remembered.
    pub now: bool,
    /// Its words: a crate seen now names its manifest (`a crate of 2 metal_ore`), anything
    /// remembered only what it was (`a crate`).
    pub words: String,
}

impl Salvage {
    /// Its glyph.
    #[must_use]
    pub fn glyph(&self) -> char {
        match self.what {
            KnownWhat::Crate => CRATE_GLYPH,
            KnownWhat::Casualty | KnownWhat::Wreck | KnownWhat::Remains => SALVAGE_GLYPH,
        }
    }

    /// A unit left on the ground, seen now.
    fn of_unit(unit: &crate::surface::site_record::LeftUnit) -> Salvage {
        Salvage {
            what: KnownWhat::of_unit(unit.condition),
            cell: unit.pose.cell(),
            now: true,
            words: format!("a {}", unit.condition.word()),
        }
    }

    /// An object on the ground, seen now.
    pub(crate) fn of_object(object: &crate::surface::site_record::GroundObject) -> Salvage {
        Salvage {
            what: KnownWhat::of_object(&object.kind),
            cell: object.pose.cell(),
            now: true,
            words: match &object.kind {
                GroundKind::Remains { .. } => "remains".to_owned(),
                GroundKind::Cargo { .. } => object_words(&object.kind),
            },
        }
    }

    /// A remembered thing.
    fn remembered(item: &crate::surface::site_record::KnownItem) -> Salvage {
        Salvage {
            what: item.0,
            cell: item.cell(),
            now: false,
            words: match item.0 {
                KnownWhat::Remains => "remains".to_owned(),
                what => format!("a {}", what.word()),
            },
        }
    }
}

/// **How the viewer sees the ground right now.**
#[derive(Clone, Debug)]
pub(crate) enum Eyes<'a> {
    /// On the open surface: the viewer's rigs on the ground, `(slot, cell)` — CF.1's rule, the
    /// record's units and objects within [`SIGHT_CELLS`] of one.
    Surface(&'a [(u8, (i64, i64))]),
    /// In a field: the viewer's unit's cell and the objects its sensors hold (the left units
    /// are kernel units there, so they are contacts, not salvage).
    Field {
        /// The unit's cell.
        from: (i64, i64),
        /// The objects held.
        sensed: Vec<Salvage>,
    },
}

impl Eyes<'_> {
    /// Whether `cell` is within sight of these eyes (a remembered thing there is superseded by
    /// what the eyes say now).
    fn covers(&self, cell: (i64, i64)) -> bool {
        match self {
            Eyes::Surface(observers) => observers
                .iter()
                .any(|(_, at)| cells_apart(*at, cell) <= SIGHT_CELLS),
            Eyes::Field { from, .. } => cells_apart(*from, cell) <= SIGHT_CELLS,
        }
    }
}

/// **The campaign layer** (Q-CF-48), in site cells.
pub(crate) struct CampaignLayer {
    /// Each block's letter at its north-west corner cell.
    pub letters: Vec<((i64, i64), char)>,
    /// The live claims.
    pub claimed: Vec<ClaimMark>,
    /// Their edges' cells (the letters' corners excluded).
    pub claim_cells: Vec<(i64, i64)>,
    /// The public pits at their pockets' cells, `v`/`V`.
    pub pits: Vec<((i64, i64), char)>,
    /// The viewer's sounded pockets.
    pub sounded: Vec<Sounded>,
    /// The viewer's scanned fields.
    pub scanned: Vec<Scanned>,
    /// The carrier's pad, when a hull stands on it.
    pub pad: Option<(i64, i64)>,
    /// The presence picture.
    pub seen: Vec<Seen>,
    /// The salvage: seen now first, nearest first, then remembered.
    pub salvage: Vec<Salvage>,
    /// The field open on the site.
    pub field: Option<FieldId>,
    /// **The site's cells along one side** (doc 32 cut S.0, §18): its ground composite's.
    pub side: u16,
}

/// **What [`campaign_layer`] reads besides the world.**
pub(crate) struct LayerInputs<'a> {
    /// The site.
    pub entry: &'a CanonicalId,
    /// Its chart context.
    pub site: &'a chart::SiteContext,
    /// Its keyed ground.
    pub ground: &'a crate::surface::SiteGround,
    /// The viewer's rigs on the ground, `(slot, cell)` — presence is seen from the nearest.
    pub observers: &'a [(u8, (i64, i64))],
    /// Every flight on the site's ground at the layer's instant.
    pub flights: &'a [Flight],
    /// The viewer's own (never on their presence picture).
    pub own: &'a BTreeSet<ObjId>,
    /// Flights heard sounding inside the IC hour.
    pub heard: &'a BTreeSet<ObjId>,
    /// How the viewer sees the ground now.
    pub eyes: Eyes<'a>,
    /// The instant.
    pub now: i64,
}

/// **Whether a hull stands on the site's pad** (the map's `H`).
fn hull_on_pad(w: &World, entry: &CanonicalId) -> bool {
    crate::port::coop::programme_port(w).is_some_and(|port| {
        port.map.hulls.iter().any(|row| {
            matches!(
                crate::port::ship::berth_of(w, ObjId(row.hull)),
                crate::port::ship::Berth::Docked { map, .. } if map.entry == *entry
            )
        })
    })
}

/// **The campaign layer for `actor` on a site** (Q-CF-48).
pub(crate) fn campaign_layer(
    w: &World,
    reads: &dyn SurfaceReads,
    actor: ObjId,
    inputs: &LayerInputs<'_>,
) -> CampaignLayer {
    let block_side = address::CELLS_PER_BLOCK;
    let mut letters = Vec::new();
    for by in 0..crate::surface::ground::SITE_BLOCKS {
        for bx in 0..crate::surface::ground::SITE_BLOCKS {
            if let Some(letter) = address::letter_of_block(bx, by) {
                letters.push(((bx * block_side, by * block_side + block_side - 1), letter));
            }
        }
    }
    let claimed = claimed_blocks(w, reads, inputs.entry, actor, inputs.now);
    let claimed_letters: Vec<char> = claimed.iter().map(|mark| mark.letter).collect();
    let claim_cells = claim_edges(&claimed_letters)
        .into_iter()
        .map(|(cell, _)| cell)
        .collect();
    let pits = pit_layer(reads, inputs.entry, inputs.ground)
        .into_iter()
        .map(|(pocket, glyph)| (pocket_cell(pocket), glyph))
        .collect();
    let row = reads.chart(actor, inputs.entry).ok().flatten();
    let (mut sounded, mut scanned) = viewer_readings(inputs.site, row.as_ref());
    // Doc 32 §17 (G-7): inside a field, the readings of every principal of the viewer's party
    // join the viewer's own on the map, for display only; the viewer's own reading of a pocket or
    // a field wins, and a mate's never writes the viewer's chart.
    let mates = crate::field::known::chart_mates(w, inputs.entry, actor, |unit| {
        reads
            .rig(unit)
            .ok()
            .flatten()
            .and_then(|record| record.excursion.map(|out| out.operator))
    });
    for mate in mates {
        let row = reads.chart(mate, inputs.entry).ok().flatten();
        let (theirs, fields) = viewer_readings(inputs.site, row.as_ref());
        for reading in theirs {
            if !sounded.iter().any(|own| own.pocket == reading.pocket) {
                sounded.push(reading);
            }
        }
        for reading in fields {
            if !scanned.iter().any(|own| own.field == reading.field) {
                scanned.push(reading);
            }
        }
    }
    let pad = hull_on_pad(w, inputs.entry).then_some(PAD_CELL);
    let seen = seen_from_heard(
        inputs.flights,
        inputs.own,
        inputs.observers,
        PAD_CELL,
        inputs.heard,
    );
    let record = reads.site_record(inputs.entry).ok().flatten();
    let salvage = record
        .as_ref()
        .map(|record| salvage_of(record, actor, &inputs.eyes))
        .unwrap_or_default();
    CampaignLayer {
        letters,
        claimed,
        claim_cells,
        pits,
        sounded,
        scanned,
        pad,
        seen,
        salvage,
        field: record.as_ref().and_then(SiteRecord::open_field),
        side: crate::surface::ground::ground_side(&inputs.ground.composite),
    }
}

/// **The salvage `actor` knows on a site** (Q-CF-28): what the eyes see now — on the surface the
/// record's units and objects in sight of a rig, in a field the objects the unit's sensors hold
/// — nearest first; then what `actor` remembers that the eyes do not cover now, most recently
/// learned first. Never the whole record.
pub(crate) fn salvage_of(record: &SiteRecord, actor: ObjId, eyes: &Eyes<'_>) -> Vec<Salvage> {
    let mut now: Vec<Salvage> = match eyes {
        Eyes::Surface(_) => record
            .units
            .iter()
            .map(Salvage::of_unit)
            .chain(record.objects.iter().map(Salvage::of_object))
            .filter(|thing| eyes.covers(thing.cell))
            .collect(),
        Eyes::Field { sensed, .. } => sensed.clone(),
    };
    let nearest = |cell: (i64, i64)| match eyes {
        Eyes::Surface(observers) => observers
            .iter()
            .map(|(_, at)| cells_apart(*at, cell))
            .min()
            .unwrap_or(u64::MAX),
        Eyes::Field { from, .. } => cells_apart(*from, cell),
    };
    now.sort_by(|a, b| (nearest(a.cell), &a.words).cmp(&(nearest(b.cell), &b.words)));
    let remembered = record
        .known_by(actor)
        .iter()
        .filter(|item| !eyes.covers(item.cell()))
        .map(Salvage::remembered);
    now.into_iter().chain(remembered).collect()
}

/// **A sounded pocket's glyph with no family chosen** (the cockpit's map, which has no family
/// selector): the best of the pocket's four families by the map's own rank.
fn best_glyph(classes: &[Word; 4]) -> char {
    OreFamily::ALL
        .into_iter()
        .map(|family| reading_glyph(classes, family))
        .max_by_key(|glyph| rank(*glyph))
        .unwrap_or('.')
}

/// **The campaign layer's marks**, in site cells and drawing order, bottom first: the letters,
/// the claims' edges (never over a letter), the pits, the readings (in `family`, else each
/// pocket's best), the pad's `H`, the salvage. A later mark over an earlier one on the same cell
/// wins, exactly as the grid's own composition.
pub(crate) fn campaign_marks(
    layer: &CampaignLayer,
    family: Option<OreFamily>,
) -> Vec<((i64, i64), char)> {
    let letter_cells: BTreeSet<(i64, i64)> = layer.letters.iter().map(|(at, _)| *at).collect();
    let mut marks: Vec<((i64, i64), char)> = layer.letters.clone();
    marks.extend(
        layer
            .claim_cells
            .iter()
            .filter(|at| !letter_cells.contains(at))
            .map(|at| (*at, '#')),
    );
    marks.extend(layer.pits.iter().copied());
    marks.extend(layer.sounded.iter().map(|s| {
        let glyph = match family {
            Some(family) => reading_glyph(&s.classes, family),
            None => best_glyph(&s.classes),
        };
        (pocket_cell(s.pocket), glyph)
    }));
    if let Some(at) = layer.pad {
        marks.push((at, 'H'));
    }
    marks.extend(
        layer
            .salvage
            .iter()
            .map(|thing| (thing.cell, thing.glyph())),
    );
    marks
}

/// **The campaign layer as the cockpit frame's `campaign` block** (Q-CF-50), site cells:
///
/// `{ site, side, block_side, field, marks: [[x, y, glyph], …], legend: [[glyph, label], …],
///   claims: [{letter, state, holder, yours}], salvage: [{what, cell, seen, words, m, dir}],
///   presence: [Seen rows], presence_line }`
///
/// `marks` are drawn under the contacts and the own marker, in order; `legend` names only the
/// glyphs `marks` holds; `salvage[].m`/`dir` are from `from`, the unit's own cell.
pub(crate) fn campaign_value(layer: &CampaignLayer, site_name: &str, from: (i64, i64)) -> Value {
    let marks = campaign_marks(layer, None);
    let glyphs: BTreeSet<char> = marks.iter().map(|(_, glyph)| *glyph).collect();
    let mut legend: Vec<Value> = Vec::new();
    for (glyph, label) in [
        ('#', "claim"),
        ('v', "pit, worked"),
        ('V', "pit, worked out"),
        ('3', "rich, 3 t a cycle"),
        ('2', "fair, 2 t a cycle"),
        ('1', "lean, 1 t a cycle"),
        ('+', "other family"),
        ('.', "barren"),
        ('H', "hull"),
        (SALVAGE_GLYPH, SALVAGE_LABEL),
        (CRATE_GLYPH, CRATE_LABEL),
    ] {
        if glyphs.contains(&glyph) {
            legend.push(json!([glyph.to_string(), label]));
        }
    }
    let salvage: Vec<Value> = layer
        .salvage
        .iter()
        .map(|thing| {
            let (m, dir) = bearing_words(from, thing.cell);
            json!({
                "what": thing.what.word(),
                "cell": [thing.cell.0, thing.cell.1],
                "seen": if thing.now { "now" } else { "remembered" },
                "words": thing.words,
                "m": m,
                "dir": dir,
            })
        })
        .collect();
    let claims: Vec<Value> = layer
        .claimed
        .iter()
        .map(|mark| {
            json!({
                "letter": mark.letter.to_string(),
                "state": if mark.held { "held" } else { "claimed" },
                "holder": mark.holder,
                "yours": mark.yours,
            })
        })
        .collect();
    json!({
        "site": site_name,
        "side": layer.side,
        "block_side": address::CELLS_PER_BLOCK,
        "field": layer.field.as_ref().map(|field| field.ordinal),
        "marks": marks
            .iter()
            .map(|((x, y), glyph)| json!([x, y, glyph.to_string()]))
            .collect::<Vec<_>>(),
        "legend": legend,
        "claims": claims,
        "salvage": salvage,
        "presence": layer.seen.iter().map(Seen::value).collect::<Vec<_>>(),
        "presence_line": crate::surface::presence::presence_line(&layer.seen),
    })
}

/// **The field layer `site.map` draws** (Q-CF-48): the viewer's own held cockpit frame, when it
/// is of the field open on `entry`.
pub(crate) struct FieldLayer {
    /// The unit the viewer sits in, and its cell.
    pub unit: (ObjId, Option<(i64, i64)>),
    /// Its callsign, as the cockpit prints it.
    pub callsign: String,
    /// Each contact: its cell, glyph, designator, relation word and party letter.
    pub contacts: Vec<FieldContact>,
    /// The designator locked, if any.
    pub lock: Option<String>,
}

/// One contact of a [`FieldLayer`].
pub(crate) struct FieldContact {
    /// Its last-seen cell.
    pub cell: (i64, i64),
    /// The cockpit's glyph for it.
    pub glyph: char,
    /// The designator a pilot types.
    pub designator: String,
    /// The viewer's relation to it.
    pub relation: &'static str,
    /// Its class word.
    pub class: String,
}

/// **The viewer's field layer on `entry`**, when they sit in a unit of the field open there.
pub(crate) fn field_layer(w: &World, actor: ObjId, entry: &CanonicalId) -> Option<FieldLayer> {
    let registry = w.module_state::<crate::arena_bridge::ArenaRegistry>()?;
    registry.seat_of_pilot(actor)?;
    let held = registry.last_frame(actor)?;
    let book = w.module_state::<crate::field::FieldBook>()?;
    if book.site_of_task(held.match_id) != Some(entry) {
        return None;
    }
    let frame = &held.frame;
    let contacts = crate::arena::view::frame_contacts_held(frame, &held.ranges)
        .into_iter()
        .filter_map(|row| {
            let (x, y) = row.cell?;
            Some(FieldContact {
                cell: (i64::from(x), i64::from(y)),
                glyph: row.glyph().chars().next().unwrap_or('?'),
                designator: row.designator.to_string(),
                relation: row.relation.as_str(),
                class: row.class.to_string(),
            })
        })
        .collect();
    let lock = frame.lock.and_then(|id| {
        crate::arena::view::frame_contacts(frame)
            .into_iter()
            .find(|row| row.id == id)
            .map(|row| row.designator.to_string())
    });
    Some(FieldLayer {
        unit: (
            frame.unit,
            frame.cell.map(|(x, y)| (i64::from(x), i64::from(y))),
        ),
        callsign: frame.callsign.to_string(),
        contacts,
        lock,
    })
}

/// **A cockpit keyframe for `actor`, with the field's picture** ([`enrich_cockpit`]): what every
/// path that hands a pilot a cockpit frame — the pulse's push, the feed's query, `VIEW`, the
/// detachment's refresh — passes its payload through, beside `detachment::commands::enrich`.
pub(crate) fn cockpit_payload(
    w: &World,
    reads: &dyn SurfaceReads,
    actor: ObjId,
    mut payload: Value,
) -> Value {
    if let Some(object) = payload.as_object_mut() {
        enrich_cockpit(w, reads, actor, object);
    }
    payload
}

/// **The cockpit frame's campaign block, sensed ground and rig panel** (Q-CF-49, Q-CF-50): added
/// to `payload` — a cockpit keyframe for `actor` — only when the frame `actor` last held is of
/// an IC field, so every simulator frame is byte-identical to CF.2's. Each key is removed again
/// when it no longer applies, as `detachment` is.
pub(crate) fn enrich_cockpit(
    w: &World,
    reads: &dyn SurfaceReads,
    actor: ObjId,
    payload: &mut Map<String, Value>,
) {
    for key in ["campaign", "ground", "rig"] {
        payload.remove(key);
    }
    let Some(registry) = w.module_state::<crate::arena_bridge::ArenaRegistry>() else {
        return;
    };
    let Some(held) = registry.last_frame(actor) else {
        return;
    };
    let Some(entry) = w
        .module_state::<crate::field::FieldBook>()
        .and_then(|book| book.site_of_task(held.match_id).cloned())
    else {
        return;
    };
    let frame = &held.frame;
    let Some(from) = frame.cell.map(|(x, y)| (i64::from(x), i64::from(y))) else {
        return;
    };
    let Ok(site) = chart::site_context(w, &entry) else {
        return;
    };
    let mut peeks = crate::surface::Peeks::default();
    let SitePeek::Warm(ground) = chart::peek_site(w, &entry, &mut peeks) else {
        return;
    };
    let record = reads.site_record(&entry).ok().flatten();
    // Q-CF-26: the objects this unit's sensors hold, by line of sight over the field's own ground.
    let sensed: Vec<Salvage> = match (
        record.as_ref(),
        registry.arena_of(held.match_id).map(|arena| &arena.terrain),
    ) {
        (Some(record), Some(terrain)) => {
            crate::field::known::sensed(terrain, (from, frame.sensor_height_cm), record)
                .into_iter()
                .map(Salvage::of_object)
                .collect()
        }
        _ => Vec::new(),
    };
    let flights: Vec<Flight> = crate::surface::tick::presence_book(w)
        .and_then(|book| book.sites.get(&entry).cloned())
        .unwrap_or_default();
    let own: BTreeSet<ObjId> = [frame.unit].into_iter().collect();
    let observers = [(0_u8, from)];
    let layer = campaign_layer(
        w,
        reads,
        actor,
        &LayerInputs {
            entry: &entry,
            site: &site,
            ground: &ground,
            observers: &observers,
            flights: &flights,
            own: &own,
            heard: &BTreeSet::new(),
            eyes: Eyes::Field {
                from,
                sensed: sensed.clone(),
            },
            now: w.world_ic_now().at,
        },
    );
    let name = crate::surface::act::site_name(w, &entry);
    payload.insert("campaign".to_owned(), campaign_value(&layer, &name, from));
    if !sensed.is_empty() {
        let rows: Vec<Value> = sensed
            .iter()
            .map(|thing| {
                let (m, dir) = bearing_words(from, thing.cell);
                json!({
                    "what": thing.what.word(),
                    "cell": [thing.cell.0, thing.cell.1],
                    "glyph": thing.glyph().to_string(),
                    "words": thing.words,
                    "m": m,
                    "dir": dir,
                })
            })
            .collect();
        payload.insert("ground".to_owned(), Value::Array(rows));
    }
    if let Some(rig) = super::rig_panel(w, reads, actor, frame.unit, from) {
        payload.insert("rig".to_owned(), rig);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::site_record::{KnownItem, SitePose};

    fn record() -> SiteRecord {
        let mut site = SiteRecord::new(CanonicalId::new("ahuna_port").unwrap_or_else(|_| {
            panic!("a canonical id");
        }));
        let manifest = [(
            CanonicalId::new("metal_ore").unwrap_or_else(|_| panic!("id")),
            2,
        )]
        .into_iter()
        .collect();
        let _ = site.drop_object(
            SitePose::at_cell(105, 100),
            1,
            GroundKind::Cargo {
                manifest,
                from: None,
            },
            1,
        );
        site.learn(
            ObjId(7),
            &[
                KnownItem(KnownWhat::Wreck, 160, 100),
                KnownItem(KnownWhat::Crate, 105, 100),
            ],
            |_| false,
            2,
        );
        site
    }

    /// **What a viewer knows** (doc 32 cut CF.4; Q-CF-28): on the surface, the record's things in
    /// sight of a rig now, then what the viewer remembers beyond sight — never a remembered thing
    /// the eyes cover; in a field, what the sensors hold, then the memory; another viewer, only
    /// what is in sight.
    #[test]
    fn cf4_salvage_is_what_the_eyes_see_then_what_is_remembered_beyond_them() {
        let site = record();
        let observers = [(1_u8, (100_i64, 100_i64))];
        let seen = salvage_of(&site, ObjId(7), &Eyes::Surface(&observers));
        let shown: Vec<(KnownWhat, (i64, i64), bool)> =
            seen.iter().map(|s| (s.what, s.cell, s.now)).collect();
        assert_eq!(
            shown,
            [
                (KnownWhat::Crate, (105, 100), true),
                (KnownWhat::Wreck, (160, 100), false),
            ],
            "the remembered crate is in sight, so it is drawn live, once"
        );
        assert_eq!(seen[0].words, "a crate of 2 metal_ore");
        assert_eq!(seen[1].words, "a wreck");
        assert_eq!(seen[0].glyph(), CRATE_GLYPH);
        assert_eq!(seen[1].glyph(), SALVAGE_GLYPH);
        let stranger = salvage_of(&site, ObjId(8), &Eyes::Surface(&[]));
        assert!(stranger.is_empty(), "no rig out, no memory: nothing");
        let field = salvage_of(
            &site,
            ObjId(7),
            &Eyes::Field {
                from: (150, 100),
                sensed: Vec::new(),
            },
        );
        assert!(
            field.iter().all(|s| !s.now) && field.iter().any(|s| s.cell == (105, 100)),
            "in a field only the sensors say what is there now; the wreck in sight is superseded"
        );
        assert!(field.iter().all(|s| s.cell != (160, 100)));
    }
}
