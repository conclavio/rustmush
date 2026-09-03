//! Faction **simulation** (design: `docs/scenes_factions_knowledge.md` §3.6):
//! fronts and clocks — the deterministic world pressure that gives factions
//! motion the LLM only *colors*, never authors.
//!
//! - A **front** is an active threat or opportunity a faction pursues.
//! - A **clock** is deterministic progress toward a front's consequence. It
//!   advances **only from a typed cause** — a staff action, or a matching
//!   **canon event** (the [`CanonRecorded`](mush_core::WorldEvent::CanonRecorded)
//!   seam: a scene concluding involving the faction's members). This is the
//!   "missing middle" every historical +influence system lacked: a deterministic
//!   path from player action → territorial consequence.
//! - When a clock fills, it emits a **canon consequence** and may flip a
//!   [`ControlRecord`](crate::data::ControlRecord) — the anarch gang seizes the
//!   waterfront; a Camarilla clock can later take it back.
//!
//! Deferred (noted): a per-faction offscreen-action **pressure budget**,
//! scheduled (timer) clock ticks, rumor seeds (need the belief system), and
//! per-cause dedup beyond the single-fire observer.

use std::collections::BTreeMap;

use mush_core::module::{ModError, ModuleCtx};
use mush_core::{Deferred, ModuleOp, ModuleWrite, Outbox};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::data::{ControlKind, FactionId, NS, RegionId, Visibility};

/// A front id (durable, monotonic per world).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FrontId(pub u64);

impl std::fmt::Display for FrontId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A clock id (durable, monotonic per world).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ClockId(pub u64);

impl std::fmt::Display for ClockId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A front: an active threat or opportunity a faction pursues.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Front {
    /// Durable id.
    pub id: FrontId,
    /// The faction driving it.
    pub faction: FactionId,
    /// Human title ("Seize the waterfront").
    pub title: String,
    /// Whether the front has resolved (all its clocks done, or staff-closed).
    #[serde(default)]
    pub resolved: bool,
}

/// A clock: deterministic progress toward a front's consequence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Clock {
    /// Durable id.
    pub id: ClockId,
    /// The front it belongs to.
    pub front: FrontId,
    /// The faction driving it (denormalized for the relevance check).
    pub faction: FactionId,
    /// Human title ("Waterfront falls").
    pub title: String,
    /// Total segments.
    pub segments: u8,
    /// Filled segments.
    pub filled: u8,
    /// Who may see the clock's progress.
    #[serde(default)]
    pub visibility: Visibility,
    /// A canon **kind** that auto-advances this clock when recorded involving
    /// the faction's members (e.g. `"scene.concluded"`). `None` = staff-only
    /// advancement.
    #[serde(default)]
    pub advance_on: Option<SmolStr>,
    /// The canon summary recorded when the clock fills (the consequence).
    #[serde(default)]
    pub consequence: String,
    /// An optional territory flip on completion: the faction takes `region`
    /// with the given control kind.
    #[serde(default)]
    pub on_complete_control: Option<(RegionId, ControlKind)>,
    /// Whether the clock has completed (fired its consequence).
    #[serde(default)]
    pub completed: bool,
}

impl Clock {
    /// Advance by `n` segments (saturating at `segments`). Returns whether this
    /// call **completed** the clock (crossed to full for the first time).
    pub fn advance(&mut self, n: u8) -> bool {
        if self.completed {
            return false;
        }
        self.filled = self.filled.saturating_add(n).min(self.segments);
        if self.filled >= self.segments {
            self.completed = true;
            return true;
        }
        false
    }

    /// A compact progress string, `"3/4"`.
    #[must_use]
    pub fn progress(&self) -> String {
        format!("{}/{}", self.filled, self.segments)
    }
}

/// The live simulation state (fronts + clocks), a second module state slot
/// alongside [`FactionState`](crate::data::FactionState).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SimState {
    /// Every front by id.
    pub fronts: BTreeMap<FrontId, Front>,
    /// Every clock by id.
    pub clocks: BTreeMap<ClockId, Clock>,
    /// Front id allocator resume point.
    pub next_front_id: u64,
    /// Clock id allocator resume point.
    pub next_clock_id: u64,
}

impl SimState {
    /// Allocate the next front id.
    pub fn alloc_front_id(&mut self) -> FrontId {
        let id = FrontId(self.next_front_id);
        self.next_front_id += 1;
        id
    }

