//! # mush-faction
//!
//! The faction **registry** (design: `docs/scenes_factions_knowledge.md` §3,
//! registry half): first-class organizations for every game line — WoD sects
//! and coteries, Shadowrun crews/corps/gangs, TacSim houses and merc
//! units — as engine data, not roster decoration.
//!
//! What the registry holds:
//!
//! - **Factions** with typed resource pools, NPC principals, and an opaque
//!   `ruleset_ext` document for ruleset-flavored mechanics.
//! - **Structural edges** (`ChapterOf` / `SubordinateTo` / `ContractedTo` /
//!   `FrontFor`) — a typed DAG per kind, never tiers, never auto-transitive.
//! - **Membership as ground truth** with a per-record [`Visibility`] policy:
//!   the engine's registry drives *authorization* (intel clearance, channel
//!   eligibility); every display surface projects it (a covert membership is
//!   invisible; a hidden faction indistinguishable from a missing one).
//! - **Territory**: abstract [`Region`]s (zones, or nothing — non-spatial
//!   influence regions) with per-kind [`ControlRecord`]s (de jure / de facto
//!   / covert / economic). Registry phase: staff-set; the faction
//!   *simulation* (fronts/clocks) later moves changes behind typed causes.
//! - **A backing [`OrgId`](mush_org::OrgId)** (E3d): a faction is a registry
//!   entry, never a principal, so anything it *owns* is owned by an organization
//!   it names ([`Faction::org`], bound by staff through `@faction/org`, resolved
//!   live by [`backing_org`]). The same cut makes the rating/money boundary
//!   explicit: `@faction/migrate` moves one `resources` key into a real journaled
//!   account, once, and records it in [`Faction::migrated`] so `@faction/resource`
//!   can never touch it again.
//! - **[`Principal`]** — the generalized M12 intel principal (`Character` or
//!   `Faction`), consumed by `mush-npc`'s informant loop: a faction's intel
//!   pool is a clearance-gated shared store, read via `+intel` by members
//!   whose level passes the faction's `intel_level`.
//!
//! ## How it hangs off the module API
//!
//! - **State** lives in one [`FactionState`] module state slot; every
//!   mutation mirrors to `mod:faction:*` module ops (docs per faction /
//!   membership / region, one control doc per region, a `meta` doc for the
//!   id allocators) — write-through with command atomicity.
//! - **Boot**: [`GameModule::on_world_loaded`] rebuilds the whole state from
//!   kv docs. Faction and region docs are containers: a corrupt one FAILS
//!   the boot (proceeding would orphan-purge live data). Membership and
//!   control docs are leaves: a corrupt one is skipped loudly.
//! - **Cleanup**: an `ObjDestroyed` observer drops a destroyed character's
//!   memberships, removes it from principal lists, and prunes region zone
//!   references.
//! - **Channels**: a faction may bind a comsys channel; **Public** members
//!   are auto-joined through `mush_comsys::api` (the owning module's public
//!   API — never by touching comsys state directly). No quieter visibility
//!   is ever auto-joined — a comsys roster is world-readable via
//!   `@channel/who` under the default open see lock, so it must not carry
//!   what the registry hides — and the module never removes a channel
//!   membership it did not create. Binding warns when the channel's see/join
//!   locks are wide open.

mod commands;
mod data;
mod econ;
mod sim;
mod sim_commands;
mod standing;

use std::collections::BTreeMap;

use mush_core::module::{
    GameModule, ModError, ModuleBootData, ModuleCtx, Registrar, WorldEvent, WorldEventKind,
};
use mush_core::{Ctx, Flags, ObjId};
use smol_str::SmolStr;

pub use data::{
    ControlKind, ControlRecord, EdgeKind, Faction, FactionEdge, FactionId, FactionMembership,
    FactionState, MAX_NAME_LEN, META_KEY, NS, Principal, Region, RegionId, ResourceChange,
    Visibility, control_key, fac_key, member_key, region_key,
};
pub use econ::{LinkError, backing_org};
pub use sim::{Clock, ClockId, Front, FrontId, SimState};
pub use standing::FactionStandingProvider;

use data::{persist_control, persist_del, persist_faction, persist_region};

/// The faction registry game module. Hand one to `Engine::with_modules`.
pub struct FactionModule;

