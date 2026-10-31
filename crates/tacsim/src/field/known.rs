//! **What a field's parties see lying on its ground, and what they remember of it** (doc 32 cut
//! CF.4; Q-CF-26, Q-CF-28).
//!
//! **Objects are sensor returns** (Q-CF-26): a remains or a crate on the site record is seen by a
//! unit in line of sight within [`SIGHT_CELLS`] of it ([`sensed`]) — the short visual range the
//! presence picture already names a hull's class at — over the field's own ground, with the eye at
//! the unit's sensor height and the object [`OBJECT_HEIGHT_CM`] tall. The trace is the world's
//! (`mush_atlas::trace_cells`), never the kernel's perception: an object has no `ContactId`, no
//! party and no lock, it is not in the mind API, and nothing about it is in the snapshot or the
//! hash. The cockpit frame carries the objects a unit holds in its `ground` rows
//! (`crate::port::site_map::picture`).
//!
//! **A party remembers what it saw** (Q-CF-28): every pulse the host notes which objects each
//! party's seated units hold ([`note`], RAM, per field); at demotion each party also looks once
//! from every one of its units' last poses, at the objects and at the casualties and wrecks the
//! field leaves ([`learned_at_close`]). The `close` writes it to the site record for each of the
//! party's principals — its leader, the pilots who sat in its units, the operators of its rigs —
//! as [`crate::surface::site_record::SiteRecord::learn`], which also forgets a remembered thing
//! the party looked at again and did not find. A restart mid-field loses the pulses' notes before
//! it (RAM); the look at demotion still runs.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{CanonicalId, Ctx, ObjId};
use serde::{Deserialize, Serialize};

use super::FieldBook;
use crate::arena::PartyId;
use crate::arena::damage::UnitReturn;
use crate::arena::state::ArenaTerrain;
use crate::arena_task::MatchId;
use crate::surface::excursion::cells_apart;
use crate::surface::presence::SIGHT_CELLS;
use crate::surface::site_record::{GroundObject, KnownItem, KnownWhat, LeftUnit, SiteRecord};

/// **An object's height for the eye**, centimetres: a crate or a heap of remains stands this far
/// off its cell.
pub const OBJECT_HEIGHT_CM: i32 = 150;
/// **The eye at demotion's look**, centimetres: a hull's sensor mast, since a return carries no
/// sensor height of its own.
pub const CLOSE_EYE_CM: u32 = 300;

/// Whether the eye at `from` (`eye_cm` up) sees a thing `height_cm` tall at `to`.
fn clear(terrain: &ArenaTerrain, (from, eye_cm): ((i64, i64), u32), to: (i64, i64)) -> bool {
    if from == to {
        return true;
    }
    let (Ok(fx), Ok(fy), Ok(tx), Ok(ty)) = (
        u16::try_from(from.0),
        u16::try_from(from.1),
        u16::try_from(to.0),
        u16::try_from(to.1),
    ) else {
        return false;
    };
    mush_atlas::trace_cells(
        terrain,
        mush_atlas::Eye {
            cell: (fx, fy),
            height_cm: i32::try_from(eye_cm).unwrap_or(i32::MAX),
        },
        mush_atlas::Eye {
            cell: (tx, ty),
            height_cm: OBJECT_HEIGHT_CM,
        },
    )
    .sight
    .is_clear_or_partial()
}

/// Whether the eye sees `to` at all: within [`SIGHT_CELLS`] and in line of sight.
fn sees(terrain: &ArenaTerrain, eye: ((i64, i64), u32), to: (i64, i64)) -> bool {
    cells_apart(eye.0, to) <= SIGHT_CELLS && clear(terrain, eye, to)
}

/// **The cell of `object`'s footprint nearest `from`** — what an eye looks at: the footprint is
/// opaque on the field's ground (`crate::field::ground`), so its far cells hide behind its near
/// ones.
fn near_cell(object: &GroundObject, from: (i64, i64)) -> (i64, i64) {
    let (cx, cy) = object.pose.cell();
    let side = i64::from(object.footprint.max(1));
    let (x0, y0) = (cx - side / 2, cy - side / 2);
    (
        from.0.clamp(x0, x0 + side - 1),
        from.1.clamp(y0, y0 + side - 1),
    )
}

