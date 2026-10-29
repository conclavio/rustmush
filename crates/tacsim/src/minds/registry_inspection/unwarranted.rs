//! **The unwarranted finding** (doc 17h cut M8d D6, D7, D8; doc 17 §10, D-UM-42): every
//! Registry inspection whose record reads an ungoverned core becomes one public canon line,
//! `minds.unwarranted`, which names nobody — not the hull, the mind, the operator or the
//! holder — and advances the Red Dies clock on the Republic's board
//! ([`mush_faction::seed_red_dies_front`], made here at world build). The same finding answers
//! the public fact `minds.unwarranted` by hull and inspection sequence
//! ([`mush_roll::commerce::facts::RegistryFindings`]), which is how an E4 condition reads it.
//!
//! **Three steps, one writer each.** The inspection's provider writes the record and nothing
//! else (M8a's A8). The `ActionStateChanged` observer, `Announce`, notes the hull in RAM
//! only — an observer of that event must not persist (the work-order index's rule,
//! `mush-industry`'s `WorkOrderIndex`). The coarse tick, `tick`, reads the hull's ring,
//! chronicles every ungoverned record past the hull's cursor, saves the cursor and commits the
//! canon lines last (canon is terminal). A restart between the record and the tick loses
//! nothing: the boot pass ([`boot`]) notes every hull the world knows, and the tick reads past
//! each cursor again. The cursor is the replay guard; the canon tail is a second one.
//!
//! **The arming half** (doc 17h §3 M8d): an `unlawful` arming line reaching a box is also
//! unwarranted. It is unreachable until the fight (no IC fire path writes one); the reader
//! that would count it is [`unlawful_on_box`], pinned by a fixture line, with no producer.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};

use mush_core::{
    Ctx, ModError, ObjId, Registrar, World, WorldEvent, WorldEventKind, WorldObserver,
};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use super::{InspectionHost, InspectionRecord};

/// The chronicle's document key.
pub const KEY: &str = "tacsim:registry_unwarranted";
/// The canon kind: the Red Dies clock's `advance_on`.
pub const KIND: &str = mush_faction::RED_DIES_KIND;
/// The public canon line. It names nothing the evidence-only rule keeps from the world.
pub const LINE: &str = "A Registry inspection found an ungoverned core.";
/// The canon source prefix: `<prefix><hull>/<seq>`, the second replay guard.
const SOURCE: &str = "tacsim.registry.unwarranted/";
/// Hulls the chronicle keeps a cursor for.
pub const MAX_HULLS: usize = 65_536;
/// Findings kept per hull, the inspection ring's size; the oldest goes first.
pub const FINDS_PER_HULL: usize = 64;
/// Hulls one tick reads.
const HULLS_PER_TICK: usize = 64;
/// The tick's name and cadence.
const TICK: &str = "registry.unwarranted";
const TICK_SECS: u64 = 1;

/// One chronicled finding: the hull's inspection sequence and the inspection's IC second.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Find {
    pub seq: u64,
    pub at_ic: i64,
}

/// One hull's chronicle: the newest inspection sequence read, and the findings kept.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HullChronicle {
    pub cursor: u64,
    pub finds: Vec<Find>,
}

/// The chronicle document: per hull, how far its inspections have been read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Chronicle {
    pub version: u8,
    pub hulls: BTreeMap<u32, HullChronicle>,
}

impl Default for Chronicle {
    fn default() -> Self {
        Self {
            version: 1,
            hulls: BTreeMap::new(),
        }
    }
}

impl Chronicle {
    /// The census: version 1, bounded, every hull's findings strictly ascending, nonzero and
    /// at or below its cursor.
    ///
    /// # Errors
    /// The first rule broken.
    pub fn check(&self) -> Result<(), String> {
        if self.version != 1 || self.hulls.len() > MAX_HULLS {
            return Err("Invalid unwarranted chronicle census".into());
        }
        for (hull, row) in &self.hulls {
            let ascending = row
                .finds
                .iter()
                .zip(row.finds.iter().skip(1))
                .all(|(a, b)| a.seq < b.seq);
            if *hull == 0
                || row.finds.len() > FINDS_PER_HULL
                || !ascending
                || row.finds.iter().any(|f| f.seq == 0 || f.seq > row.cursor)
            {
                return Err("Invalid unwarranted chronicle row".into());
            }
        }
        Ok(())
    }