impl FactionModule {
    /// A fresh module instance.
    #[must_use]
    pub fn new() -> FactionModule {
        FactionModule
    }
}

impl Default for FactionModule {
    fn default() -> Self {
        FactionModule::new()
    }
}

impl GameModule for FactionModule {
    fn name(&self) -> &'static str {
        "faction"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.state(FactionState::default());
        reg.state(sim::SimState::default());
        reg.subscribe(WorldEventKind::ObjDestroyed, std::sync::Arc::new(on_obj_destroyed));
        // Consequence-driven simulation: a canon event advances matching
        // faction clocks (the scene-outcome → clock loop).
        reg.subscribe(WorldEventKind::CanonRecorded, std::sync::Arc::new(on_canon_recorded));
        commands::register(reg)?;
        sim_commands::register(reg)?;
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        // Split the simulation docs (fronts/clocks) from the registry docs so
        // each boot loader sees only its own keys.
        let (sim_kv, fac_kv): (Vec<_>, Vec<_>) = data
            .kv
            .into_iter()
            .partition(|(k, _)| is_sim_key(k));
        load_boot(
            ctx,
            ModuleBootData {
                kv: fac_kv,
                streams: data.streams,
            },
        )?;
        sim::load_boot(ctx, sim_kv)
    }
}

/// Whether a namespace key belongs to the simulation (fronts/clocks), not the
/// registry.
fn is_sim_key(key: &str) -> bool {
    key == sim::SIM_META_KEY || key.starts_with("front:") || key.starts_with("clock:")
}

// ----------------------------------------------------------------------
// Simulation: consequence-driven clock advancement
// ----------------------------------------------------------------------

use mush_canon::CanonScope;

/// The canon kind a completed clock records. **Reserved**: a clock may not
/// watch this kind (`@clock/set on:` rejects it), which is what actually bounds
/// the canon→clock→canon chain to a single non-advancing follow-up — the
/// per-clock `completed` flag then guarantees termination overall.
pub(crate) const CONSEQUENCE_KIND: &str = "front.progress";

/// Map a registry [`Visibility`] to a canon [`CanonScope`] for consequence
/// events: members-or-officers-known becomes Restricted; covert becomes Secret.
fn scope_for(vis: Visibility) -> CanonScope {
    match vis {
        Visibility::Public => CanonScope::Public,
        Visibility::MembersKnow | Visibility::OfficersKnow => CanonScope::Restricted,
        Visibility::Covert => CanonScope::Secret,
    }
}

/// `CanonRecorded` observer: advance every non-completed clock that watches this
/// canon `kind` **and** whose faction has a member among the event's entities
/// (the relevance gate — a Camarilla clock advances on a scene its members were
/// in, not on every scene anywhere).
///
/// Termination: this runs synchronously inside `emit_world_event`, and a
/// completing clock records a fresh `CanonRecorded` that re-enters here. Two
/// facts bound it. (1) Depth: the consequence uses [`CONSEQUENCE_KIND`], which
/// `@clock/set on:` refuses, so no clock can watch it — the re-entry finds zero
/// watchers and returns, capping the stack at one nested call regardless of how
/// many clocks fire. (2) Overall: `clocks_watching` filters out completed
/// clocks and `Clock::advance` early-returns once completed, so each clock fires
/// at most once even if staff deliberately chain clocks by some other kind.
fn on_canon_recorded(ctx: &mut Ctx<'_>, ev: &WorldEvent) {
    let WorldEvent::CanonRecorded { kind, entities, .. } = ev else {
        return;
    };
    let to_advance: Vec<ClockId> = {
        let (Some(sim), Some(fac)) = (
            ctx.w.module_state::<SimState>(),
            ctx.w.module_state::<FactionState>(),
        ) else {
            return;
        };
        sim.clocks_watching(kind)
            .into_iter()
            .filter(|&cid| {
                sim.clocks.get(&cid).is_some_and(|c| {
                    entities.iter().any(|&e| fac.is_member(e, c.faction))
                })
            })
            .collect()
    };
    for cid in to_advance {
        advance_clock(ctx, cid, 1);
    }
}

