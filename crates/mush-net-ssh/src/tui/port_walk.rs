//! **Station & Ship Walk Controller and Ratatui Port Services Drawers**
//! (Track 4 Phase 4B: SSH Kitty Graphics & ANSI TUI parity with Unity & Web 3D).
//!
//! Owns the per-connection spatial avatar state (`x_m`, `y_m`, `floor`,
//! `heading_mdeg`), canonical station & 3-deck ship interior layouts (enriched
//! lazily from `tacsim.content`), doorway wall-collision + hysteresis +
//! shortest-path BFS exit command dispatch, `[E]` interactable focus & Bridge
//! Helm flight handoff, `port.trade` decoding + 1-click Quote-to-Accept buy
//! state machine, and the interactive Ratatui Port Drawers (`Trade`, `Sailing`,
//! `Co-op`, `Simulator`, `NPC Talk`).

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use mush_pixel::station_walk::WalkCamera;
use mush_proto::{NarrativeKind, ObjRef, RoomView};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use serde_json::{Value, json};

use super::keys::{Arrow, Key};

/// Panel/query name for port trade & stowage telemetry.
pub const PORT_TRADE_FEED: &str = "port.trade";
/// Panel/query name for projected tacsim content packs.
pub const TACSIM_CONTENT_FEED: &str = "tacsim.content";

const WALK_SPEED_MPS: f64 = 5.4;
const TURN_MDEG_PER_S: i64 = 135_000;
const HYSTERESIS_M: f64 = 0.85;
const MOVE_STEP_COOLDOWN: Duration = Duration::from_millis(220);
const PENDING_MOVE_TIMEOUT: Duration = Duration::from_millis(2400);
const SPEECH_TTL: Duration = Duration::from_millis(8500);

const QTY_PRESETS: [i64; 5] = [1, 5, 10, 25, 0]; // 0 = MAX FIT

/// Which interactive Port Drawer tab is open.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortDrawerKind {
    Trade,
    Sailing,
    Coop,
    Sim,
    Npc,
}

impl PortDrawerKind {
    #[must_use]
    pub fn next(self) -> PortDrawerKind {
        match self {
            PortDrawerKind::Trade => PortDrawerKind::Sailing,
            PortDrawerKind::Sailing => PortDrawerKind::Coop,
            PortDrawerKind::Coop => PortDrawerKind::Sim,
            PortDrawerKind::Sim => PortDrawerKind::Npc,
            PortDrawerKind::Npc => PortDrawerKind::Trade,
        }
    }

    #[must_use]
    pub fn prev(self) -> PortDrawerKind {
        match self {
            PortDrawerKind::Trade => PortDrawerKind::Npc,
            PortDrawerKind::Sailing => PortDrawerKind::Trade,
            PortDrawerKind::Coop => PortDrawerKind::Sailing,
            PortDrawerKind::Sim => PortDrawerKind::Coop,
            PortDrawerKind::Npc => PortDrawerKind::Sim,
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            PortDrawerKind::Trade => "EXCHANGE",
            PortDrawerKind::Sailing => "SAILING",
            PortDrawerKind::Coop => "CO-OP",
            PortDrawerKind::Sim => "SIMULATOR",
            PortDrawerKind::Npc => "TALK",
        }
    }
}

/// Outcome of handling an input key in [`PortWalkState`].
#[derive(Debug, Default)]
pub struct WalkKeyOutcome {
    pub consumed: bool,
    pub redraw: bool,
    pub commands: Vec<String>,
    pub queries: Vec<(String, Value)>,
    pub enter_helm_cockpit: bool,
    pub close_pixel_view: bool,
}

#[derive(Clone, Debug)]
struct RoomGeom {
    entry_id: String,
    name: String,
    floor: i64,
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
    kind: &'static str,
}

impl RoomGeom {
    fn center(&self) -> (f64, f64) {
        (
            (self.min_x + self.max_x) * 0.5,
            (self.min_y + self.max_y) * 0.5,
        )
    }

    fn contains(&self, x: f64, y: f64, margin: f64) -> bool {
        x >= self.min_x + margin
            && x <= self.max_x - margin
            && y >= self.min_y + margin
            && y <= self.max_y - margin
    }
}

#[derive(Clone, Debug)]
struct PortalGeom {
    from_room: String,
    to_room: String,
    command: String,
    x_m: f64,
    y_m: f64,
    floor: i64,
    width_m: f64,
    stairs_to: Option<i64>,
}

#[derive(Clone, Debug)]
struct InteractableTarget {
    label: String,
    x_m: f64,
    y_m: f64,
    floor: i64,
    action: InteractAction,
}

#[derive(Clone, Debug)]
enum InteractAction {
    OpenDrawer(PortDrawerKind),
    OpenNpc { name: String },
    Command(String),
    TakeBridgeHelm,
}

#[derive(Clone, Debug)]
struct ActorEntry {
    id: String,
    name: String,
    npc: bool,
    x_m: f64,
    y_m: f64,
    floor: i64,
}

#[derive(Clone, Debug, Default)]
pub struct StowageState {
    pub cargo_kg: i64,
    pub max_cargo_kg: i64,
    pub cargo_m3: f64,
    pub max_cargo_m3: f64,
    pub reaction_kg: i64,
    pub max_reaction_kg: i64,
    pub stores_person_days: f64,
    pub crew_count: i64,
}

#[derive(Clone, Debug)]
pub struct AskRow {
    pub listing_id: i64,
    pub good_id: String,
    pub good_name: String,
    pub quantity: i64,
    pub unit_price_minor: i64,
    pub unit_mass_kg: i64,
    pub unit_vol_dm3: i64,
}

#[derive(Clone, Debug)]
pub struct BidRow {
    pub listing_id: i64,
    pub good_id: String,
    pub good_name: String,
    pub quantity_wanted: i64,
    pub unit_price_minor: i64,
}

#[derive(Clone, Debug)]
pub struct CargoLotRow {
    pub good_id: String,
    pub good_name: String,
    pub quantity: i64,
}

#[derive(Clone, Debug)]
struct PendingQuote {
    listing_id: i64,
    quantity: i64,
    max_unit_price_minor: i64,
    created_at: Instant,
}

#[derive(Clone, Debug)]
struct OwnQuoteRow {
    quote_id: String,
    listing_id: i64,
    unit_price_minor: i64,
    expired: bool,
}

const CORRIDORS: [(&str, &str, &str); 5] = [
    (
        "vesta",
        "Vesta Foundry",
        "Short inner-belt ore & alloy run (~3.5 d)",
    ),
    (
        "ceres",
        "Ceres Freeport",
        "Primary cooperative hub & deep-water depot",
    ),
    (
        "pallas",
        "Pallas Highport",
        "Refined chemical & volatiles exchange (~5.0 d)",
    ),
    (
        "hygiea",
        "Hygiea Outer Berth",
        "Outer-belt ice & heavy machinery run (~7.5 d)",
    ),
    (
        "juno",
        "Juno Anchorage",
        "High-margin frontier station (~6.0 d)",
    ),
];

const HULL_SUGGESTIONS: [&str; 6] = [
    "Salt Moth",
    "Copperfinch",
    "Steady Ledger",
    "Quiet Margin",
    "Iron Turnip",
    "Ninth Meridian",
];

const NPC_TOPICS: [(&str, &str); 5] = [
    ("Greeting & Role", "What do you handle at this desk?"),
    (
        "Co-op Charter Terms",
        "Can you walk me through the First Captains' Lay charter?",
    ),
    (
        "Market & Spreads",
        "What goods are moving with healthy margins right now?",
    ),
    (
        "Corridors & Fuel",
        "How much reaction mass and stores should I carry outbound?",
    ),
    (
        "Berth & Clearance",
        "How do I clear the cradle and take the bridge helm?",
    ),
];

const SIM_LOADOUTS: [(&str, &str); 4] = [
    ("Join Red Team", "SIM TEAM red"),
    ("Join Blue Team", "SIM TEAM blue"),
    ("Free Range Proving Ground", "SIM RANGE 1"),
    ("Start Match", "SIM START"),
];

/// Per-connection spatial walk + port services state.
pub struct PortWalkState {
    camera: WalkCamera,
    pack_id: String,
    rooms: Vec<RoomGeom>,
    portals: Vec<PortalGeom>,
    ship_rooms: Vec<RoomGeom>,
    ship_portals: Vec<PortalGeom>,

    aboard_ship: bool,
    seated_at_helm: bool,
    hull_name: String,
    own_hull_name: String,
    docked: bool,
    server_location_key: String,
    active_location_key: String,
    room_title: String,

    you_x: f64,
    you_y: f64,
    you_floor: i64,
    you_heading_mdeg: i64,
    you_stride: i64,
    pulse_phase: i64,

    held_fwd: i8,
    held_strafe: i8,
    held_turn: i8,
    last_tick: Option<Instant>,
    last_step_at: Option<Instant>,
    pending_move_target: Option<(String, Instant)>,
    next_move_allowed_at: Option<Instant>,
    pending_orders: Vec<String>,

    actors: Vec<ActorEntry>,
    speech_bubbles: HashMap<String, (String, Instant)>,
    interactables: Vec<InteractableTarget>,
    focus_idx: usize,

    // Trade & Goods
    goods_meta: HashMap<String, (String, i64, i64)>, // id -> (name, mass_kg, vol_dm3)
    credits_minor: i64,
    stowage: StowageState,
    asks: Vec<AskRow>,
    bids: Vec<BidRow>,
    cargo_lots: Vec<CargoLotRow>,
    own_quotes: Vec<OwnQuoteRow>,
    pending_quote: Option<PendingQuote>,
    content_asked: bool,
    trade_asked_at: Option<Instant>,

    // Interactive Ratatui Drawer
    drawer: Option<PortDrawerKind>,
    trade_subtab: usize, // 0 = Buy Asks, 1 = Sell Bids
    cursor_row: usize,
    qty_preset_idx: usize,
    corridor_idx: usize,
    round_trip: bool,
    hull_suggestion_idx: usize,
    custom_hull_input: String,
    editing_hull_name: bool,
    selected_npc_name: String,
    drawer_log: VecDeque<String>,
}

impl Default for PortWalkState {
    fn default() -> Self {
        Self::new()
    }
}

impl PortWalkState {
    #[must_use]
    pub fn new() -> PortWalkState {
        let (rooms, portals) = builtin_ceres_layout();
        let (ship_rooms, ship_portals) = builtin_dray_layout();
        PortWalkState {
            camera: WalkCamera::Cutaway25D,
            pack_id: "neighborhood.ceres_freeport".to_string(),
            rooms,
            portals,
            ship_rooms,
            ship_portals,
            aboard_ship: false,
            seated_at_helm: false,
            hull_name: String::new(),
            own_hull_name: String::new(),
            docked: true,
            server_location_key: "room.c03_arrivals".to_string(),
            active_location_key: "room.c03_arrivals".to_string(),
            room_title: "Port Watch Arrivals".to_string(),
            you_x: 0.0,
            you_y: 0.0,
            you_floor: 0,
            you_heading_mdeg: 0,
            you_stride: 0,
            pulse_phase: 0,
            held_fwd: 0,
            held_strafe: 0,
            held_turn: 0,
            last_tick: None,
            last_step_at: None,
            pending_move_target: None,
            next_move_allowed_at: None,
            pending_orders: Vec::new(),
            actors: Vec::new(),
            speech_bubbles: HashMap::new(),
            interactables: Vec::new(),
            focus_idx: 0,
            goods_meta: default_goods_meta(),
            credits_minor: 0,
            stowage: StowageState {
                max_cargo_kg: 50_000,
                max_cargo_m3: 180.0,
                max_reaction_kg: 12_000,
                crew_count: 1,
                ..StowageState::default()
            },
            asks: Vec::new(),
            bids: Vec::new(),
            cargo_lots: Vec::new(),
            own_quotes: Vec::new(),
            pending_quote: None,
            content_asked: false,
            trade_asked_at: None,
            drawer: None,
            trade_subtab: 0,
            cursor_row: 0,
            qty_preset_idx: 1, // default 5 units
            corridor_idx: 0,
            round_trip: true,
            hull_suggestion_idx: 0,
            custom_hull_input: HULL_SUGGESTIONS
                .first()
                .copied()
                .unwrap_or("Salt Moth")
                .to_string(),
            editing_hull_name: false,
            selected_npc_name: String::new(),
            drawer_log: VecDeque::new(),
        }
    }