    /// Allocate the next clock id.
    pub fn alloc_clock_id(&mut self) -> ClockId {
        let id = ClockId(self.next_clock_id);
        self.next_clock_id += 1;
        id
    }

    /// The non-completed clocks that watch canon `kind`, for auto-advancement.
    #[must_use]
    pub fn clocks_watching(&self, kind: &str) -> Vec<ClockId> {
        self.clocks
            .values()
            .filter(|c| {
                !c.completed
                    && c.advance_on.as_deref() == Some(kind)
                    // A resolved (staff-closed) front freezes its clocks: they
                    // must not advance or fire a consequence afterward.
                    && self.fronts.get(&c.front).is_none_or(|f| !f.resolved)
            })
            .map(|c| c.id)
            .collect()
    }
}

// ----------------------------------------------------------------------
// Persistence
// ----------------------------------------------------------------------

/// The kv key of a front document.
#[must_use]
pub fn front_key(id: FrontId) -> SmolStr {
    SmolStr::new(format!("front:{}", id.0))
}

/// The kv key of a clock document.
#[must_use]
pub fn clock_key(id: ClockId) -> SmolStr {
    SmolStr::new(format!("clock:{}", id.0))
}

/// The kv key of the sim meta doc (the front/clock id allocators).
pub const SIM_META_KEY: &str = "simmeta";

fn push_op(out: &mut Outbox, op: ModuleWrite) {
    out.defer(Deferred::Persist(ModuleOp {
        ns: SmolStr::new_static(NS),
        op,
    }));
}

pub(crate) fn persist_front(out: &mut Outbox, f: &Front) {
    let value = serde_json::to_value(f).expect("front doc serializes");
    push_op(
        out,
        ModuleWrite::JsonSet {
            key: front_key(f.id),
            value,
        },
    );
}

pub(crate) fn persist_clock(out: &mut Outbox, c: &Clock) {
    let value = serde_json::to_value(c).expect("clock doc serializes");
    push_op(
        out,
        ModuleWrite::JsonSet {
            key: clock_key(c.id),
            value,
        },
    );
}

pub(crate) fn persist_sim_meta(out: &mut Outbox, st: &SimState) {
    push_op(
        out,
        ModuleWrite::JsonSet {
            key: SmolStr::new_static(SIM_META_KEY),
            value: serde_json::json!({
                "next_front_id": st.next_front_id,
                "next_clock_id": st.next_clock_id,
            }),
        },
    );
}

pub(crate) fn persist_del(out: &mut Outbox, key: SmolStr) {
    push_op(out, ModuleWrite::Del { key });
}

/// Remove every front and clock belonging to a destroyed `faction`, in memory,
/// and return the doc keys to delete from the store. Without this a destroyed
/// faction strands its fronts/clocks: they render as `#id`/`"?"` and their
/// control flips silently no-op, persisting across reboots (sim boot has no
/// orphan pass). Call from the faction-destroy cascade.
#[must_use]
pub(crate) fn prune_faction(st: &mut SimState, faction: FactionId) -> Vec<SmolStr> {
    let front_ids: Vec<FrontId> = st
        .fronts
        .values()
        .filter(|f| f.faction == faction)
        .map(|f| f.id)
        .collect();
    let clock_ids: Vec<ClockId> = st
        .clocks
        .values()
        .filter(|c| c.faction == faction || front_ids.contains(&c.front))
        .map(|c| c.id)
        .collect();
    let mut dels = Vec::with_capacity(front_ids.len() + clock_ids.len());
    for id in clock_ids {
        st.clocks.remove(&id);
        dels.push(clock_key(id));
    }
    for id in front_ids {
        st.fronts.remove(&id);
        dels.push(front_key(id));
    }
    dels
}

/// Clear the `on_complete_control` flip of any clock targeting a destroyed
/// `region` (the flip would otherwise silently no-op forever). Returns the
/// changed clocks to persist. Call from the region-destroy cascade.
#[must_use]
pub(crate) fn detach_region(st: &mut SimState, region: RegionId) -> Vec<Clock> {
    let mut changed = Vec::new();
    for c in st.clocks.values_mut() {
        if c.on_complete_control.map(|(r, _)| r) == Some(region) {
            c.on_complete_control = None;
            changed.push(c.clone());
        }
    }
    changed
}