/// Advance clock `id` by `n` segments; if it completes, fire its consequence.
/// The single path staff `@clock/advance` and the canon observer both use, so
/// completion is handled identically. Persists the clock.
pub(crate) fn advance_clock(ctx: &mut Ctx<'_>, id: ClockId, n: u8) -> bool {
    let outcome = {
        let Some(sim) = ctx.w.module_state_mut::<SimState>() else {
            return false;
        };
        let Some(clock) = sim.clocks.get_mut(&id) else {
            return false;
        };
        let completed = clock.advance(n);
        (clock.clone(), completed)
    };
    let (clock, completed) = outcome;
    sim::persist_clock(ctx.out, &clock);
    if completed {
        fire_consequence(ctx, &clock);
    }
    completed
}

/// Fire a completed clock's consequence: optionally flip a territory
/// [`ControlRecord`] to the faction, then record a durable canon event so the
/// world (and the chronicle) learns of it.
fn fire_consequence(ctx: &mut Ctx<'_>, clock: &Clock) {
    let now = ctx.w.now();
    let ic = ctx.w.game_now_unix();

    // Optional territory flip: the faction takes the region.
    if let Some((region, kind)) = clock.on_complete_control {
        let updated = {
            let Some(fac) = ctx.w.module_state_mut::<FactionState>() else {
                return;
            };
            if fac.regions.contains_key(&region) && fac.factions.contains_key(&clock.faction) {
                let recs = fac.control.entry(region).or_default();
                // Replace any existing (faction, kind) hold, then add the new one.
                recs.retain(|r| !(r.faction == clock.faction && r.kind == kind));
                recs.push(ControlRecord {
                    region,
                    faction: clock.faction,
                    kind,
                    strength: 3,
                    contested_by: Vec::new(),
                    // Inherit the clock's visibility: a covert operation that
                    // seizes a region produces a covert hold, not a world-visible
                    // one. Hardcoding Public here would announce a secret op's
                    // result on `+territory` to everyone — the exact leak this
                    // whole model exists to prevent.
                    visibility: clock.visibility,
                    since: now,
                });
                Some((region, recs.clone()))
            } else {
                None
            }
        };
        if let Some((region, recs)) = updated {
            persist_control(ctx.out, region, &recs);
        }
    }

    // The canon consequence — a distinct, reserved kind ([`CONSEQUENCE_KIND`])
    // that no clock may watch, so it cannot re-advance a scene-outcome clock.
    // Entities are exactly the members who could see the clock: officers only
    // for an OfficersKnow/Covert clock, all members otherwise. This keeps a
    // Restricted-scoped consequence from surfacing in a rank-and-file member's
    // chronicle when the operation itself was officers-only.
    let (summary, entities, scope) = {
        let Some(fac) = ctx.w.module_state::<FactionState>() else {
            return;
        };
        let fname = fac
            .factions
            .get(&clock.faction)
            .map_or_else(|| format!("#{}", clock.faction), |f| f.name.to_string());
        let summary = if clock.consequence.trim().is_empty() {
            format!("{fname}: {} — complete.", clock.title)
        } else {
            clock.consequence.clone()
        };
        let officers_only = matches!(
            clock.visibility,
            Visibility::OfficersKnow | Visibility::Covert
        );
        let entities: Vec<ObjId> = fac
            .members_of(clock.faction)
            .filter(|m| !officers_only || fac.is_officer(m.character, clock.faction))
            .map(|m| m.character)
            .collect();
        (summary, entities, scope_for(clock.visibility))
    };
    let _ = mush_canon::commit(
        ctx,
        mush_canon::CanonDraft {
            kind: SmolStr::new_static(CONSEQUENCE_KIND),
            summary,
            entities,
            scope,
            ic_time: ic,
            source: format!("clock:{}", clock.id),
            detail: serde_json::json!({ "clock": clock.id.0, "faction": clock.faction.0 }),
        },
    );
}

