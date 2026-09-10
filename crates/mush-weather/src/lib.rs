//! # mush-weather
//!
//! The place-scoped weather module (design doc 04 §10, PLAN's Weather
//! bullet; keyed on the atlas since S2b, doc 14 D-14-28), built as a
//! [`GameModule`] on mush-core's merged module API. The model is a
//! **deterministic** per-place Markov state machine driven by
//! climate data files; there is no LLM flavor here (that is M10 — the line
//! pools ship hand-written and leave a documented top-up seam, see
//! [`climate`]).
//!
//! ## How it hangs off the module API
//!
//! - **State** lives in one [`WeatherState`] module state slot (the live
//!   [`WeatherCell`]s + a deterministic [`WxRng`]). Every cell mutation
//!   mirrors to a `mod:weather:cell:<place id>` document via [`mush_core::ModuleOp`],
//!   so persistence is write-through with tick/command atomicity;
//!   [`GameModule::on_world_loaded`] rebuilds the cells at boot. The
//!   immutable [`ClimateSet`] is shared (`Arc`) across the tick, hooks,
//!   commands, and the query provider — it is not part of the cloned state.
//! - **Ticks**: one [`TickCadence::Coarse`] tick (default every 300 s of OOC
//!   wall time — chosen over "15 IC minutes" for simplicity; configurable via
//!   [`WeatherConfig`]). Each firing rolls every unpinned cell's transition
//!   against the current season / time-of-day (read from the core game
//!   clock) and emits transition + ambient lines.
//! - **Room opt-in via flags** (already in mush-core): a room is `OUTDOORS`
//!   (full weather), `SHELTERED` (state changes, muffled, no ambient drip),
//!   or `INDOORS` (neither flag — silent). A room's weather is its
//!   **neighborhood's** cell, else its **region's** (S2b); a room off the atlas
//!   bears none — zones keep locks and zone exits only.
//! - **Messaging**: transition broadcasts to a place's `OUTDOORS`/`SHELTERED`
//!   rooms; low-probability jittered ambient lines per `OUTDOORS` room,
//!   **suppressed when no players are present**; the `RoomView.ambient`
//!   `weather` glance (a [`HookPoint::RoomAmbient`] hook) carries the
//!   current-conditions `line` every renderer prints as the room's
//!   `Weather` fact (interface plan I3); the persistent `weather` panel feed
//!   emits `{zone, state, temp, icon}` keyframe/delta frames.
//! - **Query seam**: the reserved [`WeatherQuery`] provider (design 04
//!   §10.5) exposes a [`WxSnapshot`] per room for rulesets (WoD perception,
//!   tactics to-hit). Rhai `mush::weather(zone)` is **deferred** — the name is
//!   reserved via `script_api("weather")`, and a future binding calls the
//!   same [`mush_core::Registry::weather_query`] provider.

// WS01-S12: the panic-shaped restriction lints this crate measures zero of,
// held at zero. `crates/mush-contract-tests/tests/restriction_lints.rs` carries
// the measurement this set was cut from — including the counts of the lints
// NOT named here — and fails if this attribute loses one of them.
#![deny(
    clippy::unwrap_used,
    clippy::panic,
    clippy::unreachable,
    clippy::panic_in_result_fn
)]

mod cell;
mod climate;
mod commands;
mod flavor;
pub mod interface;
mod machine;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use mush_core::{
    Audience, CanonicalId, Ctx, EventHook, Flags, GameModule, HookPoint, HookReadContext, ModError,
    ModuleBootData, ModuleCtx, ModuleOp, ModuleWrite, ObjId, ObjKind, PanelFeedHandle, Registrar,
    TickCadence, TickHandler, WeatherQuery, World, WorldEvent, WorldEventKind, WxSnapshot,
};
use mush_proto::{OutputEvent, RichText};
use smol_str::SmolStr;

pub use cell::{
    NS, WIND_MS_PER_UNIT, WeatherCell, WeatherState, WxRng, cell_key, delete_cell, persist_cell,
};
pub use climate::{ClimateDef, ClimateSet, LinePools, WxStateDef, season_key, tod_key};
pub use flavor::{
    FlavorBucket, FlavorGenOutcome, FlavorGenSnapshot, FlavorKey, FlavorLine, FlavorStore,
    MAX_FLAVOR_LINE_BYTES, MAX_FLAVOR_LINES_PER_POOL, MAX_FLAVOR_VARIANTS_PER_BUCKET,
    parse_flavor_key, run_weather_async,
};