    /// When `hull`'s inspection `seq` was chronicled as unwarranted, if it was.
    #[must_use]
    pub fn found(&self, hull: ObjId, seq: u64) -> Option<i64> {
        self.hulls
            .get(&hull.0)?
            .finds
            .iter()
            .find(|f| f.seq == seq)
            .map(|f| f.at_ic)
    }

    /// Fold `rows` (one hull's ring as `(seq, at_ic, unwarranted)`, any order) past the hull's
    /// cursor: every row with a newer sequence moves the cursor, and each unwarranted one is a
    /// new finding, returned in sequence order. A hull past [`MAX_HULLS`] is not admitted.
    pub fn fold(&mut self, hull: ObjId, rows: &[(u64, i64, bool)]) -> Vec<Find> {
        if !self.hulls.contains_key(&hull.0) && self.hulls.len() >= MAX_HULLS {
            tracing::warn!(
                hull = hull.0,
                "tacsim: the unwarranted chronicle is full; a hull is not chronicled"
            );
            return Vec::new();
        }
        // K7: a hull with nothing past its cursor (every hull the boot pass notes) adds no row.
        let cursor = self.hulls.get(&hull.0).map_or(0, |row| row.cursor);
        let mut fresh: Vec<(u64, i64, bool)> = rows
            .iter()
            .copied()
            .filter(|(seq, _, _)| *seq > cursor)
            .collect();
        if fresh.is_empty() {
            return Vec::new();
        }
        let row = self.hulls.entry(hull.0).or_default();
        fresh.sort_by_key(|(seq, _, _)| *seq);
        let mut found = Vec::new();
        for (seq, at_ic, unwarranted) in fresh {
            row.cursor = seq;
            if unwarranted {
                let find = Find { seq, at_ic };
                row.finds.push(find);
                found.push(find);
            }
        }
        let excess = row.finds.len().saturating_sub(FINDS_PER_HULL);
        row.finds.drain(..excess);
        found
    }
}

/// Whether an inspection record is an unwarranted finding: a fitted core read ungoverned.
#[must_use]
pub fn unwarranted(record: &InspectionRecord) -> bool {
    record.core.is_some() && record.governed == Some(false)
}

/// **What the chronicle counts** (D6, fix round K8): an inspection that read an ungoverned
/// core, or a box line recording an unlawful arming or fire (`unlawful_on_box`). The chronicle's
/// read passes the hull's box lines; no IC path journals one until the fight, so it passes none.
#[must_use]
pub fn finding(ungoverned: bool, lines: &[crate::arena::ledger::EventKind]) -> bool {
    ungoverned || unlawful_on_box(lines)
}

/// **The arming half's reader**: whether any journalled line records an unlawful arming or
/// an unlawful cited fire (M8b's `Armed` and `Fired` with `unlawful`). The IC black box has no
/// fire line until the fight, and the arena fields only governed cores (D-UM-41), so this is
/// unreachable in play; the fixture line pins that such a line would count.
#[must_use]
pub fn unlawful_on_box(lines: &[crate::arena::ledger::EventKind]) -> bool {
    lines.iter().any(|line| {
        matches!(
            line,
            crate::arena::ledger::EventKind::Armed { unlawful: true, .. }
                | crate::arena::ledger::EventKind::Fired { unlawful: true, .. }
        )
    })
}

/// The chronicle in the world's slot.
#[must_use]
pub fn current(w: &World) -> Option<&Chronicle> {
    w.module_state::<crate::TacsimStore>()?
        .registry_unwarranted
        .as_ref()
}

/// The chronicle to fold into: the stored one, a fresh one when no key was ever written, and
/// none when a key was read and could not be (it bars the chronicle until staff repair it).
fn held(w: &World) -> Option<Chronicle> {
    let store = w.module_state::<crate::TacsimStore>()?;
    match (&store.registry_unwarranted, store.registry_unwarranted_held) {
        (Some(chronicle), _) => Some(chronicle.clone()),
        (None, false) => Some(Chronicle::default()),
        (None, true) => None,
    }
}