/// `ObjDestroyed` observer: drop the husk's memberships (and their docs),
/// remove it from principal lists, and prune region zone references.
fn on_obj_destroyed(ctx: &mut Ctx<'_>, ev: &WorldEvent) {
    let WorldEvent::ObjDestroyed { obj } = ev else {
        return;
    };
    let obj = *obj;
    let (member_keys, changed_factions, changed_regions) = {
        let Some(st) = ctx.w.module_state_mut::<FactionState>() else {
            return;
        };
        let member_keys: Vec<(ObjId, data::FactionId)> = st
            .memberships_of(obj)
            .map(|m| (m.character, m.faction))
            .collect();
        for k in &member_keys {
            st.members.remove(k);
        }
        let mut changed_factions: Vec<Faction> = Vec::new();
        for f in st.factions.values_mut() {
            if f.principals.contains(&obj) {
                f.principals.retain(|p| *p != obj);
                changed_factions.push(f.clone());
            }
        }
        let mut changed_regions: Vec<Region> = Vec::new();
        for r in st.regions.values_mut() {
            if r.zones.contains(&obj) {
                r.zones.retain(|z| *z != obj);
                changed_regions.push(r.clone());
            }
        }
        (member_keys, changed_factions, changed_regions)
    };
    for (p, f) in member_keys {
        persist_del(ctx.out, data::member_key(p, f));
    }
    for f in &changed_factions {
        persist_faction(ctx.out, f);
    }
    for r in &changed_regions {
        persist_region(ctx.out, r);
    }
}