    #[must_use]
    pub fn camera(&self) -> WalkCamera {
        self.camera
    }

    #[must_use]
    pub fn drawer_open(&self) -> bool {
        self.drawer.is_some()
    }

    #[must_use]
    pub fn seated_at_helm(&self) -> bool {
        self.seated_at_helm
    }

    pub fn set_seated_at_helm(&mut self, seated: bool) {
        self.seated_at_helm = seated;
    }

    #[must_use]
    pub fn stowage(&self) -> &StowageState {
        &self.stowage
    }

    #[must_use]
    pub fn credits_minor(&self) -> i64 {
        self.credits_minor
    }

    #[must_use]
    pub fn focused_label(&self) -> Option<&str> {
        self.interactables
            .get(self.focus_idx)
            .map(|i| i.label.as_str())
    }

    pub fn stop_motion(&mut self) {
        self.held_fwd = 0;
        self.held_strafe = 0;
        self.held_turn = 0;
    }

    /// Drain queued commands emitted by 20 Hz continuous walking or 1-click Quote verification.
    pub fn take_orders(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending_orders)
    }

    /// Tick using internal `last_tick` timestamp and queue any emitted movement commands.
    pub fn tick_at(&mut self, now: Instant) -> bool {
        let dt = self.last_tick.map_or(Duration::from_millis(50), |t| {
            now.saturating_duration_since(t)
        });
        self.last_tick = Some(now);
        let moved = self.held_fwd != 0 || self.held_strafe != 0 || self.held_turn != 0;
        let cmds = self.tick(dt, now);
        self.pending_orders.extend(cmds);
        moved
    }

    /// Lazily enqueue `tacsim.content` and `port.trade` queries only when the
    /// server advertises them and the player engages the walk view or drawer.
    pub fn ensure_queries(
        &mut self,
        manifest_panels: &[String],
        out: &mut Vec<(String, Value)>,
        now: Instant,
    ) {
        let has_content = manifest_panels.iter().any(|p| p == TACSIM_CONTENT_FEED);
        if has_content && !self.content_asked {
            self.content_asked = true;
            out.push((TACSIM_CONTENT_FEED.to_string(), json!({})));
        }
        let has_trade = manifest_panels.iter().any(|p| p == PORT_TRADE_FEED);
        let trade_due = self
            .trade_asked_at
            .is_none_or(|at| now.saturating_duration_since(at) >= Duration::from_secs(3));
        if has_trade && trade_due {
            self.trade_asked_at = Some(now);
            out.push((PORT_TRADE_FEED.to_string(), json!({})));
        }
    }

    /// Toggle or open the contextual Port Services Drawer (`Ctrl+T` or `[E]`).
    pub fn toggle_contextual_drawer(
        &mut self,
        manifest_panels: &[String],
        queries: &mut Vec<(String, Value)>,
        now: Instant,
    ) {
        if self.drawer.is_some() {
            self.drawer = None;
            self.editing_hull_name = false;
            return;
        }
        let default_kind = if self.pack_id.contains("ooc") {
            PortDrawerKind::Sim
        } else if self.active_location_key.contains("coop") {
            PortDrawerKind::Coop
        } else if self.active_location_key.contains("dockmaster")
            || self.active_location_key.contains("berth")
            || self.active_location_key == "bridge"
        {
            PortDrawerKind::Sailing
        } else if !self.actors.is_empty() && self.asks.is_empty() {
            if let Some(first_npc) = self
                .actors
                .iter()
                .find(|a| a.npc)
                .or_else(|| self.actors.first())
            {
                self.selected_npc_name.clone_from(&first_npc.name);
            }
            PortDrawerKind::Npc
        } else {
            PortDrawerKind::Trade
        };
        self.open_drawer(default_kind, manifest_panels, queries, now);
    }

    pub fn open_drawer(
        &mut self,
        kind: PortDrawerKind,
        manifest_panels: &[String],
        queries: &mut Vec<(String, Value)>,
        now: Instant,
    ) {
        self.stop_motion();
        self.drawer = Some(kind);
        self.cursor_row = 0;
        self.editing_hull_name = false;
        self.ensure_queries(manifest_panels, queries, now);
    }

    pub fn close_drawer(&mut self) {
        self.drawer = None;
        self.editing_hull_name = false;
    }

    /// Observe a live `RoomView` from the server.
    pub fn observe_room(&mut self, view: &RoomView, self_name: Option<&str>) {
        let was_aboard = self.aboard_ship;
        self.room_title.clone_from(&view.name);

        let tac = view.ambient.get("tacsim").and_then(Value::as_object);
        let atlas = view.ambient.get("atlas").and_then(Value::as_object);

        if let Some(st) = tac
            .and_then(|t| t.get("stowage").or_else(|| t.get("own_stowage")))
            .and_then(Value::as_object)
        {
            self.decode_stowage_obj(st);
        }

        if let Some(own_h) = tac
            .and_then(|t| t.get("own_hull_name"))
            .and_then(Value::as_str)
        {
            self.own_hull_name = own_h.to_string();
        }

        // Detect ship interior vs station room
        let mode_str = tac
            .and_then(|t| t.get("mode"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let comp_from_tac = tac
            .and_then(|t| t.get("room"))
            .and_then(Value::as_str)
            .and_then(|r| self.match_ship_compartment(r));
        let comp_from_title = view
            .name
            .rsplit_once(" - ")
            .and_then(|(_, right)| self.match_ship_compartment(right));

        if mode_str == "interior" || comp_from_tac.is_some() || comp_from_title.is_some() {
            let comp = comp_from_tac
                .or(comp_from_title)
                .unwrap_or_else(|| "airlock".to_string());
            self.aboard_ship = true;
            self.hull_name = tac
                .and_then(|t| t.get("hull_name"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| view.name.split_once(" - ").map(|(h, _)| h.to_string()))
                .unwrap_or_else(|| "Charter Hull".to_string());
            self.docked = tac
                .and_then(|t| t.get("docked"))
                .and_then(Value::as_bool)
                .unwrap_or(true);
            if comp != "bridge" {
                self.seated_at_helm = false;
            }
            self.server_location_key.clone_from(&comp);
            if let Some((pt, _)) = &self.pending_move_target
                && pt == &comp
            {
                self.pending_move_target = None;
            }
            if !was_aboard
                || !self
                    .ship_rooms
                    .iter()
                    .any(|r| r.entry_id == self.active_location_key)
                || (self.active_location_key != comp && self.pending_move_target.is_none())
            {
                self.snap_to_room(&comp);
            }
            self.rebuild_actors_and_interactables(view, self_name);
            return;
        }

        // Station / OOC room
        self.aboard_ship = false;
        self.seated_at_helm = false;

        let want_ooc = atlas
            .and_then(|a| a.get("pack"))
            .and_then(Value::as_str)
            .is_some_and(|p| p.contains("ooc"))
            || view.name.to_ascii_lowercase().contains("simulator")
            || view.name.to_ascii_lowercase().contains("pod bay");
        let want_pack = if want_ooc {
            "neighborhood.ooc_sim_centre"
        } else {
            "neighborhood.ceres_freeport"
        };
        if self.pack_id != want_pack {
            self.pack_id = want_pack.to_string();
            let (r, p) = if want_ooc {
                builtin_ooc_layout()
            } else {
                builtin_ceres_layout()
            };
            self.rooms = r;
            self.portals = p;
        }

        let entry = atlas
            .and_then(|a| a.get("entry"))
            .and_then(Value::as_str)
            .and_then(|e| {
                self.rooms
                    .iter()
                    .find(|r| r.entry_id == e)
                    .map(|r| r.entry_id.clone())
            })
            .or_else(|| {
                let lower = view.name.trim().to_ascii_lowercase();
                self.rooms
                    .iter()
                    .find(|r| {
                        r.name.to_ascii_lowercase() == lower
                            || lower.contains(&r.name.to_ascii_lowercase())
                    })
                    .map(|r| r.entry_id.clone())
            })
            .or_else(|| self.rooms.first().map(|r| r.entry_id.clone()))
            .unwrap_or_else(|| "room.c03_arrivals".to_string());

        self.server_location_key.clone_from(&entry);
        if let Some((pt, _)) = &self.pending_move_target
            && pt == &entry
        {
            self.pending_move_target = None;
        }
        if was_aboard
            || !self
                .rooms
                .iter()
                .any(|r| r.entry_id == self.active_location_key)
        {
            self.snap_to_room(&entry);
        } else if self.active_location_key != entry && self.pending_move_target.is_none() {
            // Snap when teleported or server moved us independently
            let dist = self.find_room(&entry).map_or(99.0, |r| {
                let (cx, cy) = r.center();
                ((cx - self.you_x).powi(2) + (cy - self.you_y).powi(2)).sqrt()
            });
            if dist > 16.0 {
                self.snap_to_room(&entry);
            } else {
                self.active_location_key.clone_from(&entry);
            }
        }

        self.rebuild_actors_and_interactables(view, self_name);
    }

    /// Observe narrative speech for 2.5D/1P actor speech bubbles and drawer transcript.
    pub fn observe_narrative(
        &mut self,
        _kind: NarrativeKind,
        actor: Option<&ObjRef>,
        plain_text: &str,
        now: Instant,
    ) {
        let trimmed = plain_text.trim();
        if trimmed.is_empty() {
            return;
        }
        if let Some(act) = actor {
            let exp = now + SPEECH_TTL;
            self.speech_bubbles
                .insert(act.id.to_string(), (trimmed.to_string(), exp));
            self.speech_bubbles
                .insert(act.name.to_ascii_lowercase(), (trimmed.to_string(), exp));
        }
        if self.drawer.is_some() {
            let line = match actor {
                Some(a) => format!("{}: {trimmed}", a.name),
                None => trimmed.to_string(),
            };
            self.push_drawer_log(line);
        }
    }

    pub fn observe_sys(&mut self, text: &str) {
        if self.drawer.is_some() && !text.trim().is_empty() {
            self.push_drawer_log(text.trim().to_string());
        }
    }

    /// Observe `port.trade` panel/query keyframe and run 1-click Quote-to-Accept verification.
    pub fn observe_port_trade(
        &mut self,
        data: &Value,
        now: Instant,
    ) -> (Vec<String>, Vec<(String, Value)>) {
        let mut cmds = Vec::new();
        let mut queries = Vec::new();
        let Some(obj) = data.as_object() else {
            return (cmds, queries);
        };

        if let Some(cr) = obj
            .get("credits_minor")
            .or_else(|| obj.get("balance_minor"))
            .and_then(Value::as_i64)
        {
            self.credits_minor = cr;
        }
        if let Some(st) = obj.get("stowage").and_then(Value::as_object) {
            self.decode_stowage_obj(st);
        }

        if let Some(asks_arr) = obj.get("asks").and_then(Value::as_array) {
            self.asks.clear();
            for item in asks_arr.iter().take(64) {
                let lid = item
                    .get("listing_id")
                    .or_else(|| item.get("id"))
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                let gid = item
                    .get("good")
                    .or_else(|| item.get("good_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if lid <= 0 || gid.is_empty() {
                    continue;
                }
                let (gname, mass_kg, vol_dm3) = self
                    .goods_meta
                    .get(&gid)
                    .cloned()
                    .unwrap_or_else(|| (gid.clone(), 25, 40));
                let qty = item
                    .get("quantity")
                    .or_else(|| item.get("qty"))
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                let price = item
                    .get("unit_price_minor")
                    .or_else(|| item.get("price_minor"))
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                self.asks.push(AskRow {
                    listing_id: lid,
                    good_id: gid,
                    good_name: gname,
                    quantity: qty,
                    unit_price_minor: price,
                    unit_mass_kg: mass_kg,
                    unit_vol_dm3: vol_dm3,
                });
            }
        }

        if let Some(bids_arr) = obj.get("bids").and_then(Value::as_array) {
            self.bids.clear();
            for item in bids_arr.iter().take(64) {
                let lid = item
                    .get("listing_id")
                    .or_else(|| item.get("id"))
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                let gid = item
                    .get("good")
                    .or_else(|| item.get("good_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if lid <= 0 || gid.is_empty() {
                    continue;
                }
                let gname = self
                    .goods_meta
                    .get(&gid)
                    .map_or_else(|| gid.clone(), |(n, _, _)| n.clone());
                let qty = item
                    .get("quantity_wanted")
                    .or_else(|| item.get("quantity"))
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                let price = item
                    .get("unit_price_minor")
                    .or_else(|| item.get("price_minor"))
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                self.bids.push(BidRow {
                    listing_id: lid,
                    good_id: gid,
                    good_name: gname,
                    quantity_wanted: qty,
                    unit_price_minor: price,
                });
            }
        }

        if let Some(cargo_arr) = obj
            .get("cargo")
            .or_else(|| obj.get("lots"))
            .and_then(Value::as_array)
        {
            self.cargo_lots.clear();
            for item in cargo_arr.iter().take(64) {
                let gid = item
                    .get("good")
                    .or_else(|| item.get("good_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let qty = item.get("quantity").and_then(Value::as_i64).unwrap_or(0);
                if gid.is_empty() || qty <= 0 {
                    continue;
                }
                let gname = self
                    .goods_meta
                    .get(&gid)
                    .map_or_else(|| gid.clone(), |(n, _, _)| n.clone());
                self.cargo_lots.push(CargoLotRow {
                    good_id: gid,
                    good_name: gname,
                    quantity: qty,
                });
            }
        }

        self.own_quotes.clear();
        if let Some(q_arr) = obj
            .get("own")
            .and_then(|o| o.get("quotes"))
            .or_else(|| obj.get("quotes"))
            .and_then(Value::as_array)
        {
            for q in q_arr.iter().take(16) {
                let qid = q
                    .get("quote_id")
                    .or_else(|| q.get("id"))
                    .map(|v| match v {
                        Value::String(s) => s.clone(),
                        Value::Number(n) => n.to_string(),
                        _ => String::new(),
                    })
                    .unwrap_or_default();
                let lid = q.get("listing_id").and_then(Value::as_i64).unwrap_or(0);
                let uprice = q
                    .get("unit_price_minor")
                    .or_else(|| q.get("price_minor"))
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                let expired = q.get("expired").and_then(Value::as_bool).unwrap_or(false);
                if !qid.is_empty() {
                    self.own_quotes.push(OwnQuoteRow {
                        quote_id: qid,
                        listing_id: lid,
                        unit_price_minor: uprice,
                        expired,
                    });
                }
            }
        }

        // Verify pending 1-click Quote -> Accept
        if let Some(pq) = self.pending_quote.clone() {
            if now.saturating_duration_since(pq.created_at) > Duration::from_secs(12) {
                self.pending_quote = None;
            } else if let Some(matched) = self
                .own_quotes
                .iter()
                .find(|q| !q.expired && (q.listing_id == 0 || q.listing_id == pq.listing_id))
            {
                self.pending_quote = None;
                if matched.unit_price_minor <= pq.max_unit_price_minor {
                    let accept = format!("+market/accept {}", matched.quote_id);
                    self.push_drawer_log(format!(
                        "[QUOTE VERIFIED] Accepting #{} ({} units)...",
                        matched.quote_id, pq.quantity
                    ));
                    self.pending_orders.push(accept.clone());
                    cmds.push(accept);
                    queries.push((PORT_TRADE_FEED.to_string(), json!({})));
                } else {
                    self.push_drawer_log(format!(
                        "[PRICE MOVED] Quote {} > expected {}; purchase halted.",
                        format_credits(matched.unit_price_minor),
                        format_credits(pq.max_unit_price_minor)
                    ));
                }
            }
        }

        (cmds, queries)
    }

    /// Observe `tacsim.content` manifest or pack JSON keyframe.
    pub fn observe_tacsim_content(&mut self, data: &Value) -> Vec<(String, Value)> {
        let mut followups = Vec::new();
        let Some(obj) = data.as_object() else {
            return followups;
        };
        if let Some(packs) = obj.get("packs").and_then(Value::as_array) {
            for p in packs.iter().take(16) {
                let id = p.get("id").and_then(Value::as_str).unwrap_or("");
                if id == "goods" || id == self.pack_id {
                    followups.push((TACSIM_CONTENT_FEED.to_string(), json!({ "id": id })));
                }
            }
            return followups;
        }
        let id = obj.get("id").and_then(Value::as_str).unwrap_or("");
        let parsed_pack = obj
            .get("json")
            .and_then(Value::as_str)
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
            .or_else(|| obj.get("pack").cloned());
        let Some(pack_val) = parsed_pack else {
            return followups;
        };
        if id == "goods"
            && let Some(entries) = pack_val.get("entries").and_then(Value::as_object)
        {
            for (gid, gval) in entries {
                let name = gval
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(gid.as_str())
                    .to_string();
                let mass = gval
                    .get("unit_mass_kg")
                    .or_else(|| gval.get("mass_kg"))
                    .and_then(Value::as_i64)
                    .unwrap_or(25)
                    .max(1);
                let vol = gval
                    .get("unit_volume_dm3")
                    .or_else(|| gval.get("volume_dm3"))
                    .and_then(Value::as_i64)
                    .unwrap_or(40)
                    .max(1);
                self.goods_meta.insert(gid.clone(), (name, mass, vol));
            }
        }
        followups
    }

    /// Advance 20 Hz continuous avatar walking, turning, doorway collision, and BFS room sync.
    pub fn tick(&mut self, dt: Duration, now: Instant) -> Vec<String> {
        let mut cmds = Vec::new();
        self.speech_bubbles.retain(|_, (_, exp)| *exp > now);
        self.pulse_phase = (self.pulse_phase + 1) % 60;

        if let Some((_, sent_at)) = &self.pending_move_target
            && now.saturating_duration_since(*sent_at) > PENDING_MOVE_TIMEOUT
        {
            self.pending_move_target = None;
        }

        if self.drawer.is_some() {
            return cmds;
        }

        let secs = dt.as_secs_f64().clamp(0.0, 0.25);
        if self.held_turn != 0 {
            let delta =
                ((TURN_MDEG_PER_S as f64) * secs * f64::from(self.held_turn)).round() as i64;
            self.you_heading_mdeg = (self.you_heading_mdeg + delta).rem_euclid(360_000);
        }

        if self.held_fwd != 0 || self.held_strafe != 0 {
            self.step_avatar(
                f64::from(self.held_fwd) * WALK_SPEED_MPS * secs,
                f64::from(self.held_strafe) * WALK_SPEED_MPS * secs,
                now,
                &mut cmds,
            );
        }

        cmds
    }

    /// Handle a key while either a Port Drawer OR `PictureKind::StationWalk` is active.
    pub fn handle_key(
        &mut self,
        key: Key,
        letters_held: bool,
        manifest_panels: &[String],
        now: Instant,
    ) -> WalkKeyOutcome {
        if self.drawer.is_some() {
            return self.handle_drawer_key(key, manifest_panels, now);
        }

        let mut out = WalkKeyOutcome {
            consumed: true,
            redraw: true,
            ..WalkKeyOutcome::default()
        };

        // Pass global screen digits, chat triggers, palette, and Ctrl/Alt chords to `TuiApp`!
        if matches!(
            key,
            Key::Char('0'..='9' | '"' | ':' | ';' | '\\' | '/' | '?')
                | Key::Enter
                | Key::Ctrl(_)
                | Key::Alt(_)
        ) {
            out.consumed = false;
            out.redraw = false;
            return out;
        }

        match key {
            Key::Esc => {
                self.stop_motion();
                out.close_pixel_view = true;
            }
            Key::Char('v' | 'V') => {
                self.camera = self.camera.next();
            }
            Key::Tab => {
                if !self.interactables.is_empty() {
                    self.focus_idx = (self.focus_idx + 1) % self.interactables.len();
                }
            }
            Key::Char('e' | 'E') => {
                self.trigger_focused_interactable(manifest_panels, now, &mut out);
            }
            Key::Char('m' | 'M') => {
                self.open_drawer(
                    PortDrawerKind::Trade,
                    manifest_panels,
                    &mut out.queries,
                    now,
                );
            }
            Key::Char('k' | 'K') => {
                self.open_drawer(
                    PortDrawerKind::Sailing,
                    manifest_panels,
                    &mut out.queries,
                    now,
                );
            }
            Key::Char('o' | 'O') => {
                self.open_drawer(PortDrawerKind::Coop, manifest_panels, &mut out.queries, now);
            }
            // Held releases under Kitty keyboard protocol (`CSI > 15 u`)
            Key::Release(Arrow::Up) | Key::CharRelease('w' | 'W') => {
                if self.held_fwd > 0 {
                    self.held_fwd = 0;
                }
            }
            Key::Release(Arrow::Down) | Key::CharRelease('s' | 'S') => {
                if self.held_fwd < 0 {
                    self.held_fwd = 0;
                }
            }
            Key::Release(Arrow::Left) | Key::CharRelease('a' | 'A') => {
                if self.held_turn < 0 {
                    self.held_turn = 0;
                }
            }
            Key::Release(Arrow::Right) | Key::CharRelease('d' | 'D') => {
                if self.held_turn > 0 {
                    self.held_turn = 0;
                }
            }
            Key::CharRelease('q' | 'Q') => {
                if self.held_strafe < 0 {
                    self.held_strafe = 0;
                }
            }
            Key::CharRelease('x' | 'X') => {
                if self.held_strafe > 0 {
                    self.held_strafe = 0;
                }
            }
            Key::CharRepeat('w' | 'W' | 's' | 'S' | 'a' | 'A' | 'd' | 'D') if letters_held => {
                out.redraw = false;
            }
            Key::Up | Key::Repeat(Arrow::Up) => {
                if letters_held {
                    self.held_fwd = 1;
                }
                self.step_avatar(1.35, 0.0, now, &mut out.commands);
            }
            Key::Down | Key::Repeat(Arrow::Down) => {
                if letters_held {
                    self.held_fwd = -1;
                }
                self.step_avatar(-1.15, 0.0, now, &mut out.commands);
            }
            Key::Char('w' | 'W') | Key::CharRepeat('w' | 'W') => {
                if letters_held {
                    self.held_fwd = 1;
                }
                self.step_avatar(1.35, 0.0, now, &mut out.commands);
            }
            Key::Char('s' | 'S') | Key::CharRepeat('s' | 'S') => {
                if letters_held {
                    self.held_fwd = -1;
                }
                self.step_avatar(-1.15, 0.0, now, &mut out.commands);
            }
            Key::Left
            | Key::Repeat(Arrow::Left)
            | Key::Char('a' | 'A')
            | Key::CharRepeat('a' | 'A') => {
                if letters_held {
                    self.held_turn = -1;
                }
                self.you_heading_mdeg = (self.you_heading_mdeg - 22_500).rem_euclid(360_000);
            }
            Key::Right
            | Key::Repeat(Arrow::Right)
            | Key::Char('d' | 'D')
            | Key::CharRepeat('d' | 'D') => {
                if letters_held {
                    self.held_turn = 1;
                }
                self.you_heading_mdeg = (self.you_heading_mdeg + 22_500).rem_euclid(360_000);
            }
            Key::Char('q' | 'Q') => {
                self.step_avatar(0.0, -1.2, now, &mut out.commands);
            }
            Key::Char('x' | 'X') => {
                self.step_avatar(0.0, 1.2, now, &mut out.commands);
            }
            _ => {
                out.consumed = false;
                out.redraw = false;
            }
        }

        out
    }

    /// Synthesize the [`mush_pixel::station_walk::STATION_WALK_FEED`] JSON keyframe.
    #[must_use]
    pub fn to_feed_value(&self) -> Value {
        let active_rooms = if self.aboard_ship {
            &self.ship_rooms
        } else {
            &self.rooms
        };
        let active_portals = if self.aboard_ship {
            &self.ship_portals
        } else {
            &self.portals
        };

        let rooms_json: Vec<Value> = active_rooms
            .iter()
            .map(|r| {
                json!({
                    "id": r.entry_id,
                    "name": r.name,
                    "floor": r.floor,
                    "min_x": r.min_x,
                    "min_y": r.min_y,
                    "max_x": r.max_x,
                    "max_y": r.max_y,
                    "active": r.entry_id == self.active_location_key,
                    "kind": r.kind,
                })
            })
            .collect();

        let doors_json: Vec<Value> = active_portals
            .iter()
            .map(|p| {
                json!({
                    "x_m": p.x_m,
                    "y_m": p.y_m,
                    "floor": p.floor,
                    "width_m": p.width_m,
                    "stairs_to": p.stairs_to,
                })
            })
            .collect();

        let props_json: Vec<Value> = self
            .interactables
            .iter()
            .filter_map(|it| {
                let kind = match &it.action {
                    InteractAction::OpenDrawer(PortDrawerKind::Sim) => "pod",
                    InteractAction::OpenDrawer(_) => "desk",
                    InteractAction::TakeBridgeHelm => "helm",
                    InteractAction::Command(c) if c.starts_with("+ship/") => "airlock",
                    InteractAction::Command(_) => "plaque",
                    InteractAction::OpenNpc { .. } => return None,
                };
                Some(json!({
                    "x_m": it.x_m,
                    "y_m": it.y_m,
                    "floor": it.floor,
                    "kind": kind,
                    "label": it.label,
                }))
            })
            .collect();

        let mut props_all = props_json;
        if self.aboard_ship
            && let Some(br) = self.ship_rooms.iter().find(|r| r.entry_id == "bridge")
        {
            let (bx, by) = br.center();
            props_all.push(json!({
                "x_m": bx,
                "y_m": by - 0.8,
                "floor": br.floor,
                "kind": "holotank",
                "label": "Holo-Tank",
            }));
        }

        let mut moored_json = Vec::new();
        if !self.aboard_ship && self.pack_id == "neighborhood.ceres_freeport" {
            let own_title = if self.own_hull_name.is_empty() {
                "Charter Cradle 01"
            } else {
                self.own_hull_name.as_str()
            };
            moored_json.push(json!({
                "x_m": -10.0,
                "y_m": 43.0,
                "name": own_title,
                "style": "freighter",
                "own": !self.own_hull_name.is_empty(),
            }));
            moored_json.push(json!({
                "x_m": 6.0,
                "y_m": 44.0,
                "name": "Cold Comfort",
                "style": "corvette",
                "own": false,
            }));
            moored_json.push(json!({
                "x_m": 20.0,
                "y_m": 43.5,
                "name": "Ninth Ledger",
                "style": "freighter",
                "own": false,
            }));
        }

        let actors_json: Vec<Value> = self
            .actors
            .iter()
            .map(|a| {
                let speech = self
                    .speech_bubbles
                    .get(&a.id)
                    .or_else(|| self.speech_bubbles.get(&a.name.to_ascii_lowercase()))
                    .map_or("", |(s, _)| s.as_str());
                json!({
                    "x_m": a.x_m,
                    "y_m": a.y_m,
                    "floor": a.floor,
                    "name": a.name,
                    "npc": a.npc,
                    "speech": speech,
                })
            })
            .collect();

        let focus_json = self.interactables.get(self.focus_idx).map(|f| {
            json!({
                "x_m": f.x_m,
                "y_m": f.y_m,
                "floor": f.floor,
                "label": f.label,
            })
        });

        let cargo_permille = if self.stowage.max_cargo_kg > 0 {
            ((self.stowage.cargo_kg * 1000) / self.stowage.max_cargo_kg).clamp(0, 1000)
        } else {
            0
        };

        let subtitle = if self.aboard_ship {
            format!(
                "{}  |  {}",
                self.hull_name,
                if self.docked { "Docked" } else { "Under Way" }
            )
        } else if !self.own_hull_name.is_empty() {
            format!("Deck {:+}  |  Hull: {}", self.you_floor, self.own_hull_name)
        } else {
            format!("Deck {:+}  |  No Active Charter", self.you_floor)
        };

        json!({
            "camera": self.camera.as_str(),
            "mode": if self.aboard_ship { "interior" } else { "station" },
            "title": self.room_title,
            "subtitle": subtitle,
            "you": {
                "x_m": self.you_x,
                "y_m": self.you_y,
                "floor": self.you_floor,
                "heading_mdeg": self.you_heading_mdeg,
                "stride": self.you_stride,
            },
            "cargo_fill_permille": cargo_permille,
            "pulse_phase": self.pulse_phase,
            "focus": focus_json,
            "rooms": rooms_json,
            "doors": doors_json,
            "props": props_all,
            "moored": moored_json,
            "actors": actors_json,
        })
    }

    /// HUD top status line and bottom legend when `PictureKind::StationWalk` is active.
    #[must_use]
    pub fn hud_lines(&self, encoder_name: &str) -> (String, String) {
        let cam_name = match self.camera {
            WalkCamera::Cutaway25D => "2.5D Cutaway",
            WalkCamera::FirstPersonRaycast => "1P Corridor",
        };
        let mode_word = if self.aboard_ship {
            "Ship Interior"
        } else {
            "Spaceport"
        };
        let top = format!(
            "{mode_word} ({cam_name})  |  {}  |  {}  |  drawing {encoder_name}",
            self.room_title,
            format_credits(self.credits_minor)
        );
        let bottom = if self.drawer.is_some() {
            "Tab: switch drawer   Up/Down: select   [/]: qty   Enter: execute   Esc: back to walk"
                .to_string()
        } else {
            "WASD/Arrows: walk   v: 2.5D/1P   Tab/e: [E] interact   m: trade   k: sailing   o: co-op   Enter: chat   Esc: close"
                .to_string()
        };
        (top, bottom)
    }

    /// Draw the bottom-third panel under `PictureKind::StationWalk`: either the
    /// active interactive Port Drawer, or Stowage Gauges + `[E]` Nearby + Comms.
    pub fn draw_bottom_panel(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        utf8: bool,
        comms: &dyn Fn(usize) -> Vec<Line<'static>>,
    ) {
        if area.height < 2 {
            return;
        }
        if self.drawer.is_some() {
            let [drawer_area, talk_area] =
                Layout::horizontal([Constraint::Fill(3), Constraint::Fill(2)])
                    .spacing(1)
                    .areas(area);
            self.draw_drawer_box(frame, drawer_area, utf8);
            draw_comms_col(frame, talk_area, comms);
            return;
        }

        let [stow_area, near_area, talk_area] = Layout::horizontal([
            Constraint::Length(36),
            Constraint::Fill(3),
            Constraint::Fill(4),
        ])
        .spacing(1)
        .areas(area);

        let stow_lines = self.stowage_gauge_lines(utf8);
        draw_titled_col(frame, stow_area, "Stowage & Wallet", stow_lines);

        let mut near_lines = Vec::new();
        if self.interactables.is_empty() {
            near_lines.push(Line::from(Span::styled(
                "Walk to a desk, hatch, or crewmate.",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            for (idx, it) in self.interactables.iter().take(6).enumerate() {
                let focused = idx == self.focus_idx;
                let prefix = if focused { "[E] > " } else { "      " };
                let style = if focused {
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Gray)
                };
                near_lines.push(Line::from(Span::styled(
                    format!("{prefix}{}", it.label),
                    style,
                )));
            }
        }
        draw_titled_col(
            frame,
            near_area,
            "Nearby Interactables (Tab / e)",
            near_lines,
        );
        draw_comms_col(frame, talk_area, comms);
    }

    /// Draw the Port Services Drawer as a centered modal overlay when invoked via `Ctrl+T`
    /// in plain ANSI SSH TUI mode (when Kitty/Sixel pixel view is closed).
    pub fn draw_modal_overlay(&self, frame: &mut Frame<'_>, body: Rect, utf8: bool) {
        if self.drawer.is_none() || body.width < 60 || body.height < 12 {
            return;
        }
        let w = body.width.saturating_sub(8).min(108);
        let h = body.height.saturating_sub(2).min(24);
        let x = body.x + (body.width.saturating_sub(w)) / 2;
        let y = body.y + (body.height.saturating_sub(h)) / 2;
        let rect = Rect::new(x, y, w, h);
        frame.render_widget(Clear, rect);
        self.draw_drawer_box(frame, rect, utf8);
    }

    // ---- Private Helpers ----------------------------------------------------

    fn handle_drawer_key(
        &mut self,
        key: Key,
        manifest_panels: &[String],
        now: Instant,
    ) -> WalkKeyOutcome {
        let mut out = WalkKeyOutcome {
            consumed: true,
            redraw: true,
            ..WalkKeyOutcome::default()
        };
        let Some(kind) = self.drawer else {
            return out;
        };

        // Free-text hull name editing inside Co-op drawer
        if kind == PortDrawerKind::Coop && self.editing_hull_name {
            match key {
                Key::Esc => self.editing_hull_name = false,
                Key::Backspace => {
                    self.custom_hull_input.pop();
                }
                Key::Enter => {
                    self.editing_hull_name = false;
                    let trimmed = self.custom_hull_input.trim();
                    if !trimmed.is_empty() {
                        out.commands.push(format!("+coop/enroll {trimmed}"));
                        self.push_drawer_log(format!("> +coop/enroll {trimmed}"));
                    }
                }
                Key::Char(c)
                    if (c.is_ascii_alphanumeric() || c == ' ' || c == '-' || c == '\'')
                        && self.custom_hull_input.len() < 28 =>
                {
                    self.custom_hull_input.push(c);
                }
                _ => {}
            }
            return out;
        }

        match key {
            Key::Esc | Key::Ctrl('t') => {
                self.close_drawer();
                return out;
            }
            Key::Tab => {
                self.drawer = Some(kind.next());
                self.cursor_row = 0;
                return out;
            }
            Key::BackTab => {
                self.drawer = Some(kind.prev());
                self.cursor_row = 0;
                return out;
            }
            _ => {}
        }

        match kind {
            PortDrawerKind::Trade => {
                let row_count = if self.trade_subtab == 0 {
                    self.asks.len()
                } else {
                    self.bids.len()
                };
                match key {
                    Key::Left | Key::Right | Key::Char('h' | 'l') => {
                        self.trade_subtab = 1 - self.trade_subtab;
                        self.cursor_row = 0;
                    }
                    Key::Up | Key::Char('k') => {
                        self.cursor_row = self.cursor_row.saturating_sub(1);
                    }
                    Key::Down | Key::Char('j') => {
                        if row_count > 0 {
                            self.cursor_row = (self.cursor_row + 1).min(row_count - 1);
                        }
                    }
                    Key::Char('[') => {
                        self.qty_preset_idx = self.qty_preset_idx.saturating_sub(1);
                    }
                    Key::Char(']') => {
                        self.qty_preset_idx = (self.qty_preset_idx + 1).min(QTY_PRESETS.len() - 1);
                    }
                    Key::Char('r') => {
                        self.trade_asked_at = None;
                        self.ensure_queries(manifest_panels, &mut out.queries, now);
                    }
                    Key::Enter | Key::Char('b' | 's') => {
                        if self.trade_subtab == 0 {
                            if let Some(ask) = self.asks.get(self.cursor_row).cloned() {
                                let max_fit = self.max_fit_for_ask(&ask);
                                let preset =
                                    QTY_PRESETS.get(self.qty_preset_idx).copied().unwrap_or(1);
                                let want_qty = if preset == 0 {
                                    max_fit.max(1)
                                } else {
                                    preset.min(ask.quantity.max(1))
                                };
                                self.pending_quote = Some(PendingQuote {
                                    listing_id: ask.listing_id,
                                    quantity: want_qty,
                                    max_unit_price_minor: ask.unit_price_minor,
                                    created_at: now,
                                });
                                out.commands
                                    .push(format!("+market/quote {} {want_qty}", ask.listing_id));
                                out.queries.push((PORT_TRADE_FEED.to_string(), json!({})));
                                self.push_drawer_log(format!(
                                    "[QUOTE] Requesting {want_qty} x {} @ {}...",
                                    ask.good_name,
                                    format_credits(ask.unit_price_minor)
                                ));
                            }
                        } else if let Some(bid) = self.bids.get(self.cursor_row).cloned() {
                            let held = self
                                .cargo_lots
                                .iter()
                                .filter(|c| c.good_id == bid.good_id)
                                .map(|c| c.quantity)
                                .sum::<i64>();
                            let preset = QTY_PRESETS.get(self.qty_preset_idx).copied().unwrap_or(1);
                            let sell_qty = if preset == 0 {
                                held.min(bid.quantity_wanted).max(1)
                            } else {
                                preset.min(bid.quantity_wanted.max(1))
                            };
                            out.commands.push(format!(
                                "+market/sell {} {} {sell_qty}",
                                bid.listing_id, bid.good_id
                            ));
                            out.queries.push((PORT_TRADE_FEED.to_string(), json!({})));
                            self.push_drawer_log(format!(
                                "[SELL] Selling {sell_qty} x {} into bid #{}...",
                                bid.good_name, bid.listing_id
                            ));
                        }
                    }
                    _ => {}
                }
            }
            PortDrawerKind::Sailing => match key {
                Key::Up | Key::Char('k') => {
                    self.corridor_idx = self.corridor_idx.saturating_sub(1);
                }
                Key::Down | Key::Char('j') => {
                    self.corridor_idx = (self.corridor_idx + 1).min(CORRIDORS.len() - 1);
                }
                Key::Char('r') => {
                    self.round_trip = !self.round_trip;
                }
                Key::Char('u') => {
                    if let Some((dest_id, _, _)) = CORRIDORS.get(self.corridor_idx) {
                        out.commands.push(format!("+ship/undock {dest_id}"));
                        self.push_drawer_log(format!("> +ship/undock {dest_id}"));
                    }
                }
                Key::Char('d') => {
                    if let Some((dest_id, _, _)) = CORRIDORS.get(self.corridor_idx) {
                        out.commands.push(format!("+helm/dock {dest_id}"));
                        self.push_drawer_log(format!("> +helm/dock {dest_id}"));
                    }
                }
                Key::Enter => {
                    if let Some((dest_id, label, _)) = CORRIDORS.get(self.corridor_idx) {
                        let cmd = if self.round_trip {
                            format!("+ship/order depart {dest_id}")
                        } else {
                            format!("+ship/plan {dest_id}")
                        };
                        out.commands.push(cmd.clone());
                        self.push_drawer_log(format!("[Sailing] {label}: {cmd}"));
                    }
                }
                _ => {}
            },
            PortDrawerKind::Coop => match key {
                Key::Char('r') => {
                    self.hull_suggestion_idx =
                        (self.hull_suggestion_idx + 1) % HULL_SUGGESTIONS.len();
                    if let Some(&sug) = HULL_SUGGESTIONS.get(self.hull_suggestion_idx) {
                        self.custom_hull_input = sug.to_string();
                    }
                }
                Key::Char('n' | 'e') => {
                    self.editing_hull_name = true;
                }
                Key::Char('t') => {
                    out.commands.push("+coop".to_string());
                    self.push_drawer_log("> +coop".to_string());
                }
                Key::Char('s') => {
                    out.commands.push("+coop/statement".to_string());
                    self.push_drawer_log("> +coop/statement".to_string());
                }
                Key::Enter => {
                    let hull = self.custom_hull_input.trim();
                    if !hull.is_empty() {
                        out.commands.push(format!("+coop/enroll {hull}"));
                        self.push_drawer_log(format!("> +coop/enroll {hull}"));
                    }
                }
                _ => {}
            },
            PortDrawerKind::Sim => match key {
                Key::Up | Key::Char('k') => {
                    self.cursor_row = self.cursor_row.saturating_sub(1);
                }
                Key::Down | Key::Char('j') => {
                    self.cursor_row = (self.cursor_row + 1).min(SIM_LOADOUTS.len() - 1);
                }
                Key::Enter => {
                    if let Some(&(label, cmd)) = SIM_LOADOUTS.get(self.cursor_row) {
                        out.commands.push(cmd.to_string());
                        self.push_drawer_log(format!("[SIM] {label} ({cmd})"));
                    }
                }
                _ => {}
            },
            PortDrawerKind::Npc => match key {
                Key::Up | Key::Char('k') => {
                    self.cursor_row = self.cursor_row.saturating_sub(1);
                }
                Key::Down | Key::Char('j') => {
                    self.cursor_row = (self.cursor_row + 1).min(NPC_TOPICS.len() - 1);
                }
                Key::Char('n') => {
                    if !self.actors.is_empty() {
                        let cur = self
                            .actors
                            .iter()
                            .position(|a| a.name == self.selected_npc_name)
                            .unwrap_or(0);
                        if let Some(next_act) = self.actors.get((cur + 1) % self.actors.len()) {
                            self.selected_npc_name.clone_from(&next_act.name);
                        }
                    }
                }
                Key::Enter => {
                    if let Some(&(_, question)) = NPC_TOPICS.get(self.cursor_row) {
                        let cmd = if self.selected_npc_name.is_empty() {
                            format!("say {question}")
                        } else {
                            format!("say {}, {question}", self.selected_npc_name)
                        };
                        out.commands.push(cmd);
                    }
                }
                _ => {}
            },
        }

        out
    }

    fn trigger_focused_interactable(
        &mut self,
        manifest_panels: &[String],
        now: Instant,
        out: &mut WalkKeyOutcome,
    ) {
        let Some(target) = self.interactables.get(self.focus_idx).cloned() else {
            return;
        };
        // Flush any unreached room transition before triggering an action!
        self.flush_pending_move_steps(&mut out.commands);

        match target.action {
            InteractAction::OpenDrawer(kind) => {
                self.open_drawer(kind, manifest_panels, &mut out.queries, now);
            }
            InteractAction::OpenNpc { name } => {
                self.selected_npc_name = name;
                self.open_drawer(PortDrawerKind::Npc, manifest_panels, &mut out.queries, now);
            }
            InteractAction::Command(cmd) => {
                out.commands.push(cmd);
            }
            InteractAction::TakeBridgeHelm => {
                self.seated_at_helm = true;
                out.commands.push("+helm/station command".to_string());
                out.enter_helm_cockpit = true;
            }
        }
    }

    fn step_avatar(&mut self, fwd_m: f64, strafe_m: f64, now: Instant, cmds: &mut Vec<String>) {
        let rad = ((self.you_heading_mdeg as f64) / 1000.0).to_radians();
        let dx = rad.sin() * fwd_m + rad.cos() * strafe_m;
        let dy = rad.cos() * fwd_m - rad.sin() * strafe_m;
        let cand_x = self.you_x + dx;
        let cand_y = self.you_y + dy;

        if self.is_walkable(cand_x, cand_y, self.you_floor) {
            self.you_x = cand_x;
            self.you_y = cand_y;
            if self
                .last_step_at
                .is_none_or(|t| now.saturating_duration_since(t) >= Duration::from_millis(180))
            {
                self.you_stride = self.you_stride.wrapping_add(1);
                self.last_step_at = Some(now);
            }
        } else if self.is_walkable(cand_x, self.you_y, self.you_floor) {
            self.you_x = cand_x;
        } else if self.is_walkable(self.you_x, cand_y, self.you_floor) {
            self.you_y = cand_y;
        }

        // Check stairway portals first (changes floor + room!)
        let active_portals = if self.aboard_ship {
            &self.ship_portals
        } else {
            &self.portals
        };
        let mut stepped_stair: Option<(i64, String)> = None;
        for p in active_portals {
            if p.from_room == self.active_location_key
                && let Some(dest_floor) = p.stairs_to
                && ((p.x_m - self.you_x).powi(2) + (p.y_m - self.you_y).powi(2)).sqrt() <= 1.65
            {
                stepped_stair = Some((dest_floor, p.to_room.clone()));
                break;
            }
        }
        if let Some((dest_floor, dest_room)) = stepped_stair {
            self.you_floor = dest_floor;
            self.active_location_key = dest_room;
            if let Some(rm) = self.find_room(&self.active_location_key).cloned() {
                let (cx, cy) = rm.center();
                self.you_x = (self.you_x * 0.4) + (cx * 0.6);
                self.you_y = (self.you_y * 0.4) + (cy * 0.6);
            }
        } else {
            // Check horizontal room transition with hysteresis
            let still_inside_current = self.find_room(&self.active_location_key).is_some_and(|r| {
                r.floor == self.you_floor && r.contains(self.you_x, self.you_y, -0.25)
            });
            if !still_inside_current {
                let rooms = if self.aboard_ship {
                    &self.ship_rooms
                } else {
                    &self.rooms
                };
                if let Some(entered) = rooms
                    .iter()
                    .find(|r| {
                        r.floor == self.you_floor
                            && r.contains(self.you_x, self.you_y, HYSTERESIS_M)
                    })
                    .map(|r| r.entry_id.clone())
                {
                    self.active_location_key = entered;
                }
            }
        }

        // 1-in-flight BFS server movement synchronization
        if self.active_location_key != self.server_location_key
            && self.pending_move_target.is_none()
            && self.next_move_allowed_at.is_none_or(|t| now >= t)
            && let Some((cmd, hop_room)) =
                self.first_bfs_step(&self.server_location_key, &self.active_location_key)
        {
            self.pending_move_target = Some((hop_room, now));
            self.next_move_allowed_at = Some(now + MOVE_STEP_COOLDOWN);
            cmds.push(cmd);
        }

        self.refresh_nearest_focus();
    }

    fn flush_pending_move_steps(&mut self, cmds: &mut Vec<String>) {
        if self.active_location_key == self.server_location_key {
            return;
        }
        let mut cur = self.server_location_key.clone();
        for _ in 0..8 {
            if cur == self.active_location_key {
                break;
            }
            let Some((cmd, next_room)) = self.first_bfs_step(&cur, &self.active_location_key)
            else {
                break;
            };
            cmds.push(cmd);
            cur = next_room;
        }
        self.server_location_key
            .clone_from(&self.active_location_key);
        self.pending_move_target = None;
    }

    fn is_walkable(&self, x: f64, y: f64, floor: i64) -> bool {
        let rooms = if self.aboard_ship {
            &self.ship_rooms
        } else {
            &self.rooms
        };
        if rooms
            .iter()
            .any(|r| r.floor == floor && r.contains(x, y, 0.55))
        {
            return true;
        }
        let portals = if self.aboard_ship {
            &self.ship_portals
        } else {
            &self.portals
        };
        portals.iter().any(|p| {
            (p.floor == floor || p.stairs_to == Some(floor))
                && ((p.x_m - x).powi(2) + (p.y_m - y).powi(2)).sqrt() <= (p.width_m * 0.65).max(1.3)
        })
    }

    fn find_room(&self, entry_id: &str) -> Option<&RoomGeom> {
        let list = if self.aboard_ship {
            &self.ship_rooms
        } else {
            &self.rooms
        };
        list.iter().find(|r| r.entry_id == entry_id)
    }

    fn snap_to_room(&mut self, entry_id: &str) {
        self.active_location_key = entry_id.to_string();
        if let Some(r) = self.find_room(entry_id).cloned() {
            let (cx, cy) = r.center();
            self.you_x = cx;
            self.you_y = cy;
            self.you_floor = r.floor;
        }
    }

    fn first_bfs_step(&self, from_id: &str, to_id: &str) -> Option<(String, String)> {
        if from_id == to_id {
            return None;
        }
        let portals = if self.aboard_ship {
            &self.ship_portals
        } else {
            &self.portals
        };
        let mut queue: VecDeque<(String, Option<(String, String)>)> = VecDeque::new();
        let mut visited: HashMap<String, bool> = HashMap::new();
        queue.push_back((from_id.to_string(), None));
        visited.insert(from_id.to_string(), true);

        while let Some((cur, first_hop)) = queue.pop_front() {
            for p in portals {
                if p.from_room != cur || visited.contains_key(&p.to_room) {
                    continue;
                }
                let hop = first_hop
                    .clone()
                    .unwrap_or_else(|| (p.command.clone(), p.to_room.clone()));
                if p.to_room == to_id {
                    return Some(hop);
                }
                visited.insert(p.to_room.clone(), true);
                queue.push_back((p.to_room.clone(), Some(hop)));
            }
        }
        None
    }

    fn match_ship_compartment(&self, raw: &str) -> Option<String> {
        let lower = raw.trim().to_ascii_lowercase();
        for r in &self.ship_rooms {
            if r.entry_id == lower || r.name.to_ascii_lowercase() == lower {
                return Some(r.entry_id.clone());
            }
        }
        if lower.contains("bridge") {
            Some("bridge".to_string())
        } else if lower.contains("airlock") {
            Some("airlock".to_string())
        } else if lower.contains("hold") || lower.contains("cargo") {
            Some("hold".to_string())
        } else if lower.contains("eng") {
            Some("engineering".to_string())
        } else if lower.contains("mess") || lower.contains("galley") {
            Some("mess".to_string())
        } else if lower.contains("quarter") || lower.contains("bunk") {
            Some("quarters".to_string())
        } else {
            None
        }
    }

    fn rebuild_actors_and_interactables(&mut self, view: &RoomView, self_name: Option<&str>) {
        self.actors.clear();
        self.interactables.clear();

        let (rcx, rcy, rfloor, rmin_y, rmax_y) =
            self.find_room(&self.active_location_key)
                .map_or((0.0, 0.0, 0, -6.0, 6.0), |r| {
                    let (cx, cy) = r.center();
                    (cx, cy, r.floor, r.min_y, r.max_y)
                });

        // Collect dormant NPC IDs so unstaffed desks stay empty
        let mut dormant = HashMap::new();
        if let Some(d_arr) = view.ambient.get("dormant_npcs").and_then(Value::as_array) {
            for d in d_arr {
                if let Some(s) = d.as_str() {
                    dormant.insert(s.to_ascii_lowercase(), true);
                }
            }
        }

        // Actors from `ambient.npcs` + `ambient.players` (falling back to `view.contents`)
        let mut idx = 0usize;
        if let Some(npcs) = view.ambient.get("npcs").and_then(Value::as_array) {
            for n in npcs.iter().take(12) {
                let id = n
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let name = n
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if name.is_empty()
                    || dormant.contains_key(&id.to_ascii_lowercase())
                    || dormant.contains_key(&name.to_ascii_lowercase())
                {
                    continue;
                }
                let angle = (idx as f64) * 1.25 + 0.5;
                let ax = rcx + angle.cos() * 2.6;
                let ay = rcy + angle.sin() * 2.2;
                self.actors.push(ActorEntry {
                    id,
                    name: name.clone(),
                    npc: true,
                    x_m: ax,
                    y_m: ay,
                    floor: rfloor,
                });
                self.interactables.push(InteractableTarget {
                    label: format!("Talk to {name}"),
                    x_m: ax,
                    y_m: ay,
                    floor: rfloor,
                    action: InteractAction::OpenNpc { name },
                });
                idx += 1;
            }
        }

        if let Some(players) = view.ambient.get("players").and_then(Value::as_array) {
            for p in players.iter().take(12) {
                let id = p
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let name = p
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if name.is_empty() || self_name.is_some_and(|me| me.eq_ignore_ascii_case(&name)) {
                    continue;
                }
                let angle = (idx as f64) * 1.25 + 0.9;
                let ax = rcx + angle.cos() * 2.4;
                let ay = rcy + angle.sin() * 2.0;
                self.actors.push(ActorEntry {
                    id,
                    name: name.clone(),
                    npc: false,
                    x_m: ax,
                    y_m: ay,
                    floor: rfloor,
                });
                self.interactables.push(InteractableTarget {
                    label: format!("Greet {name}"),
                    x_m: ax,
                    y_m: ay,
                    floor: rfloor,
                    action: InteractAction::OpenNpc { name },
                });
                idx += 1;
            }
        }

        if self.actors.is_empty() {
            for item in view.contents.iter().take(8) {
                if self_name.is_some_and(|me| me.eq_ignore_ascii_case(&item.name)) {
                    continue;
                }
                if dormant.contains_key(&item.name.to_ascii_lowercase()) {
                    continue;
                }
                let angle = (idx as f64) * 1.3 + 0.6;
                let ax = rcx + angle.cos() * 2.5;
                let ay = rcy + angle.sin() * 2.0;
                self.actors.push(ActorEntry {
                    id: item.id.to_string(),
                    name: item.name.clone(),
                    npc: true,
                    x_m: ax,
                    y_m: ay,
                    floor: rfloor,
                });
                self.interactables.push(InteractableTarget {
                    label: format!("Talk to {}", item.name),
                    x_m: ax,
                    y_m: ay,
                    floor: rfloor,
                    action: InteractAction::OpenNpc {
                        name: item.name.clone(),
                    },
                });
                idx += 1;
            }
        }

        // Architectural interactables for current room / compartment
        if self.aboard_ship {
            match self.active_location_key.as_str() {
                "bridge" => {
                    self.interactables.insert(
                        0,
                        InteractableTarget {
                            label: "Take Bridge Helm (+helm/station command)".to_string(),
                            x_m: rcx,
                            y_m: rmax_y - 1.2,
                            floor: rfloor,
                            action: InteractAction::TakeBridgeHelm,
                        },
                    );
                    self.interactables.push(InteractableTarget {
                        label: "Sailing & Corridor Plan".to_string(),
                        x_m: rcx + 1.6,
                        y_m: rcy,
                        floor: rfloor,
                        action: InteractAction::OpenDrawer(PortDrawerKind::Sailing),
                    });
                }
                "airlock" => {
                    self.interactables.insert(
                        0,
                        InteractableTarget {
                            label: "Disembark to Berth (+ship/leave)".to_string(),
                            x_m: rcx,
                            y_m: rmin_y + 1.0,
                            floor: rfloor,
                            action: InteractAction::Command("+ship/leave".to_string()),
                        },
                    );
                }
                "hold" => {
                    self.interactables.insert(
                        0,
                        InteractableTarget {
                            label: "Inspect Hold & Port Exchange".to_string(),
                            x_m: rcx,
                            y_m: rcy,
                            floor: rfloor,
                            action: InteractAction::OpenDrawer(PortDrawerKind::Trade),
                        },
                    );
                }
                _ => {}
            }
        } else {
            let key = self.active_location_key.as_str();
            if key.contains("coop") {
                self.interactables.insert(
                    0,
                    InteractableTarget {
                        label: "Co-op Charter & Enrollment Desk".to_string(),
                        x_m: rcx,
                        y_m: rmax_y - 1.8,
                        floor: rfloor,
                        action: InteractAction::OpenDrawer(PortDrawerKind::Coop),
                    },
                );
            } else if key.contains("exchange") || key.contains("chandlery") {
                self.interactables.insert(
                    0,
                    InteractableTarget {
                        label: "Port Commodity Exchange Board".to_string(),
                        x_m: rcx,
                        y_m: rmax_y - 1.8,
                        floor: rfloor,
                        action: InteractAction::OpenDrawer(PortDrawerKind::Trade),
                    },
                );
            } else if key.contains("dockmaster") {
                self.interactables.insert(
                    0,
                    InteractableTarget {
                        label: "Sailing & Corridor Route Desk".to_string(),
                        x_m: rcx,
                        y_m: rmax_y - 1.8,
                        floor: rfloor,
                        action: InteractAction::OpenDrawer(PortDrawerKind::Sailing),
                    },
                );
            } else if key.contains("berth") {
                self.interactables.insert(
                    0,
                    InteractableTarget {
                        label: "Board Cradled Hull (+ship/board)".to_string(),
                        x_m: rcx,
                        y_m: rmax_y - 1.2,
                        floor: rfloor,
                        action: InteractAction::Command("+ship/board".to_string()),
                    },
                );
            } else if self.pack_id.contains("ooc") {
                self.interactables.insert(
                    0,
                    InteractableTarget {
                        label: "Simulator Pod Console".to_string(),
                        x_m: rcx,
                        y_m: rmax_y - 1.6,
                        floor: rfloor,
                        action: InteractAction::OpenDrawer(PortDrawerKind::Sim),
                    },
                );
            }
        }

        self.refresh_nearest_focus();
    }

    fn refresh_nearest_focus(&mut self) {
        if self.interactables.is_empty() {
            self.focus_idx = 0;
            return;
        }
        let mut best_idx = self.focus_idx.min(self.interactables.len() - 1);
        let mut best_dist = f64::MAX;
        for (i, it) in self.interactables.iter().enumerate() {
            if it.floor != self.you_floor {
                continue;
            }
            let d = ((it.x_m - self.you_x).powi(2) + (it.y_m - self.you_y).powi(2)).sqrt();
            if d < best_dist {
                best_dist = d;
                best_idx = i;
            }
        }
        if best_dist <= 5.0 {
            self.focus_idx = best_idx;
        } else {
            self.focus_idx = self.focus_idx.min(self.interactables.len() - 1);
        }
    }

    fn decode_stowage_obj(&mut self, st: &serde_json::Map<String, Value>) {
        if let Some(v) = st.get("cargo_kg").and_then(Value::as_i64) {
            self.stowage.cargo_kg = v;
        }
        if let Some(v) = st
            .get("max_cargo_kg")
            .and_then(Value::as_i64)
            .filter(|&n| n > 0)
        {
            self.stowage.max_cargo_kg = v;
        }
        if let Some(v) = st.get("cargo_m3").and_then(Value::as_f64) {
            self.stowage.cargo_m3 = v;
        }
        if let Some(v) = st
            .get("max_cargo_m3")
            .and_then(Value::as_f64)
            .filter(|&n| n > 0.0)
        {
            self.stowage.max_cargo_m3 = v;
        }
        if let Some(v) = st
            .get("reaction_kg")
            .or_else(|| st.get("fuel_kg"))
            .and_then(Value::as_i64)
        {
            self.stowage.reaction_kg = v;
        }
        if let Some(v) = st
            .get("max_reaction_kg")
            .or_else(|| st.get("max_fuel_kg"))
            .and_then(Value::as_i64)
            .filter(|&n| n > 0)
        {
            self.stowage.max_reaction_kg = v;
        }
        if let Some(v) = st.get("stores_person_days").and_then(Value::as_f64) {
            self.stowage.stores_person_days = v;
        }
        if let Some(v) = st
            .get("crew_count")
            .and_then(Value::as_i64)
            .filter(|&n| n > 0)
        {
            self.stowage.crew_count = v;
        }
    }

    fn max_fit_for_ask(&self, ask: &AskRow) -> i64 {
        let by_avail = ask.quantity.max(0);
        let by_cash = if ask.unit_price_minor > 0 {
            (self.credits_minor.max(0)) / ask.unit_price_minor
        } else {
            by_avail
        };
        let free_kg = (self.stowage.max_cargo_kg - self.stowage.cargo_kg).max(0);
        let by_mass = if ask.unit_mass_kg > 0 {
            free_kg / ask.unit_mass_kg
        } else {
            by_avail
        };
        let free_dm3 =
            ((self.stowage.max_cargo_m3 - self.stowage.cargo_m3).max(0.0) * 1000.0).floor() as i64;
        let by_vol = if ask.unit_vol_dm3 > 0 {
            free_dm3 / ask.unit_vol_dm3
        } else {
            by_avail
        };
        by_avail.min(by_cash).min(by_mass).min(by_vol).max(0)
    }

    fn push_drawer_log(&mut self, line: String) {
        self.drawer_log.push_back(line);
        while self.drawer_log.len() > 4 {
            self.drawer_log.pop_front();
        }
    }

    fn stowage_gauge_lines(&self, utf8: bool) -> Vec<Line<'static>> {
        let st = &self.stowage;
        let days = if st.crew_count > 0 {
            st.stores_person_days / (st.crew_count as f64)
        } else {
            st.stores_person_days
        };
        vec![
            Line::from(vec![
                Span::styled("Wallet: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    format_credits(self.credits_minor),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            gauge_row(
                "Hold Mass",
                st.cargo_kg as f64,
                st.max_cargo_kg.max(1) as f64,
                format!(
                    "{:.1}/{:.1} t",
                    st.cargo_kg as f64 / 1000.0,
                    st.max_cargo_kg as f64 / 1000.0
                ),
                utf8,
            ),
            gauge_row(
                "Packed Vol",
                st.cargo_m3,
                st.max_cargo_m3.max(1.0),
                format!("{:.0}/{:.0} m3", st.cargo_m3, st.max_cargo_m3),
                utf8,
            ),
            gauge_row(
                "Reaction",
                st.reaction_kg as f64,
                st.max_reaction_kg.max(1) as f64,
                format!("{:.1} t", st.reaction_kg as f64 / 1000.0),
                utf8,
            ),
            gauge_row(
                "Stores",
                days,
                30.0,
                format!("{days:.1} d ({} crew)", st.crew_count),
                utf8,
            ),
        ]
    }

    fn draw_drawer_box(&self, frame: &mut Frame<'_>, area: Rect, utf8: bool) {
        let Some(active) = self.drawer else {
            return;
        };
        let all_tabs = [
            PortDrawerKind::Trade,
            PortDrawerKind::Sailing,
            PortDrawerKind::Coop,
            PortDrawerKind::Sim,
            PortDrawerKind::Npc,
        ];
        let mut title_spans = vec![Span::raw(" PORT SERVICES: ")];
        for t in all_tabs {
            if t == active {
                title_spans.push(Span::styled(
                    format!("[{}] ", t.label()),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ));
            } else {
                title_spans.push(Span::styled(
                    format!(" {}  ", t.label()),
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(Line::from(title_spans));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.height == 0 || inner.width < 20 {
            return;
        }

        let mut lines: Vec<Line<'static>> = Vec::new();
        match active {
            PortDrawerKind::Trade => {
                for g in self.stowage_gauge_lines(utf8).into_iter().take(3) {
                    lines.push(g);
                }
                let preset = QTY_PRESETS.get(self.qty_preset_idx).copied().unwrap_or(1);
                let qty_word = if preset == 0 {
                    "Max Fit".to_string()
                } else {
                    format!("{preset}")
                };
                let buy_style = if self.trade_subtab == 0 {
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                let sell_style = if self.trade_subtab == 1 {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                lines.push(Line::from(vec![
                    Span::styled(" < ", Style::default().fg(Color::DarkGray)),
                    Span::styled("Buy Asks", buy_style),
                    Span::raw(" | "),
                    Span::styled("Sell Bids", sell_style),
                    Span::styled(" >   ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        format!("Lot Qty ([/]): {qty_word}   r: refresh"),
                        Style::default().fg(Color::Cyan),
                    ),
                ]));

                if self.trade_subtab == 0 {
                    if self.asks.is_empty() {
                        lines.push(Line::from(Span::styled(
                            "No standing asks loaded yet (press r at a port exchange).",
                            Style::default().fg(Color::DarkGray),
                        )));
                    } else {
                        for (idx, ask) in self.asks.iter().take(6).enumerate() {
                            let sel = idx == self.cursor_row;
                            let fit = self.max_fit_for_ask(ask);
                            let mark = if sel { "> " } else { "  " };
                            let st = if sel {
                                Style::default()
                                    .fg(Color::White)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().fg(Color::Gray)
                            };
                            lines.push(Line::from(Span::styled(
                                format!(
                                    "{mark}#{:<4} {:<18} {:>5} avail  {:>10}/u  (fit: {fit})",
                                    ask.listing_id,
                                    truncate_str(&ask.good_name, 18),
                                    ask.quantity,
                                    format_credits(ask.unit_price_minor)
                                ),
                                st,
                            )));
                        }
                    }
                } else if self.bids.is_empty() {
                    lines.push(Line::from(Span::styled(
                        "No standing bids loaded yet (press r at a port exchange).",
                        Style::default().fg(Color::DarkGray),
                    )));
                } else {
                    for (idx, bid) in self.bids.iter().take(6).enumerate() {
                        let sel = idx == self.cursor_row;
                        let held: i64 = self
                            .cargo_lots
                            .iter()
                            .filter(|c| c.good_id == bid.good_id)
                            .map(|c| c.quantity)
                            .sum();
                        let mark = if sel { "> " } else { "  " };
                        let st = if sel {
                            Style::default()
                                .fg(Color::White)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::Gray)
                        };
                        lines.push(Line::from(Span::styled(
                            format!(
                                "{mark}#{:<4} {:<18} wants {:>5}  {:>10}/u  (in hold: {held})",
                                bid.listing_id,
                                truncate_str(&bid.good_name, 18),
                                bid.quantity_wanted,
                                format_credits(bid.unit_price_minor)
                            ),
                            st,
                        )));
                    }
                }
            }
            PortDrawerKind::Sailing => {
                lines.push(Line::from(Span::styled(
                    format!(
                        "Action: {} (r toggles)   Enter: run   u: +ship/undock   d: +helm/dock",
                        if self.round_trip {
                            "Queue Depart (+ship/order)"
                        } else {
                            "Dry-Run Passage (+ship/plan)"
                        }
                    ),
                    Style::default().fg(Color::Cyan),
                )));
                for (idx, &(_, name, desc)) in CORRIDORS.iter().enumerate() {
                    let sel = idx == self.cursor_idx_clamped(CORRIDORS.len());
                    let mark = if sel { "> " } else { "  " };
                    let st = if sel {
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::Gray)
                    };
                    lines.push(Line::from(Span::styled(
                        format!("{mark}{name:<20} {desc}"),
                        st,
                    )));
                }
            }
            PortDrawerKind::Coop => {
                lines.push(Line::from(Span::styled(
                    "First Captains' Lay Charter: zero upfront cash; cooperative lay settles from voyage margin.",
                    Style::default().fg(Color::Gray),
                )));
                let edit_tag = if self.editing_hull_name {
                    " (TYPING - Enter submits, Esc stops)"
                } else {
                    " (r: cycle suggestion, n: type custom, Enter: enroll)"
                };
                lines.push(Line::from(vec![
                    Span::styled("Hull Name: ", Style::default().fg(Color::Cyan)),
                    Span::styled(
                        self.custom_hull_input.clone(),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(edit_tag, Style::default().fg(Color::DarkGray)),
                ]));
                lines.push(Line::from(Span::styled(
                    "Keys: Enter -> +coop/enroll <name>   t -> +coop (terms)   s -> +coop/statement",
                    Style::default().fg(Color::Green),
                )));
            }
            PortDrawerKind::Sim => {
                lines.push(Line::from(Span::styled(
                    "Simulator Center Pod Bay — select a training slot and press Enter:",
                    Style::default().fg(Color::Cyan),
                )));
                for (idx, &(label, cmd)) in SIM_LOADOUTS.iter().enumerate() {
                    let sel = idx == self.cursor_row.min(SIM_LOADOUTS.len() - 1);
                    let mark = if sel { "> " } else { "  " };
                    let st = if sel {
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::Gray)
                    };
                    lines.push(Line::from(Span::styled(
                        format!("{mark}{label:<34} ({cmd})"),
                        st,
                    )));
                }
            }
            PortDrawerKind::Npc => {
                let speaker = if self.selected_npc_name.is_empty() {
                    "Room".to_string()
                } else {
                    format!("{} (n cycles speaker)", self.selected_npc_name)
                };
                lines.push(Line::from(Span::styled(
                    format!("Speaking with: {speaker}"),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )));
                for (idx, &(topic, q)) in NPC_TOPICS.iter().enumerate() {
                    let sel = idx == self.cursor_row.min(NPC_TOPICS.len() - 1);
                    let mark = if sel { "> " } else { "  " };
                    let st = if sel {
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::Gray)
                    };
                    lines.push(Line::from(Span::styled(
                        format!("{mark}{topic:<22} \"{q}\""),
                        st,
                    )));
                }
            }
        }

        for log_ln in &self.drawer_log {
            lines.push(Line::from(Span::styled(
                format!("  {log_ln}"),
                Style::default().fg(Color::Yellow),
            )));
        }

        let max_rows = usize::from(inner.height);
        frame.render_widget(
            Paragraph::new(lines.into_iter().take(max_rows).collect::<Vec<_>>()),
            inner,
        );
    }

    fn cursor_idx_clamped(&self, len: usize) -> usize {
        if len == 0 {
            0
        } else {
            self.corridor_idx.min(len - 1)
        }
    }
}

// ---- Formatting & Canonical Built-in Layouts --------------------------------

fn format_credits(minor: i64) -> String {
    let sign = if minor < 0 { "-" } else { "" };
    let abs = minor.unsigned_abs();
    let whole = abs / 100;
    let cents = abs % 100;
    format!("{sign}{whole}.{cents:02} cr")
}

fn truncate_str(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i >= max_chars {
            break;
        }
        out.push(ch);
    }
    out
}

fn gauge_row(label: &str, used: f64, cap: f64, right: String, utf8: bool) -> Line<'static> {
    let ratio = if cap > 0.0 {
        (used / cap).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let width = 8usize;
    let filled = ((ratio * (width as f64)).round() as usize).min(width);
    let (on_ch, off_ch) = if utf8 {
        ('\u{2588}', '\u{2591}')
    } else {
        ('#', '-')
    };
    let mut bar = String::with_capacity(width);
    for i in 0..width {
        bar.push(if i < filled { on_ch } else { off_ch });
    }
    let bar_col = if ratio >= 0.92 {
        Color::Red
    } else if ratio >= 0.75 {
        Color::Yellow
    } else {
        Color::Cyan
    };
    Line::from(vec![
        Span::styled(format!("{label:<10} "), Style::default().fg(Color::Gray)),
        Span::styled(bar, Style::default().fg(bar_col)),
        Span::styled(format!(" {right}"), Style::default().fg(Color::White)),
    ])
}

fn draw_titled_col(frame: &mut Frame<'_>, area: Rect, title: &str, lines: Vec<Line<'static>>) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let max_h = usize::from(inner.height);
    frame.render_widget(
        Paragraph::new(lines.into_iter().take(max_h).collect::<Vec<_>>()),
        inner,
    );
}

fn draw_comms_col(frame: &mut Frame<'_>, area: Rect, comms: &dyn Fn(usize) -> Vec<Line<'static>>) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            " COMMS ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rows = comms(usize::from(inner.width));
    let skip = rows.len().saturating_sub(usize::from(inner.height));
    frame.render_widget(
        Paragraph::new(rows.into_iter().skip(skip).collect::<Vec<_>>()),
        inner,
    );
}

fn default_goods_meta() -> HashMap<String, (String, i64, i64)> {
    let mut m = HashMap::new();
    m.insert("water_ice".to_string(), ("Water Ice".to_string(), 50, 55));
    m.insert(
        "reaction_water".to_string(),
        ("Reaction Water".to_string(), 50, 50),
    );
    m.insert(
        "crew_stores".to_string(),
        ("Crew Stores".to_string(), 20, 35),
    );
    m.insert(
        "ore_concentrate".to_string(),
        ("Ore Concentrate".to_string(), 80, 30),
    );
    m.insert(
        "hull_plate".to_string(),
        ("Hull Plate".to_string(), 100, 25),
    );
    m.insert(
        "machine_spares".to_string(),
        ("Machine Spares".to_string(), 40, 60),
    );
    m.insert(
        "volatiles".to_string(),
        ("Refined Volatiles".to_string(), 35, 55),
    );
    m.insert(
        "med_supplies".to_string(),
        ("Medical Supplies".to_string(), 15, 30),
    );
    m
}

fn builtin_ceres_layout() -> (Vec<RoomGeom>, Vec<PortalGeom>) {
    let rooms = vec![
        RoomGeom {
            entry_id: "room.c03_arrivals".into(),
            name: "Port Watch Arrivals".into(),
            floor: 0,
            min_x: -10.0,
            min_y: -8.0,
            max_x: 10.0,
            max_y: 8.0,
            kind: "desk",
        },
        RoomGeom {
            entry_id: "room.c06_concourse".into(),
            name: "Station Concourse".into(),
            floor: 0,
            min_x: 10.0,
            min_y: -10.0,
            max_x: 34.0,
            max_y: 10.0,
            kind: "corridor",
        },
        RoomGeom {
            entry_id: "room.c08_coop".into(),
            name: "Registry & Co-op Desk".into(),
            floor: 0,
            min_x: -8.0,
            min_y: 8.0,
            max_x: 10.0,
            max_y: 22.0,
            kind: "desk",
        },
        RoomGeom {
            entry_id: "room.c07_exchange".into(),
            name: "Exchange Hall".into(),
            floor: 0,
            min_x: 12.0,
            min_y: 10.0,
            max_x: 34.0,
            max_y: 26.0,
            kind: "desk",
        },
        RoomGeom {
            entry_id: "room.c09_chandlery".into(),
            name: "Port Chandlery".into(),
            floor: 0,
            min_x: 34.0,
            min_y: -8.0,
            max_x: 50.0,
            max_y: 8.0,
            kind: "desk",
        },
        RoomGeom {
            entry_id: "room.c10_canteen".into(),
            name: "Watch Canteen".into(),
            floor: 0,
            min_x: 12.0,
            min_y: -24.0,
            max_x: 32.0,
            max_y: -10.0,
            kind: "room",
        },
        RoomGeom {
            entry_id: "room.c04_freight".into(),
            name: "Freight Deck".into(),
            floor: -1,
            min_x: -12.0,
            min_y: -10.0,
            max_x: 14.0,
            max_y: 10.0,
            kind: "hold",
        },
        RoomGeom {
            entry_id: "room.c11_dockmaster".into(),
            name: "Dockmaster's Office".into(),
            floor: 1,
            min_x: -10.0,
            min_y: 6.0,
            max_x: 10.0,
            max_y: 22.0,
            kind: "desk",
        },
        RoomGeom {
            entry_id: "room.c05_berths".into(),
            name: "Visiting Berth Gallery".into(),
            floor: 1,
            min_x: -22.0,
            min_y: 22.0,
            max_x: 28.0,
            max_y: 36.0,
            kind: "berth",
        },
    ];

    let mut portals = Vec::new();
    let mut link =
        |a: &str, b: &str, cmd_ab: &str, cmd_ba: &str, x: f64, y: f64, f_a: i64, f_b: i64| {
            let stairs_ab = (f_a != f_b).then_some(f_b);
            let stairs_ba = (f_a != f_b).then_some(f_a);
            portals.push(PortalGeom {
                from_room: a.into(),
                to_room: b.into(),
                command: cmd_ab.into(),
                x_m: x,
                y_m: y,
                floor: f_a,
                width_m: 2.8,
                stairs_to: stairs_ab,
            });
            portals.push(PortalGeom {
                from_room: b.into(),
                to_room: a.into(),
                command: cmd_ba.into(),
                x_m: x,
                y_m: y,
                floor: f_b,
                width_m: 2.8,
                stairs_to: stairs_ba,
            });
        };

    link(
        "room.c03_arrivals",
        "room.c06_concourse",
        "e",
        "w",
        10.0,
        0.0,
        0,
        0,
    );
    link(
        "room.c03_arrivals",
        "room.c08_coop",
        "n",
        "s",
        0.0,
        8.0,
        0,
        0,
    );
    link(
        "room.c03_arrivals",
        "room.c04_freight",
        "d",
        "u",
        -5.0,
        -4.0,
        0,
        -1,
    );
    link(
        "room.c06_concourse",
        "room.c07_exchange",
        "n",
        "s",
        22.0,
        10.0,
        0,
        0,
    );
    link(
        "room.c06_concourse",
        "room.c09_chandlery",
        "e",
        "w",
        34.0,
        0.0,
        0,
        0,
    );
    link(
        "room.c06_concourse",
        "room.c10_canteen",
        "s",
        "n",
        22.0,
        -10.0,
        0,
        0,
    );
    link(
        "room.c03_arrivals",
        "room.c11_dockmaster",
        "u",
        "d",
        5.0,
        6.0,
        0,
        1,
    );
    link(
        "room.c11_dockmaster",
        "room.c05_berths",
        "n",
        "s",
        0.0,
        22.0,
        1,
        1,
    );

    (rooms, portals)
}

fn builtin_ooc_layout() -> (Vec<RoomGeom>, Vec<PortalGeom>) {
    let rooms = vec![
        RoomGeom {
            entry_id: "room.ooc_lobby".into(),
            name: "Simulator Center Lobby".into(),
            floor: 0,
            min_x: -10.0,
            min_y: -8.0,
            max_x: 10.0,
            max_y: 8.0,
            kind: "desk",
        },
        RoomGeom {
            entry_id: "room.ooc_podbay".into(),
            name: "Pod Bay".into(),
            floor: 0,
            min_x: -12.0,
            min_y: 8.0,
            max_x: 12.0,
            max_y: 24.0,
            kind: "pod",
        },
    ];
    let portals = vec![
        PortalGeom {
            from_room: "room.ooc_lobby".into(),
            to_room: "room.ooc_podbay".into(),
            command: "n".into(),
            x_m: 0.0,
            y_m: 8.0,
            floor: 0,
            width_m: 3.0,
            stairs_to: None,
        },
        PortalGeom {
            from_room: "room.ooc_podbay".into(),
            to_room: "room.ooc_lobby".into(),
            command: "s".into(),
            x_m: 0.0,
            y_m: 8.0,
            floor: 0,
            width_m: 3.0,
            stairs_to: None,
        },
    ];
    (rooms, portals)
}

fn builtin_dray_layout() -> (Vec<RoomGeom>, Vec<PortalGeom>) {
    let rooms = vec![
        RoomGeom {
            entry_id: "engineering".into(),
            name: "Engineering".into(),
            floor: -1,
            min_x: -4.5,
            min_y: -12.0,
            max_x: 4.5,
            max_y: -2.0,
            kind: "eng",
        },
        RoomGeom {
            entry_id: "airlock".into(),
            name: "Airlock".into(),
            floor: 0,
            min_x: -3.2,
            min_y: -11.0,
            max_x: 3.2,
            max_y: -5.0,
            kind: "room",
        },
        RoomGeom {
            entry_id: "hold".into(),
            name: "Cargo Hold".into(),
            floor: 0,
            min_x: -5.5,
            min_y: -5.0,
            max_x: 5.5,
            max_y: 7.0,
            kind: "hold",
        },
        RoomGeom {
            entry_id: "mess".into(),
            name: "Mess & Galley".into(),
            floor: 1,
            min_x: -4.5,
            min_y: -5.0,
            max_x: 4.5,
            max_y: 2.0,
            kind: "room",
        },
        RoomGeom {
            entry_id: "quarters".into(),
            name: "Crew Quarters".into(),
            floor: 1,
            min_x: -4.5,
            min_y: -11.0,
            max_x: 4.5,
            max_y: -5.0,
            kind: "room",
        },
        RoomGeom {
            entry_id: "bridge".into(),
            name: "Bridge".into(),
            floor: 1,
            min_x: -4.0,
            min_y: 2.0,
            max_x: 4.0,
            max_y: 9.5,
            kind: "bridge",
        },
    ];

    let mut portals = Vec::new();
    let mut link =
        |a: &str, b: &str, cmd_ab: &str, cmd_ba: &str, x: f64, y: f64, f_a: i64, f_b: i64| {
            portals.push(PortalGeom {
                from_room: a.into(),
                to_room: b.into(),
                command: cmd_ab.into(),
                x_m: x,
                y_m: y,
                floor: f_a,
                width_m: 2.0,
                stairs_to: (f_a != f_b).then_some(f_b),
            });
            portals.push(PortalGeom {
                from_room: b.into(),
                to_room: a.into(),
                command: cmd_ba.into(),
                x_m: x,
                y_m: y,
                floor: f_b,
                width_m: 2.0,
                stairs_to: (f_a != f_b).then_some(f_a),
            });
        };

    link("airlock", "hold", "fore", "aft", 0.0, -5.0, 0, 0);
    link("hold", "engineering", "d", "u", 0.0, -3.5, 0, -1);
    link("hold", "mess", "u", "d", 0.0, 1.0, 0, 1);
    link("mess", "quarters", "aft", "fore", 0.0, -5.0, 1, 1);
    link("mess", "bridge", "fore", "aft", 0.0, 2.0, 1, 1);

    (rooms, portals)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_room(name: &str, entry: &str, tacsim: Option<Value>) -> RoomView {
        let mut amb = std::collections::BTreeMap::new();
        if !entry.is_empty() {
            amb.insert(
                "atlas".into(),
                json!({ "pack": "neighborhood.ceres_freeport", "entry": entry }),
            );
        }
        if let Some(t) = tacsim {
            amb.insert("tacsim".into(), t);
        }
        RoomView {
            id: "#100".into(),
            name: name.to_string(),
            zone: None,
            desc: "A busy deck.".into(),
            contents: Vec::new(),
            exits: Vec::new(),
            ambient: amb,
        }
    }

    #[test]
    fn walking_across_threshold_emits_canonical_exit_command() {
        let mut st = PortWalkState::new();
        let now = Instant::now();
        st.observe_room(
            &sample_room("Port Watch Arrivals", "room.c03_arrivals", None),
            Some("Pilot"),
        );

        // Turn North (heading 0) and walk through the north doorway (y = 8.0) into Registry & Co-op Desk!
        let mut emitted = Vec::new();
        for i in 0..12 {
            let t = now + Duration::from_millis(250 * i);
            let out = st.handle_key(Key::Char('w'), false, &[], t);
            emitted.extend(out.commands);
        }
        assert_eq!(emitted, vec!["n".to_string()]);
    }

    #[test]
    fn one_click_quote_verifies_price_and_emits_accept() {
        let mut st = PortWalkState::new();
        let now = Instant::now();
        let _ = st.observe_port_trade(
            &json!({
                "credits_minor": 100_000,
                "stowage": {
                    "cargo_kg": 0, "max_cargo_kg": 50_000,
                    "cargo_m3": 0.0, "max_cargo_m3": 180.0,
                    "reaction_kg": 8000, "max_reaction_kg": 12_000,
                    "stores_person_days": 14.0, "crew_count": 1
                },
                "asks": [
                    { "listing_id": 42, "good": "water_ice", "quantity": 50, "unit_price_minor": 1200 }
                ],
                "bids": []
            }),
            now,
        );
        st.open_drawer(PortDrawerKind::Trade, &[], &mut Vec::new(), now);
        let buy_out = st.handle_key(Key::Enter, false, &[], now);
        assert_eq!(buy_out.commands, vec!["+market/quote 42 5"]);

        // Matching quote arrives at <= expected price -> auto-emits `+market/accept`!
        let (cmds, _) = st.observe_port_trade(
            &json!({
                "own": {
                    "quotes": [
                        { "quote_id": "Q77", "listing_id": 42, "unit_price_minor": 1200, "expired": false }
                    ]
                }
            }),
            now + Duration::from_millis(200),
        );
        assert_eq!(cmds, vec!["+market/accept Q77"]);
    }

    #[test]
    fn bridge_helm_interaction_triggers_cockpit_handoff() {
        let mut st = PortWalkState::new();
        let now = Instant::now();
        st.observe_room(
            &sample_room(
                "Salt Moth - Bridge",
                "",
                Some(json!({
                    "mode": "interior",
                    "unit": "dray",
                    "room": "bridge",
                    "hull_name": "Salt Moth",
                    "docked": false
                })),
            ),
            Some("Pilot"),
        );
        // Walk forward toward helm couch at y = 8.3 and press `e`
        for i in 0..4 {
            let _ = st.handle_key(
                Key::Char('w'),
                false,
                &[],
                now + Duration::from_millis(50 * i),
            );
        }
        let out = st.handle_key(Key::Char('e'), false, &[], now + Duration::from_millis(300));
        assert!(out.enter_helm_cockpit);
        assert_eq!(out.commands, vec!["+helm/station command"]);
        assert!(st.seated_at_helm());
    }
}