use flavor::FlavorBucket as Bucket;
use machine::{pick_ambient_line, pick_transition_line, roll_temp, roll_transition};

/// The default coarse-tick interval: 300 s of OOC wall time. (The design
/// offers "every 15 IC minutes"; a fixed OOC interval is simpler and
/// independent of the IC:OOC ratio, so it is the default — override via
/// [`WeatherConfig::tick`].)
pub const DEFAULT_TICK: Duration = Duration::from_secs(300);

/// The default ambient-line probability, per `OUTDOORS` occupied room, per
/// coarse tick, in parts-per-thousand. At the default 300 s tick this
/// averages roughly one ambient line per room per ~50 minutes of real time —
/// the jittered, spam-averse rate the design calls for. The RNG supplies the
/// jitter; empty rooms are skipped entirely.
pub const DEFAULT_AMBIENT_PERMILLE: u32 = 100;

/// The default number of generated variants `@weather/generate` requests per
/// `(climate, state)` bucket (transition + ambient), per the PLAN's "~N
/// (config, ~10)". Override via [`WeatherConfig::flavor_variants`].
pub const DEFAULT_FLAVOR_VARIANTS: usize = 10;

/// The prefix a `SHELTERED` room's muffled transition line carries, so
/// "sees state changes, muffled" is observable (and testable).
const MUFFLE_PREFIX: &str = "Through the windows, ";

/// Tuning for the weather module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WeatherConfig {
    /// The coarse-tick interval (OOC wall time).
    pub tick: Duration,
    /// Ambient-line probability per `OUTDOORS` occupied room per tick, in
    /// parts-per-thousand.
    pub ambient_permille: u32,
    /// How many generated variants `@weather/generate` requests per bucket
    /// (the M10 LLM flavor top-up). Defaults to [`DEFAULT_FLAVOR_VARIANTS`];
    /// zero disables generation and larger values are clamped to
    /// [`MAX_FLAVOR_VARIANTS_PER_BUCKET`].
    pub flavor_variants: usize,
}

impl Default for WeatherConfig {
    fn default() -> WeatherConfig {
        WeatherConfig {
            tick: DEFAULT_TICK,
            ambient_permille: DEFAULT_AMBIENT_PERMILLE,
            flavor_variants: DEFAULT_FLAVOR_VARIANTS,
        }
    }
}

/// The weather game module. Hand one to `Engine::with_modules`.
pub struct WeatherModule {
    climates: Arc<ClimateSet>,
    config: WeatherConfig,
}

impl WeatherModule {
    /// A fresh module with the shipped climates and default config.
    ///
    /// # Panics
    ///
    /// Panics if the crate's embedded climate data fails to parse — that is a
    /// bug in the shipped data files, caught by this crate's tests.
    #[must_use]
    pub fn new() -> WeatherModule {
        WeatherModule::with_config(WeatherConfig::default())
    }

    /// A fresh module with a custom [`WeatherConfig`].
    ///
    /// # Panics
    ///
    /// See [`WeatherModule::new`].
    #[must_use]
    pub fn with_config(config: WeatherConfig) -> WeatherModule {
        let climates =
            ClimateSet::builtin().expect("shipped climate data files must parse (crate bug)");
        WeatherModule {
            climates: Arc::new(climates),
            config,
        }
    }

    /// A module over a caller-supplied [`ClimateSet`] (custom climate packs, or
    /// tests that need a known set) and [`WeatherConfig`].
    #[must_use]
    pub fn with_climates(climates: ClimateSet, config: WeatherConfig) -> WeatherModule {
        WeatherModule {
            climates: Arc::new(climates),
            config,
        }
    }

    /// The loaded climate registry (shared, immutable).
    #[must_use]
    pub fn climates(&self) -> Arc<ClimateSet> {
        Arc::clone(&self.climates)
    }
}

impl Default for WeatherModule {
    fn default() -> Self {
        WeatherModule::new()
    }
}