/// The hulls announced and not yet read (RAM, the inspection slot's).
#[derive(Clone, Default)]
pub(crate) struct Pending(Arc<Mutex<BTreeSet<ObjId>>>);

impl Pending {
    fn note(&self, hull: ObjId) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(hull);
    }
    fn take(&self, n: usize) -> Vec<ObjId> {
        let mut set = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let taken: Vec<ObjId> = set.iter().copied().take(n).collect();
        for hull in &taken {
            set.remove(hull);
        }
        taken
    }
}

/// The `ActionStateChanged` observer: a changed inspection slot notes its hull. RAM only.
struct Announce(Pending);

impl WorldObserver for Announce {
    fn on_event(&self, ctx: &mut Ctx<'_>, ev: &WorldEvent) {
        let WorldEvent::ActionStateChanged {
            owner,
            family,
            keys,
        } = ev
        else {
            return;
        };
        let Some(handle) = InspectionHost::of(ctx.w).and_then(|h| h.writer.family().ok()) else {
            return;
        };
        if owner != handle.owner() || family != handle.descriptor().id() {
            return;
        }
        for key in keys {
            if let Some(hull) = slot_hull(key) {
                self.0.note(hull);
            }
        }
    }
}

/// The hull of an inspection slot key (`hull/<8 hex>/slot/<2 hex>`).
fn slot_hull(key: &SmolStr) -> Option<ObjId> {
    let rest = key.strip_prefix("hull/")?;
    let (hex, slot) = rest.split_once("/slot/")?;
    if hex.len() != 8 || slot.len() != 2 {
        return None;
    }
    let id = u32::from_str_radix(hex, 16).ok().filter(|id| *id != 0)?;
    (format!("{id:08x}") == hex).then_some(ObjId(id))
}

/// Register the observer and the tick, beside the inspection's provider and slot.
///
/// # Errors
/// A registrar that refuses either.
pub(super) fn register(reg: &mut Registrar<'_>, pending: Pending) -> Result<(), ModError> {
    reg.subscribe(
        WorldEventKind::ActionStateChanged,
        Arc::new(Announce(pending)),
    );
    reg.tick(
        TICK,
        mush_core::TickCadence::Coarse(std::time::Duration::from_secs(TICK_SECS)),
        Arc::new(tick),
    )?;
    Ok(())
}

/// **The boot pass**: note every hull the world knows, so the first tick reads past each
/// cursor (a finding recorded just before a restart is chronicled after it), and make the Red
/// Dies front on the launch's Republic. A roster without the inspection does nothing.
///
/// # Errors
/// A Red Dies front the faction owner refuses.
pub fn boot(ctx: &mut Ctx<'_>) -> Result<(), String> {
    let Some(host) = InspectionHost::of(ctx.w) else {
        return Ok(());
    };
    let pending = host.pending.clone();
    for hull in super::facts::known_hulls(ctx.w) {
        pending.note(hull);
    }
    // Doc 17h cut M8d D3: the fine's slots, the same way.
    super::fine::boot(ctx);
    seed_red_dies(ctx)
}

/// The Red Dies front on the named launch's Republic (Q-M8-5), made once: the faction owner
/// answers a second call with the front it made.
fn seed_red_dies(ctx: &mut Ctx<'_>) -> Result<(), String> {
    if ctx
        .w
        .module_state_owner(std::any::TypeId::of::<
            crate::launch::registration::LaunchService,
        >())
        .map(|o| o.as_str())
        != Some(crate::NS)
    {
        return Ok(());
    }
    let Some(republic) = ctx
        .w
        .module_state::<crate::launch::registration::LaunchService>()
        .map(|launch| launch.0.deployment().republic)
    else {
        return Ok(());
    };
    let draft = mush_canon::CanonDraft {
        kind: "registry.red_dies".into(),
        summary: "The Red Dies: the Republic watches for ungoverned cores.".into(),
        entities: vec![],
        scope: mush_canon::CanonScope::Public,
        ic_time: ctx.w.world_ic_now().at,
        source: "tacsim.registry.red_dies".into(),
        detail: serde_json::json!({}),
    };
    mush_faction::seed_red_dies_front(ctx, republic, draft).map(|_| ())
}