/// Whether the eye sees `object`.
fn sees_object(terrain: &ArenaTerrain, eye: ((i64, i64), u32), object: &GroundObject) -> bool {
    sees(terrain, eye, near_cell(object, eye.0))
}

/// **The record's objects a unit at `from` holds** (Q-CF-26), in the record's order.
#[must_use]
pub(crate) fn sensed<'r>(
    terrain: &ArenaTerrain,
    eye: ((i64, i64), u32),
    record: &'r SiteRecord,
) -> Vec<&'r GroundObject> {
    record
        .objects
        .iter()
        .filter(|object| sees_object(terrain, eye, object))
        .collect()
}

/// **The pulses' notes** (module state): field site → party → the object numbers its seated units
/// have held.
#[derive(Clone, Debug, Default)]
pub(crate) struct Sightings(pub BTreeMap<CanonicalId, BTreeMap<PartyId, BTreeSet<u32>>>);

/// **Note what one pulse's frames hold** (the bridge's branch, every pulse of a field).
pub(crate) fn note(
    ctx: &mut Ctx<'_>,
    task: MatchId,
    frames: &[(ObjId, crate::arena::step::Frame)],
) {
    let Some(site) = ctx
        .w
        .module_state::<FieldBook>()
        .and_then(|book| book.site_of_task(task).cloned())
    else {
        return;
    };
    let Some(record) = super::close::site_record(ctx, &site).filter(|r| !r.objects.is_empty())
    else {
        return;
    };
    let mut held: Vec<(PartyId, u32)> = Vec::new();
    if let Some(terrain) = ctx
        .w
        .module_state::<crate::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.arena_of(task))
        .map(|arena| &arena.terrain)
    {
        for (_, frame) in frames {
            let Some((x, y)) = frame.cell else {
                continue;
            };
            let eye = ((i64::from(x), i64::from(y)), frame.sensor_height_cm);
            held.extend(
                sensed(terrain, eye, &record)
                    .into_iter()
                    .map(|object| (frame.party, object.id)),
            );
        }
    }
    if held.is_empty() {
        return;
    }
    if let Some(notes) = ctx.w.module_state_mut::<Sightings>() {
        let site_notes = notes.0.entry(site).or_default();
        for (party, id) in held {
            site_notes.entry(party).or_default().insert(id);
        }
    }
}

/// **What one principal learns as a field closes**: the things seen and the cells looked at (the
/// close's `known` rows; `swept` is every eye's cell, a thing within [`SIGHT_CELLS`] of one not
/// seen again is forgotten).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Learned {
    /// Who learns.
    pub principal: ObjId,
    /// What they saw.
    pub items: Vec<KnownItem>,
    /// Where their eyes stood at the end.
    pub eyes: Vec<(i64, i64)>,
}

impl Learned {
    /// Whether `cell` was looked at.
    #[must_use]
    pub fn swept(&self, cell: (i64, i64)) -> bool {
        self.eyes
            .iter()
            .any(|eye| cells_apart(*eye, cell) <= SIGHT_CELLS)
    }
}

/// **The principals of a field's party**: its leader, the pilots who sat in its units at
/// admission, and the operators of its rigs.
fn principals(ctx: &Ctx<'_>, live: &super::FieldLive, party: PartyId) -> BTreeSet<ObjId> {
    let mut out: BTreeSet<ObjId> = live
        .parties
        .iter()
        .filter(|row| row.party == party)
        .filter_map(|row| row.leader)
        .collect();
    let handles = ctx
        .w
        .module_state::<crate::port::families::PortHandles>()
        .cloned();
    for unit in live.units.iter().filter(|row| row.party == party) {
        if let Some((pilot, _)) = &unit.pilot {
            out.insert(*pilot);
        }
        if let Some(handles) = &handles
            && let Some(out_on) = ctx
                .action_state(
                    &handles.families.excursions,
                    &crate::surface::excursion::RigKey(unit.unit),
                )
                .ok()
                .flatten()
                .and_then(|record| record.excursion.as_ref())
        {
            out.insert(out_on.operator);
        }
    }
    out
}