impl GameModule for WeatherModule {
    fn name(&self) -> &'static str {
        NS
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        reg.state(WeatherState::default());
        let panel = reg.panel_feed("weather")?;
        reg.tick(
            "weather",
            TickCadence::Coarse(self.config.tick),
            Arc::new(WeatherTick {
                climates: Arc::clone(&self.climates),
                config: self.config,
                panel: panel.clone(),
            }),
        )?;
        // The structured glance under `RoomView.ambient["weather"]`
        // (interface plan I1, D-I-4); its `line` is the prose every renderer
        // prints as the room's `Weather` fact, so the module no longer
        // appends a separate `LookRendered` line (I3).
        reg.hook(
            HookPoint::RoomAmbient,
            Arc::new(WeatherLook {
                climates: Arc::clone(&self.climates),
            }),
        );
        reg.hook(
            HookPoint::Connected,
            Arc::new(WeatherConnect {
                climates: Arc::clone(&self.climates),
                panel: panel.clone(),
            }),
        );
        reg.subscribe(
            WorldEventKind::Moved,
            Arc::new(WeatherMove {
                climates: Arc::clone(&self.climates),
                panel: panel.clone(),
            }),
        );
        // S6: the meteorologist's interface (doc 14 §4.6).
        reg.interface_provider(
            crate::interface::WEATHER_INTERFACE,
            Arc::new(crate::interface::WeatherInterface {
                climates: Arc::clone(&self.climates),
            }),
        )?;
        reg.weather_query(Arc::new(WeatherProvider {
            climates: Arc::clone(&self.climates),
        }))?;
        // The `mush::weather` Rhai namespace is reserved but not yet bound
        // (deferred; see the crate docs) — a future binding reads the same
        // registered `WeatherQuery` provider.
        reg.script_api("weather")?;
        commands::register(reg, Arc::clone(&self.climates), panel, self.config)?;
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        let mut cells: BTreeMap<CanonicalId, WeatherCell> = BTreeMap::new();
        let mut orphans: Vec<CanonicalId> = Vec::new();
        let mut legacy: Vec<SmolStr> = Vec::new();
        let mut flavor = FlavorStore::default();
        let mut stale_flavor: Vec<FlavorKey> = Vec::new();
        for (key, value) in data.kv {
            if let Some(id) = key.strip_prefix("cell:") {
                // S2b: a cell keys on an atlas place id. A pre-atlas cell keyed by a
                // zone dbref (`cell:<digits>`) is dropped loudly — zones keep locks and
                // exits only (doc 14 D-14-28); `@weather/climate <place>=<climate>`
                // opts the neighborhood or region in.
                let place = match CanonicalId::new(id) {
                    Ok(place) if !id.bytes().all(|b| b.is_ascii_digit()) => place,
                    _ => {
                        tracing::warn!(%key, "weather: dropping a zone-keyed cell; weather keys on the atlas now");
                        legacy.push(key.clone());
                        continue;
                    }
                };
                let cell = match serde_json::from_value::<WeatherCell>(value) {
                    Ok(cell) => cell,
                    Err(e) => {
                        tracing::warn!(%key, error = %e, "weather: dropping an unreadable cell");
                        legacy.push(key.clone());
                        continue;
                    }
                };
                if !self.climates.contains(&cell.climate) {
                    tracing::warn!(place = %place, climate = %cell.climate,
                        "weather: dropping cell with unknown climate");
                    orphans.push(place);
                    continue;
                }
                cells.insert(place, cell);
            } else if let Some(fkey) = parse_flavor_key(&key) {
                // The M10 flavor pool. Drop pools for a climate that no longer
                // ships (its lines can never enter selection anyway).
                let lines = serde_json::from_value::<Vec<FlavorLine>>(value)
                    .map_err(|e| ModError::msg(format!("weather: bad flavor doc {key:?}: {e}")))?;
                let key_is_live = self
                    .climates
                    .get(&fkey.climate)
                    .is_some_and(|climate| climate.state(&fkey.state).is_some());
                if !key_is_live {
                    tracing::warn!(
                        climate = %fkey.climate,
                        state = %fkey.state,
                        bucket = fkey.bucket.as_str(),
                        "weather: dropping flavor pool for unknown climate/state"
                    );
                    stale_flavor.push(fkey);
                    continue;
                }
                flavor.load_pool(fkey, lines).map_err(|detail| {
                    ModError::msg(format!("weather: invalid flavor doc {key:?}: {detail}"))
                })?;
            } else {
                // Tolerate unknown keys (future schema).
                continue;
            }
        }
        flavor
            .recompute_next_id()
            .map_err(|detail| ModError::msg(format!("weather: invalid flavor store: {detail}")))?;
        if let Some(st) = ctx.w.module_state_mut::<WeatherState>() {
            st.cells = cells;
            st.flavor = flavor;
        }
        for place in orphans {
            ctx.persist(delete_cell(&place));
        }
        for key in legacy {
            ctx.persist(ModuleOp {
                ns: SmolStr::new_static(NS),
                op: ModuleWrite::Del { key },
            });
        }
        for fkey in stale_flavor {
            ctx.persist(ModuleOp {
                ns: SmolStr::new_static(NS),
                op: ModuleWrite::Del {
                    key: fkey.doc_key(),
                },
            });
        }
        Ok(())
    }
}