/// **The tick** (GOD, coarse, 1 s): read the announced hulls' rings, chronicle what is new,
/// save the cursor, then commit one canon line per finding. Infallible: a missing slot or an
/// unreadable record leaves the hull for the next announcement or boot.
fn tick(ctx: &mut Ctx<'_>, _: &str, _: Option<mush_core::SceneId>) {
    let Some(host) = InspectionHost::of(ctx.w) else {
        return;
    };
    let pending = host.pending.clone();
    let Ok(family) = host.writer.family() else {
        return;
    };
    let hulls = pending.take(HULLS_PER_TICK);
    if hulls.is_empty() {
        return;
    }
    let Some(mut chronicle) = held(ctx.w) else {
        tracing::warn!("tacsim: the unwarranted chronicle is unreadable; nothing is chronicled");
        return;
    };
    let before = chronicle.clone();
    let mut found = Vec::new();
    for hull in hulls {
        let mut records = Vec::new();
        for key in mush_roll::commerce::registry_inspection::inspection_slots(hull) {
            match ctx.action_state(&family, &key) {
                Ok(Some(record)) if record.hull == hull => {
                    // No IC box journals an arming line until the fight: none to pass.
                    records.push((
                        record.seq,
                        record.at_ic.at,
                        finding(unwarranted(record), &[]),
                    ));
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::debug!(?error, hull = hull.0, "unwarranted chronicle: ring unread");
                    pending.note(hull);
                    records.clear();
                    break;
                }
            }
        }
        for find in chronicle.fold(hull, &records) {
            found.push((hull, find));
        }
    }
    if chronicle == before || chronicle.check().is_err() {
        return;
    }
    if let Err(error) = save(ctx, chronicle) {
        tracing::warn!(%error, "unwarranted chronicle unsaved");
        return;
    }
    for (hull, find) in found {
        commit(ctx, hull, find);
    }
}

fn save(ctx: &mut Ctx<'_>, chronicle: Chronicle) -> Result<(), String> {
    let value = serde_json::to_value(&chronicle).map_err(|e| e.to_string())?;
    if let Some(store) = ctx.w.module_state_mut::<crate::TacsimStore>() {
        store.registry_unwarranted = Some(chronicle);
    }
    crate::store::push_op(
        ctx.out,
        mush_core::ModuleWrite::JsonSet {
            key: KEY.into(),
            value,
        },
    );
    ctx.out.pin_deferred();
    Ok(())
}

/// One canon line, unless the chronicle's tail already holds this finding's.
fn commit(ctx: &mut Ctx<'_>, hull: ObjId, find: Find) {
    let source = format!("{SOURCE}{}/{}", hull.0, find.seq);
    let already = ctx
        .w
        .module_state::<mush_canon::CanonState>()
        .is_some_and(|state| {
            mush_canon::records_for_source(state, &source, mush_canon::MAX_RECORDS_PER_SOURCE)
                .iter()
                .any(|event| event.kind.as_str() == KIND)
        });
    if already {
        return;
    }
    mush_canon::commit(
        ctx,
        mush_canon::CanonDraft {
            kind: SmolStr::new_static(KIND),
            summary: LINE.into(),
            entities: vec![],
            scope: mush_canon::CanonScope::Public,
            ic_time: find.at_ic,
            source,
            detail: serde_json::json!({}),
        },
    );
}

/// The public findings' source for the `minds` fact provider (D6): a seizure, never yet (its
/// producer is M8d2's). The unwarranted finding is scoped (K6) and read by no condition.
pub(crate) struct Findings;

impl mush_roll::commerce::facts::RegistryFindings for Findings {
    fn seized(&self, _: &World, _: ObjId, _: u64) -> Option<i64> {
        None
    }
}

#[cfg(test)]
mod tests;
