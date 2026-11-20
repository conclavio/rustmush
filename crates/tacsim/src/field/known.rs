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
use crate::surface::site_record::{GroundObject, KnownItem, KnownWhat, SiteRecord};

/// **An object's height for the eye**, centimetres: a crate or a heap of remains stands this far
/// off its cell.
pub const OBJECT_HEIGHT_CM: i32 = 150;
/// **The eye at demotion's look**, centimetres: a hull's sensor mast, since a return carries no
/// sensor height of its own.
pub const CLOSE_EYE_CM: u32 = 300;
/// **The most eye cells one [`Learned`] row carries** (security hardening CRIT-18): keeps
/// `FieldProvider::project_offer`'s close payload bounded even for very large parties.
pub const MAX_LEARNED_EYES: usize = 64;

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

/// Maximum cached per-site LOS results in [`Sightings`] (M36.2).
const MAX_LOS_CACHE_PER_SITE: usize = 4_096;

/// Cache key for static-terrain LOS checks in [`SiteLosCache`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct LosKey {
    from: (i64, i64),
    eye_cm: u32,
    target: (i64, i64),
}

/// Per-site cached LOS trace outcomes keyed by [`LosKey`] for one running field `task`
/// (terrain is immutable for a given `task`).
#[derive(Clone, Debug, Default)]
pub(crate) struct SiteLosCache {
    task: Option<MatchId>,
    results: BTreeMap<LosKey, bool>,
}

/// **The pulses' notes** (module state): field site → party → the object numbers its seated units
/// have held, plus a bounded per-site LOS cache to avoid redundant raycasts on static objects.
#[derive(Clone, Debug, Default)]
pub(crate) struct Sightings {
    pub seen: BTreeMap<CanonicalId, BTreeMap<PartyId, BTreeSet<u32>>>,
    los: BTreeMap<CanonicalId, SiteLosCache>,
}