// ----------------------------------------------------------------------
// Tick: the state-machine driver
// ----------------------------------------------------------------------

/// The registered [`TickCadence::Coarse`] handler.
struct WeatherTick {
    climates: Arc<ClimateSet>,
    config: WeatherConfig,
    panel: PanelFeedHandle,
}

impl TickHandler for WeatherTick {
    fn tick(&self, ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<mush_core::SceneId>) {
        run_tick(ctx, &self.climates, &self.config, &self.panel);
    }
}

/// One weather step. Phased to keep borrows disjoint: (1) read the world to
/// scan each cell's rooms/occupancy, (2) borrow the state slot to roll every
/// cell and decide outputs (all RNG use lives here), (3) emit and persist.
fn run_tick(
    ctx: &mut Ctx<'_>,
    climates: &ClimateSet,
    config: &WeatherConfig,
    panel: &PanelFeedHandle,
) {
    let now = ctx.game_now_unix();
    let season = ctx.season();
    let tod = ctx.time_of_day();

    // (1) Read-only world scan for the places that have a cell.
    let places: Vec<CanonicalId> = ctx
        .w
        .module_state::<WeatherState>()
        .map(|s| s.cells.keys().cloned().collect())
        .unwrap_or_default();
    if places.is_empty() {
        // Still advance the tick counter for diagnostics.
        if let Some(st) = ctx.w.module_state_mut::<WeatherState>() {
            st.tick_count = st.tick_count.saturating_add(1);
        }
        return;
    }
    let scans: Vec<PlaceScan> = places.iter().map(|p| scan_place(ctx.w, p)).collect();

    // (2) Roll every cell against the machine; collect emissions.
    let mut emits: Vec<Emit> = Vec::new();
    let mut changed: Vec<WeatherCell> = Vec::new();
    {
        let Some(st) = ctx.w.module_state_mut::<WeatherState>() else {
            return;
        };
        st.tick_count = st.tick_count.saturating_add(1);
        for scan in &scans {
            let Some(cell) = st.cells.get_mut(&scan.place) else {
                continue;
            };
            let Some(climate) = climates.get(&cell.climate) else {
                continue; // unknown climate: leave the cell inert
            };
            let from = cell.state.clone();
            let mut state_changed = false;

            if !cell.pinned {
                let to = roll_transition(climate, &from, season, tod, &mut st.rng);
                if to != from {
                    let temp = climate
                        .state(&to)
                        .map(|def| roll_temp(def, season, &mut st.rng))
                        .unwrap_or(cell.temp);
                    cell.state = to.clone();
                    cell.temp = temp;
                    cell.since = now;
                    state_changed = true;
                    // Merge the approved-generated transition overlay (M10
                    // flavor top-up) into selection; empty ⇒ pure M5 behavior.
                    let extra = st
                        .flavor
                        .approved_texts(&cell.climate, &to, Bucket::Transition);
                    let line = pick_transition_line(climate, &from, &to, &extra, &mut st.rng);
                    for room in &scan.rooms {
                        let text = if room.sheltered {
                            muffle(&line)
                        } else {
                            line.clone()
                        };
                        emits.push(Emit::Weather {
                            room: room.room,
                            zone_name: scan.name.clone(),
                            text,
                            icon: to.clone(),
                        });
                        emits.push(Emit::Panel {
                            room: room.room,
                            data: panel_payload(&scan.name, &to, temp),
                        });
                    }
                    changed.push(cell.clone());
                }
            }

            // Ambient: only when the state did NOT change this tick (avoid
            // double lines), only OUTDOORS + occupied rooms.
            if !state_changed {
                let cur = cell.state.clone();
                // The approved-generated ambient overlay is the same for every
                // room in this cell this tick — look it up once.
                let ambient_extra = st
                    .flavor
                    .approved_texts(&cell.climate, &cur, Bucket::Ambient);
                for room in &scan.rooms {
                    if room.sheltered || !room.occupied {
                        continue;
                    }
                    if !st.rng.chance_permille(config.ambient_permille) {
                        continue;
                    }
                    if let Some(line) =
                        pick_ambient_line(climate, &cur, season, tod, &ambient_extra, &mut st.rng)
                    {
                        emits.push(Emit::Weather {
                            room: room.room,
                            zone_name: scan.name.clone(),
                            text: line,
                            icon: cur.clone(),
                        });
                    }
                }
            }
        }
    }

    // (3) Emit + persist.
    for emit in emits {
        match emit {
            Emit::Weather {
                room,
                zone_name,
                text,
                icon,
            } => {
                ctx.out
                    .to_room(room, Vec::new(), weather_event(&zone_name, text, icon));
            }
            Emit::Panel { room, data } => {
                panel.delta_to(
                    ctx,
                    Audience::Room {
                        room,
                        exclude: Vec::new(),
                        subject: None,
                    },
                    data,
                );
            }
        }
    }
    for cell in changed {
        ctx.persist(persist_cell(&cell));
    }
}