/// **What each principal of the field on `site` learns as it closes** (Q-CF-28): per party, the
/// objects its seated units held over the field ([`note`]) and what every one of its units sees
/// from its last pose — objects, and the casualties and wrecks in `left` (its own included: a
/// party knows where its own fell). Read before the host forgets the field.
pub(crate) fn learned_at_close(
    ctx: &Ctx<'_>,
    task: MatchId,
    site: &CanonicalId,
    returns: &[UnitReturn],
    left: &[LeftUnit],
) -> Vec<Learned> {
    let Some(live) = ctx
        .w
        .module_state::<FieldBook>()
        .and_then(|book| book.hosted(site))
        .map(|hosted| hosted.live.clone())
    else {
        return Vec::new();
    };
    let record = super::close::site_record(ctx, site);
    let notes: BTreeMap<PartyId, BTreeSet<u32>> = ctx
        .w
        .module_state::<Sightings>()
        .and_then(|notes| notes.0.get(site).cloned())
        .unwrap_or_default();
    let terrain = ctx
        .w
        .module_state::<crate::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.arena_of(task))
        .map(|arena| &arena.terrain);
    let mut out: BTreeMap<ObjId, Learned> = BTreeMap::new();
    for party in live.parties.iter().map(|row| row.party) {
        let eyes: Vec<(i64, i64)> = returns
            .iter()
            .filter(|back| live.party_of(back.unit) == Some(party))
            .map(|back| crate::surface::excursion::site_cell(back.pose.x_cm, back.pose.y_cm))
            .collect();
        let mut items: Vec<KnownItem> = Vec::new();
        let mut push = |item: KnownItem| {
            if !items.iter().any(|kept| kept.cell() == item.cell()) {
                items.push(item);
            }
        };
        if let Some(record) = &record {
            let noted = notes.get(&party);
            for object in &record.objects {
                let cell = object.pose.cell();
                let seen_now = terrain.is_some_and(|terrain| {
                    eyes.iter()
                        .any(|eye| sees_object(terrain, (*eye, CLOSE_EYE_CM), object))
                });
                if seen_now || noted.is_some_and(|ids| ids.contains(&object.id)) {
                    push(KnownItem(
                        KnownWhat::of_object(&object.kind),
                        cell.0,
                        cell.1,
                    ));
                }
            }
        }
        for unit in left {
            let cell = unit.pose.cell();
            let own = live.party_of(unit.unit) == Some(party);
            let seen_now = terrain.is_some_and(|terrain| {
                eyes.iter()
                    .any(|eye| sees(terrain, (*eye, CLOSE_EYE_CM), cell))
            });
            if own || seen_now {
                push(KnownItem(
                    KnownWhat::of_unit(unit.condition),
                    cell.0,
                    cell.1,
                ));
            }
        }
        for principal in principals(ctx, &live, party) {
            let row = out.entry(principal).or_insert_with(|| Learned {
                principal,
                items: Vec::new(),
                eyes: Vec::new(),
            });
            for item in &items {
                if !row.items.iter().any(|kept| kept.cell() == item.cell()) {
                    row.items.push(*item);
                }
            }
            row.eyes.extend(eyes.iter().copied());
        }
    }
    out.into_values()
        .map(|mut row| {
            row.items
                .truncate(crate::surface::site_record::MAX_KNOWN_ITEMS);
            row.eyes.sort_unstable();
            row.eyes.dedup();
            row
        })
        .collect()
}

/// Forget the field on `site`'s notes (it closed).
pub(crate) fn forget(ctx: &mut Ctx<'_>, site: &CanonicalId) {
    if let Some(notes) = ctx.w.module_state_mut::<Sightings>() {
        notes.0.remove(site);
    }
}