/// **Note what one pulse's frames hold** (the bridge's branch, every pulse of a field).
pub(crate) fn note(
    ctx: &mut Ctx<'_>,
    task: MatchId,
    frames: &[(ObjId, crate::arena::step::Frame)],
) {
    if frames.is_empty() {
        return;
    }
    let Some(site) = ctx
        .w
        .module_state::<FieldBook>()
        .and_then(|book| book.site_of_task(task).cloned())
    else {
        return;
    };
    // M36.2: borrow `&SiteRecord` without cloning the whole site record every pulse, and collect
    // only the tiny `(id, near_cell)` metadata needed to test unseen objects.
    let objects: Vec<(u32, (i64, i64), i64)> = {
        let Some(record) = super::close::site_record_ref(ctx, &site) else {
            return;
        };
        if record.objects.is_empty() {
            return;
        }
        record
            .objects
            .iter()
            .map(|object| {
                (
                    object.id,
                    object.pose.cell(),
                    i64::from(object.footprint.max(1)),
                )
            })
            .collect()
    };
    let mut held: Vec<(PartyId, u32)> = Vec::new();
    let mut cache_updates: Vec<(LosKey, bool)> = Vec::new();
    if let Some(terrain) = ctx
        .w
        .module_state::<crate::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.arena_of(task))
        .map(|arena| &arena.terrain)
    {
        let notes = ctx.w.module_state::<Sightings>();
        let site_seen = notes.and_then(|n| n.seen.get(&site));
        let site_cache = notes
            .and_then(|n| n.los.get(&site))
            .filter(|cache| cache.task == Some(task));
        for (_, frame) in frames {
            let Some((x, y)) = frame.cell else {
                continue;
            };
            let from = (i64::from(x), i64::from(y));
            let eye_cm = frame.sensor_height_cm;
            let party_seen = site_seen.and_then(|s| s.get(&frame.party));
            for &(id, (cx, cy), side) in &objects {
                if party_seen.is_some_and(|ids| ids.contains(&id)) {
                    continue;
                }
                let (x0, y0) = (cx - side / 2, cy - side / 2);
                let target = (
                    from.0.clamp(x0, x0 + side - 1),
                    from.1.clamp(y0, y0 + side - 1),
                );
                if cells_apart(from, target) > SIGHT_CELLS {
                    continue;
                }
                let key = LosKey {
                    from,
                    eye_cm,
                    target,
                };
                let visible = if let Some(cached) =
                    site_cache.and_then(|cache| cache.results.get(&key).copied())
                {
                    cached
                } else {
                    let result = clear(terrain, (from, eye_cm), target);
                    cache_updates.push((key, result));
                    result
                };
                if visible {
                    held.push((frame.party, id));
                }
            }
        }
    }
    if held.is_empty() && cache_updates.is_empty() {
        return;
    }
    if let Some(notes) = ctx.w.module_state_mut::<Sightings>() {
        if !cache_updates.is_empty() {
            let cache = notes.los.entry(site.clone()).or_default();
            if cache.task != Some(task) {
                cache.results.clear();
                cache.task = Some(task);
            }
            for (key, result) in cache_updates {
                if cache.results.len() >= MAX_LOS_CACHE_PER_SITE
                    && !cache.results.contains_key(&key)
                {
                    break;
                }
                cache.results.insert(key, result);
            }
        }
        if !held.is_empty() {
            let site_notes = notes.seen.entry(site).or_default();
            for (party, id) in held {
                site_notes.entry(party).or_default().insert(id);
            }
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

    /// Enforce canonical bounds on `items` and `eyes` (deduped, sorted, capped).
    pub fn clamp(&mut self) {
        self.items
            .truncate(crate::surface::site_record::MAX_KNOWN_ITEMS);
        self.eyes.sort_unstable();
        self.eyes.dedup();
        self.eyes.truncate(MAX_LEARNED_EYES);
    }
}

/// **The principals of a field's party**: its leader, the pilots who sat in its units at
/// admission, and the operators of its rigs.
fn principals(ctx: &Ctx<'_>, live: &super::FieldLive, party: PartyId) -> BTreeSet<ObjId> {
    let handles = ctx
        .w
        .module_state::<crate::port::families::PortHandles>()
        .cloned();
    party_principals(live, party, |unit| {
        let handles = handles.as_ref()?;
        ctx.action_state(
            &handles.families.excursions,
            &crate::surface::excursion::RigKey(unit),
        )
        .ok()
        .flatten()
        .and_then(|record| record.excursion.as_ref().map(|out| out.operator))
    })
}

/// **[`principals`] over any reader of a rig's operator** (doc 32 §17, G-7): the leader of
/// `party`, the pilots who sat in its units at admission, and `operator_of` each of its units.
pub(crate) fn party_principals(
    live: &super::FieldLive,
    party: PartyId,
    operator_of: impl Fn(ObjId) -> Option<ObjId>,
) -> BTreeSet<ObjId> {
    let mut out: BTreeSet<ObjId> = live
        .parties
        .iter()
        .filter(|row| row.party == party)
        .filter_map(|row| row.leader)
        .collect();
    for unit in live.units.iter().filter(|row| row.party == party) {
        if let Some((pilot, _)) = &unit.pilot {
            out.insert(*pilot);
        }
        if let Some(operator) = operator_of(unit.unit) {
            out.insert(operator);
        }
    }
    out
}

/// **Whose charts `viewer` sees on the field map of `site`** (doc 32 §17, G-7; Q-CF-48,
/// Q-CF-49): while a field runs there and `viewer` sits in one of its units, every principal of
/// that unit's party ([`party_principals`]) other than `viewer`, for display only. Empty when no
/// field runs there or `viewer` sits in none of its units. A chart stays per person (doc 20 §5):
/// nothing is written to anyone's row, and the party's readings leave the map with the field.
pub(crate) fn chart_mates(
    w: &mush_core::World,
    site: &CanonicalId,
    viewer: ObjId,
    operator_of: impl Fn(ObjId) -> Option<ObjId>,
) -> BTreeSet<ObjId> {
    let Some(hosted) = w
        .module_state::<FieldBook>()
        .and_then(|book| book.hosted(site))
    else {
        return BTreeSet::new();
    };
    let live = &hosted.live;
    let Some(party) = live
        .units
        .iter()
        .find(|row| {
            row.pilot
                .as_ref()
                .is_some_and(|(pilot, _)| *pilot == viewer)
        })
        .map(|row| row.party)
    else {
        return BTreeSet::new();
    };
    let mut mates = party_principals(live, party, operator_of);
    mates.remove(&viewer);
    mates
}

/// **What each principal of the field on `site` learns as it closes** (Q-CF-28): per party, the
/// objects its seated units held over the field ([`note`]) and what every one of its units sees
/// from its last pose — objects, and the casualties, wrecks and remains the demotion leaves
/// (its own included: a party knows where its own fell; doc 36 cut GC.3 adds the remains). Read
/// before the host forgets the field.
pub(crate) fn learned_at_close(
    ctx: &Ctx<'_>,
    task: MatchId,
    site: &CanonicalId,
    returns: &[UnitReturn],
    demoted: &super::close::Demoted,
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
        .and_then(|notes| notes.seen.get(site).cloned())
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
        let fallen = demoted
            .left
            .iter()
            .map(|unit| (unit.unit, unit.pose, KnownWhat::of_unit(unit.condition)))
            .chain(
                demoted
                    .remains
                    .iter()
                    .map(|remains| (remains.unit, remains.pose, KnownWhat::Remains)),
            );
        for (unit, pose, what) in fallen {
            let cell = pose.cell();
            let own = live.party_of(unit) == Some(party);
            let seen_now = terrain.is_some_and(|terrain| {
                eyes.iter()
                    .any(|eye| sees(terrain, (*eye, CLOSE_EYE_CM), cell))
            });
            if own || seen_now {
                push(KnownItem(what, cell.0, cell.1));
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
            row.clamp();
            row
        })
        .collect()
}

/// Forget the field on `site`'s notes and cached LOS entries (it closed).
pub(crate) fn forget(ctx: &mut Ctx<'_>, site: &CanonicalId) {
    if let Some(notes) = ctx.w.module_state_mut::<Sightings>() {
        notes.seen.remove(site);
        notes.los.remove(site);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn m36_sightings_forget_clears_both_seen_objects_and_los_cache() {
        let site = CanonicalId::new("ahuna_flank").expect("site");
        let other = CanonicalId::new("urvara_reach").expect("other");
        let mut tw = mush_testkit::TestWorld::with_modules(vec![Box::new(
            crate::TacsimRuleset::with_config(crate::TacsimConfig {
                enabled: true,
                ..crate::TacsimConfig::default()
            }),
        )]);
        {
            let sightings = tw
                .world_mut()
                .module_state_mut::<Sightings>()
                .expect("sightings");
            sightings
                .seen
                .entry(site.clone())
                .or_default()
                .entry(PartyId::from_letter('A').expect("party"))
                .or_default()
                .insert(7);
            sightings
                .los
                .entry(site.clone())
                .or_default()
                .results
                .insert(
                    LosKey {
                        from: (1, 1),
                        eye_cm: 300,
                        target: (2, 2),
                    },
                    true,
                );
            sightings
                .los
                .entry(other.clone())
                .or_default()
                .results
                .insert(
                    LosKey {
                        from: (3, 3),
                        eye_cm: 300,
                        target: (4, 4),
                    },
                    false,
                );
        }
        let mut out = mush_core::Outbox::new();
        {
            let engine = tw.engine_mut();
            let mut ctx = Ctx {
                w: &mut engine.world,
                actor: mush_core::GOD,
                switches: Default::default(),
                args: Default::default(),
                out: &mut out,
                reg: &engine.registry,
                module: Some(crate::NS.into()),
                typed_state: mush_core::ModuleActionState::unavailable(),
            };
            forget(&mut ctx, &site);
        }
        let sightings = tw.world().module_state::<Sightings>().expect("sightings");
        assert!(!sightings.seen.contains_key(&site));
        assert!(!sightings.los.contains_key(&site));
        assert!(sightings.los.contains_key(&other));
    }
}