/// A queued emission from a tick (built while the state slot is borrowed,
/// replayed against the outbox after the borrow is released).
enum Emit {
    /// A weather line to every occupant of `room`.
    Weather {
        room: ObjId,
        zone_name: String,
        text: String,
        icon: SmolStr,
    },
    /// A panel delta to `room`'s occupants.
    Panel {
        room: ObjId,
        data: serde_json::Value,
    },
}

// ----------------------------------------------------------------------
// RoomAmbient hook: current conditions on outdoor room views
// ----------------------------------------------------------------------

struct WeatherLook {
    climates: Arc<ClimateSet>,
}

impl EventHook for WeatherLook {
    /// The current conditions as a structured glance: `{ state, icon, temp,
    /// visibility, sheltered, line }` under the `weather` ambient key, so a
    /// room band can show an icon and a word without parsing the prose, and
    /// every renderer prints `line` as the room's `Weather` fact.
    fn room_ambient(
        &self,
        ctx: &HookReadContext<'_>,
        _looker: ObjId,
        room: ObjId,
    ) -> Vec<(SmolStr, serde_json::Value)> {
        let Some((cell, def, sheltered)) = resolve_room_weather(ctx.world(), &self.climates, room)
        else {
            return Vec::new();
        };
        vec![(
            SmolStr::new_static("weather"),
            serde_json::json!({
                "state": cell.state,
                "icon": cell.state,
                "temp": cell.temp,
                "visibility": def.visibility,
                "sheltered": sheltered,
                "line": conditions_line(&cell, def, sheltered),
            }),
        )]
    }
}

// ----------------------------------------------------------------------
// Connected hook: send a panel keyframe to a newly-live actor
// ----------------------------------------------------------------------

struct WeatherConnect {
    climates: Arc<ClimateSet>,
    panel: PanelFeedHandle,
}

impl EventHook for WeatherConnect {
    fn connected(&self, ctx: &mut Ctx<'_>, actor: ObjId) {
        let Some(room) = ctx.w.get(actor).and_then(|o| o.location) else {
            return;
        };
        if let Some(data) = current_panel_data(ctx.w, &self.climates, room) {
            self.panel.keyframe(ctx, data);
        }
    }
}

// ----------------------------------------------------------------------
// Moved observer: refresh a mover's weather panel on room change
// ----------------------------------------------------------------------

/// Pushes a fresh panel keyframe to a player when they enter a new room, so a
/// persistent weather panel never lingers on the previous zone's conditions.
/// The tick's transition deltas only reach a room's *current* occupants, and a
/// pinned zone emits none, so without a keyframe on entry a player walking
/// from zone A into zone B keeps seeing A's `{state, temp, icon}`. Mirrors the
/// [`WeatherConnect`] keyframe, keyed on movement instead of connection.
struct WeatherMove {
    climates: Arc<ClimateSet>,
    panel: PanelFeedHandle,
}

impl mush_core::WorldObserver for WeatherMove {
    fn on_event(&self, ctx: &mut Ctx<'_>, ev: &WorldEvent) {
        let WorldEvent::Moved { who, to, .. } = ev else {
            return;
        };
        // Only a live player carries the panel; skip non-connected movers.
        if !ctx.w.is_connected(*who) {
            return;
        }
        if let Some(data) = current_panel_data(ctx.w, &self.climates, *to) {
            self.panel.keyframe_to(ctx, Audience::Actor(*who), data);
        }
    }
}