/// Rebuild [`SimState`] from the simulation boot docs (`front:*`, `clock:*`,
/// `simmeta`). Individual malformed front/clock docs are skipped with a warning
/// (leaves); the id allocators resume past the meta value and any loaded id.
pub(crate) fn load_boot(
    ctx: &mut ModuleCtx<'_>,
    kv: Vec<(SmolStr, serde_json::Value)>,
) -> Result<(), ModError> {
    let mut fronts: BTreeMap<FrontId, Front> = BTreeMap::new();
    let mut clocks: BTreeMap<ClockId, Clock> = BTreeMap::new();
    let mut next_front: Option<u64> = None;
    let mut next_clock: Option<u64> = None;

    for (key, value) in kv {
        if key == SIM_META_KEY {
            next_front = value
                .get("next_front_id")
                .and_then(serde_json::Value::as_u64);
            next_clock = value
                .get("next_clock_id")
                .and_then(serde_json::Value::as_u64);
        } else if key.starts_with("front:") {
            match serde_json::from_value::<Front>(value) {
                Ok(f) => {
                    fronts.insert(f.id, f);
                }
                Err(e) => tracing::error!(%key, error = %e, "faction: bad front doc skipped"),
            }
        } else if key.starts_with("clock:") {
            match serde_json::from_value::<Clock>(value) {
                Ok(c) => {
                    clocks.insert(c.id, c);
                }
                Err(e) => tracing::error!(%key, error = %e, "faction: bad clock doc skipped"),
            }
        } else {
            tracing::warn!(%key, "faction: unknown sim key ignored");
        }
    }

    let next_front_id = next_front
        .unwrap_or(0)
        .max(fronts.keys().last().map_or(0, |i| i.0 + 1));
    let next_clock_id = next_clock
        .unwrap_or(0)
        .max(clocks.keys().last().map_or(0, |i| i.0 + 1));

    *ctx.w
        .module_state_mut::<SimState>()
        .expect("sim state slot installed at registration") = SimState {
        fronts,
        clocks,
        next_front_id,
        next_clock_id,
    };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock(segments: u8, filled: u8) -> Clock {
        Clock {
            id: ClockId(0),
            front: FrontId(0),
            faction: FactionId(0),
            title: "T".into(),
            segments,
            filled,
            visibility: Visibility::Public,
            advance_on: Some(SmolStr::new("scene.concluded")),
            consequence: "the waterfront falls".into(),
            on_complete_control: None,
            completed: false,
        }
    }

    #[test]
    fn advance_reports_completion_once() {
        let mut c = clock(4, 2);
        assert!(!c.advance(1)); // 3/4
        assert_eq!(c.progress(), "3/4");
        assert!(c.advance(1)); // 4/4 — completes now
        assert!(c.completed);
        // A further advance never re-completes.
        assert!(!c.advance(1));
        assert_eq!(c.filled, 4);
    }

    #[test]
    fn advance_saturates_and_can_overshoot_to_complete() {
        let mut c = clock(3, 0);
        assert!(c.advance(10)); // saturates to 3/3, completes
        assert_eq!(c.filled, 3);
    }

    #[test]
    fn clocks_watching_filters_kind_and_completion() {
        let mut st = SimState::default();
        let mut c1 = clock(4, 0);
        c1.id = st.alloc_clock_id();
        let mut c2 = clock(4, 4);
        c2.id = st.alloc_clock_id();
        c2.completed = true; // completed — never re-advanced
        let mut c3 = clock(4, 0);
        c3.id = st.alloc_clock_id();
        c3.advance_on = Some(SmolStr::new("faction.control")); // different kind
        for c in [c1.clone(), c2, c3] {
            st.clocks.insert(c.id, c);
        }
        assert_eq!(st.clocks_watching("scene.concluded"), vec![c1.id]);
        assert!(st.clocks_watching("nothing").is_empty());

        // A clock whose owning front is RESOLVED is frozen: never watched.
        st.fronts.insert(
            FrontId(7),
            Front {
                id: FrontId(7),
                faction: FactionId(0),
                title: "closed".into(),
                resolved: true,
            },
        );
        let mut c4 = clock(4, 0);
        c4.id = st.alloc_clock_id();
        c4.front = FrontId(7);
        let c4id = c4.id;
        st.clocks.insert(c4id, c4);
        assert!(
            !st.clocks_watching("scene.concluded").contains(&c4id),
            "resolved front freezes its clock"
        );
        assert_eq!(
            st.clocks_watching("scene.concluded"),
            vec![c1.id],
            "only the live front's clock advances"
        );
    }
}