/// Rebuild [`FactionState`] from persisted boot data.
///
/// Faction and region documents are containers — a corrupt one FAILS THE
/// BOOT (proceeding would classify that faction's memberships and control
/// records as orphans and permanently delete them; corrupt data is an
/// operational problem to fix, never a license for destructive "recovery").
/// Membership and control documents are individually skippable leaves.
/// Orphans (memberships of destroyed players or missing factions, control
/// records naming missing regions/factions, zone refs to destroyed objects)
/// are dropped and their persisted docs corrected.
fn load_boot(ctx: &mut ModuleCtx<'_>, data: ModuleBootData) -> Result<(), ModError> {
    let mut factions: BTreeMap<data::FactionId, Faction> = BTreeMap::new();
    let mut regions: BTreeMap<data::RegionId, Region> = BTreeMap::new();
    let mut raw_members: Vec<FactionMembership> = Vec::new();
    let mut raw_control: Vec<(data::RegionId, Vec<ControlRecord>)> = Vec::new();
    let mut meta_faction: Option<u64> = None;
    let mut meta_region: Option<u64> = None;

    for (key, value) in data.kv {
        if key == META_KEY {
            meta_faction = value
                .get("next_faction_id")
                .and_then(serde_json::Value::as_u64);
            meta_region = value
                .get("next_region_id")
                .and_then(serde_json::Value::as_u64);
        } else if let Some(id) = key.strip_prefix("fac:") {
            let id = id
                .parse::<u64>()
                .map_err(|_| ModError::msg(format!("faction: bad faction key {key:?}")))?;
            let f = serde_json::from_value::<Faction>(value)
                .map_err(|e| ModError::msg(format!("faction: bad faction doc {key:?}: {e}")))?;
            if f.id != data::FactionId(id) {
                return Err(ModError::msg(format!(
                    "faction: doc {key:?} claims id {}",
                    f.id
                )));
            }
            factions.insert(f.id, f);
        } else if let Some(id) = key.strip_prefix("region:") {
            let id = id
                .parse::<u64>()
                .map_err(|_| ModError::msg(format!("faction: bad region key {key:?}")))?;
            let r = serde_json::from_value::<Region>(value)
                .map_err(|e| ModError::msg(format!("faction: bad region doc {key:?}: {e}")))?;
            if r.id != data::RegionId(id) {
                return Err(ModError::msg(format!(
                    "faction: region doc {key:?} claims id {}",
                    r.id
                )));
            }
            regions.insert(r.id, r);
        } else if key.starts_with("member:") {
            match serde_json::from_value::<FactionMembership>(value) {
                Ok(m) => raw_members.push(m),
                Err(e) => {
                    tracing::error!(%key, error = %e, "faction: bad membership doc skipped");
                }
            }
        } else if let Some(id) = key.strip_prefix("control:") {
            match (
                id.parse::<u64>(),
                serde_json::from_value::<Vec<ControlRecord>>(value),
            ) {
                (Ok(region), Ok(recs)) => raw_control.push((data::RegionId(region), recs)),
                (Err(e), _) => {
                    tracing::error!(%key, error = %e, "faction: bad control key skipped");
                }
                (_, Err(e)) => {
                    tracing::error!(%key, error = %e, "faction: unparseable control doc skipped");
                }
            }
        } else {
            tracing::warn!(%key, "faction: unknown persisted key ignored");
        }
    }

    // Membership orphan pass: the player must exist (not GOING) and the
    // faction must exist.
    let mut members: BTreeMap<(ObjId, data::FactionId), FactionMembership> = BTreeMap::new();
    let mut orphan_keys: Vec<smol_str::SmolStr> = Vec::new();
    for m in raw_members {
        let live = ctx
            .w
            .get(m.character)
            .is_some_and(|o| !o.flags.contains(Flags::GOING));
        if live && factions.contains_key(&m.faction) {
            members.insert((m.character, m.faction), m);
        } else {
            orphan_keys.push(data::member_key(m.character, m.faction));
        }
    }

    // Control orphan pass: region + holder faction must exist; contested_by
    // prunes to live factions. Changed vecs are re-persisted.
    let mut control: BTreeMap<data::RegionId, Vec<ControlRecord>> = BTreeMap::new();
    let mut fix_control: Vec<data::RegionId> = Vec::new();
    for (region, recs) in raw_control {
        if !regions.contains_key(&region) {
            orphan_keys.push(data::control_key(region));
            continue;
        }
        let before = recs.len();
        let mut kept: Vec<ControlRecord> = recs
            .into_iter()
            .filter(|r| factions.contains_key(&r.faction) && r.region == region)
            .collect();
        let mut pruned_contest = false;
        for r in &mut kept {
            let b = r.contested_by.len();
            r.contested_by.retain(|c| factions.contains_key(c));
            pruned_contest |= r.contested_by.len() != b;
        }
        if kept.is_empty() {
            orphan_keys.push(data::control_key(region));
            continue;
        }
        if kept.len() != before || pruned_contest {
            fix_control.push(region);
        }
        control.insert(region, kept);
    }

    // Region zone refs to destroyed objects prune (and re-persist).
    let mut fix_regions: Vec<data::RegionId> = Vec::new();
    for r in regions.values_mut() {
        let before = r.zones.len();
        r.zones.retain(|z| {
            ctx.w
                .get(*z)
                .is_some_and(|o| !o.flags.contains(Flags::GOING))
        });
        if r.zones.len() != before {
            fix_regions.push(r.id);
        }
    }
    // Principal refs to destroyed NPCs prune likewise.
    let mut fix_factions: Vec<data::FactionId> = Vec::new();
    for f in factions.values_mut() {
        let before = f.principals.len();
        f.principals.retain(|p| {
            ctx.w
                .get(*p)
                .is_some_and(|o| !o.flags.contains(Flags::GOING))
        });
        if f.principals.len() != before {
            fix_factions.push(f.id);
        }
    }

    // Allocator resume points: the meta doc's value, but never below the ids
    // actually loaded — an older/partially restored meta doc must not make
    // alloc hand out an id that is live (silently overwriting a faction whose
    // memberships and control records would then authorize the impostor).
    let loaded_next_faction = factions.keys().last().map_or(0, |id| id.0 + 1);
    let loaded_next_region = regions.keys().last().map_or(0, |id| id.0 + 1);
    let next_faction_id = meta_faction.unwrap_or(0).max(loaded_next_faction);
    let next_region_id = meta_region.unwrap_or(0).max(loaded_next_region);

    let st = FactionState {
        factions,
        members,
        regions,
        control,
        next_faction_id,
        next_region_id,
    };
    for key in orphan_keys {
        persist_del(ctx.out, key);
    }
    for id in fix_control {
        let recs = st.control.get(&id).cloned().unwrap_or_default();
        persist_control(ctx.out, id, &recs);
    }
    for id in fix_regions {
        if let Some(r) = st.regions.get(&id) {
            persist_region(ctx.out, r);
        }
    }
    for id in fix_factions {
        if let Some(f) = st.factions.get(&id) {
            persist_faction(ctx.out, f);
        }
    }
    *ctx.w
        .module_state_mut::<FactionState>()
        .expect("faction state slot installed at registration") = st;
    Ok(())
}