// ----------------------------------------------------------------------
// WeatherQuery provider (the reserved ruleset seam)
// ----------------------------------------------------------------------

struct WeatherProvider {
    climates: Arc<ClimateSet>,
}

impl WeatherQuery for WeatherProvider {
    fn current(&self, w: &World, room: ObjId) -> Option<WxSnapshot> {
        let (cell, def, _sheltered) = resolve_room_weather(w, &self.climates, room)?;
        Some(cell.snapshot(def))
    }
}

// ----------------------------------------------------------------------
// Shared helpers (used by the tick, hooks, provider, and commands)
// ----------------------------------------------------------------------

/// One place's weather-bearing rooms, gathered from the atlas's records.
pub(crate) struct PlaceScan {
    pub(crate) place: CanonicalId,
    pub(crate) name: String,
    pub(crate) rooms: Vec<RoomInfo>,
}

/// One weather-bearing room in a place.
pub(crate) struct RoomInfo {
    pub(crate) room: ObjId,
    /// `SHELTERED` (and not also `OUTDOORS`): muffled, no ambient drip.
    pub(crate) sheltered: bool,
    /// At least one connected occupant right now (ambient suppression).
    pub(crate) occupied: bool,
}

/// The atlas place a cell id names: a neighborhood by pack id, else a region.
pub(crate) fn place_ref(w: &World, place: &CanonicalId) -> Option<mush_atlas::PlaceRef> {
    let records = mush_atlas::place::records(w)?;
    mush_atlas::place::neighborhood_by_pack(records, place)
        .map(mush_atlas::PlaceRef::Neighborhood)
        .or_else(|| {
            mush_atlas::place::region_by_pack(records, place).map(mush_atlas::PlaceRef::Region)
        })
}

/// A place's display name, or its id when the atlas does not know it (yet).
pub(crate) fn place_name(w: &World, place: &CanonicalId) -> String {
    match (place_ref(w, place), mush_atlas::place::records(w)) {
        (Some(p), Some(records)) => p.name(records),
        _ => place.to_string(),
    }
}

/// The place whose cell governs `room` (S2b): its neighborhood when that has a
/// cell, else its region when that has one. A room off the atlas has none.
pub(crate) fn place_of_room(w: &World, room: ObjId) -> Option<(CanonicalId, String)> {
    let records = mush_atlas::place::records(w)?;
    let st = w.module_state::<WeatherState>()?;
    let nb_key = mush_atlas::place::neighborhood_of_room(records, room)?;
    let nb = records.neighborhoods.get(&nb_key)?;
    if st.cells.contains_key(&nb.key.pack) {
        return Some((nb.key.pack.clone(), nb.name.clone()));
    }
    let region = records.regions.get(&nb.region)?;
    if st.cells.contains_key(&region.key.pack) {
        return Some((region.key.pack.clone(), region.name.clone()));
    }
    None
}

/// Scan a place for its `OUTDOORS`/`SHELTERED` rooms and their occupancy: a
/// neighborhood's own rooms, or — for a region — the rooms of every neighborhood
/// in it that has no cell of its own. A room with both flags counts as `OUTDOORS`
/// (full weather).
pub(crate) fn scan_place(w: &World, place: &CanonicalId) -> PlaceScan {
    let mut rooms = Vec::new();
    let mut name = place.to_string();
    if let Some(records) = mush_atlas::place::records(w) {
        let has_cell = |pack: &CanonicalId| {
            w.module_state::<WeatherState>()
                .is_some_and(|st| st.cells.contains_key(pack))
        };
        let mut room_ids: Vec<ObjId> = Vec::new();
        if let Some(nb_key) = mush_atlas::place::neighborhood_by_pack(records, place) {
            if let Some(nb) = records.neighborhoods.get(&nb_key) {
                name.clone_from(&nb.name);
            }
            room_ids.extend(records.rooms_in(&nb_key).map(|g| g.room));
        } else if let Some(region_key) = mush_atlas::place::region_by_pack(records, place) {
            if let Some(region) = records.regions.get(&region_key) {
                name.clone_from(&region.name);
            }
            for nb in records.neighborhoods_in(&region_key) {
                if has_cell(&nb.key.pack) {
                    continue;
                }
                room_ids.extend(records.rooms_in(&nb.key).map(|g| g.room));
            }
        }
        for id in room_ids {
            let Some(o) = w.get(id) else { continue };
            if o.kind != ObjKind::Room {
                continue;
            }
            let outdoors = o.flags.contains(Flags::OUTDOORS);
            let sheltered = o.flags.contains(Flags::SHELTERED);
            if !outdoors && !sheltered {
                continue; // INDOORS: silent
            }
            let occupied = w.contents_of(id).iter().any(|&c| w.is_connected(c));
            rooms.push(RoomInfo {
                room: id,
                sheltered: sheltered && !outdoors,
                occupied,
            });
        }
    }
    PlaceScan {
        place: place.clone(),
        name,
        rooms,
    }
}

/// Muffle a transition line for a `SHELTERED` room.
pub(crate) fn muffle(line: &str) -> String {
    let mut lowered = line.chars();
    let first = lowered.next().map(|c| c.to_ascii_lowercase());
    let rest: String = lowered.collect();
    match first {
        Some(c) => format!("{MUFFLE_PREFIX}{c}{rest}"),
        None => MUFFLE_PREFIX.trim_end().to_string(),
    }
}

/// Build the [`OutputEvent::Weather`] frame for one line.
pub(crate) fn weather_event(zone_name: &str, text: String, icon: SmolStr) -> OutputEvent {
    OutputEvent::Weather {
        zone: SmolStr::new(zone_name),
        text: RichText::tag("weather", text),
        icon,
    }
}

/// The `{zone, state, temp, icon}` panel payload (keyframe/delta body).
pub(crate) fn panel_payload(zone_name: &str, state: &str, temp: i16) -> serde_json::Value {
    serde_json::json!({
        "zone": zone_name,
        "state": state,
        "temp": temp,
        "icon": state,
    })
}

/// Resolve `room` → `(cell, state-def, sheltered)` when the room bears
/// weather (`OUTDOORS`/`SHELTERED`, in a neighborhood or region with a cell whose
/// climate is loaded). `None` for `INDOORS`, off-atlas and unknown-climate rooms.
pub(crate) fn resolve_room_weather<'a>(
    w: &World,
    climates: &'a ClimateSet,
    room: ObjId,
) -> Option<(WeatherCell, &'a WxStateDef, bool)> {
    let o = w.get(room)?;
    if o.kind != ObjKind::Room {
        return None;
    }
    let outdoors = o.flags.contains(Flags::OUTDOORS);
    let sheltered = o.flags.contains(Flags::SHELTERED);
    if !outdoors && !sheltered {
        return None;
    }
    let (place, _) = place_of_room(w, room)?;
    let cell = w.module_state::<WeatherState>()?.cell(&place)?.clone();
    let climate = climates.get(&cell.climate)?;
    let def = climate.state(&cell.state)?;
    Some((cell, def, sheltered && !outdoors))
}

/// The current-conditions line appended to an outdoor room render.
pub(crate) fn conditions_line(cell: &WeatherCell, def: &WxStateDef, sheltered: bool) -> String {
    let desc = describe_state(&cell.state);
    if sheltered {
        format!("Through the windows, the weather outside looks like {desc}.")
    } else {
        format!(
            "Overhead: {desc}, around {}\u{b0}C{}.",
            cell.temp,
            visibility_note(def.visibility)
        )
    }
}

/// The panel keyframe body for `room`'s current weather, or `None` if the
/// room bears none (`INDOORS`/unzoned/unknown-climate).
pub(crate) fn current_panel_data(
    w: &World,
    climates: &ClimateSet,
    room: ObjId,
) -> Option<serde_json::Value> {
    let (cell, _def, _sheltered) = resolve_room_weather(w, climates, room)?;
    let (_, name) = place_of_room(w, room)?;
    Some(panel_payload(&name, &cell.state, cell.temp))
}

/// A short human phrase for a state id (falls back to the id itself for a
/// state a climate defines that this table does not name).
pub(crate) fn describe_state(state: &str) -> &str {
    match state {
        "clear" => "clear skies",
        "overcast" => "an overcast sky",
        "fog" => "thick fog",
        "rain" => "steady rain",
        "storm" => "a storm",
        "snow" => "falling snow",
        other => other,
    }
}

/// A visibility qualifier for the conditions line.
fn visibility_note(visibility: u8) -> &'static str {
    match visibility {
        0..=30 => ", visibility very poor",
        31..=60 => ", visibility poor",
        61..=85 => ", visibility fair",
        _ => "",
    }
}
