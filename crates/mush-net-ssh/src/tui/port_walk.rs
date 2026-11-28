//! **Station & Ship Walk Controller and Ratatui Port Services Drawers**
//! (Track 4 Phase 4B: SSH Kitty Graphics & ANSI TUI parity with Unity & Web 3D).
//!
//! Owns the per-connection spatial avatar state (`x_m`, `y_m`, `floor`,
//! `heading_mdeg`), canonical station & multi-deck ship interior layouts
//! (loaded dynamically from `tacsim.content` neighborhood, unit, and market
//! packs with canonical built-in fallbacks), doorway wall-collision +
//! hysteresis + shortest-path BFS exit command dispatch, `[E]` interactable
//! focus & Bridge Helm flight handoff, `port.trade` decoding + 1-click
//! Quote-to-Accept buy state machine, and the interactive Ratatui Port Drawers
//! (`Exchange`, `Jobs`, `Works`, `Sailing`, `Co-op`, `Simulator`, `Talk`).

use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;
use std::time::{Duration, Instant};

use mush_pixel::station_walk::WalkCamera;
use mush_proto::{NarrativeKind, ObjRef, RoomView};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use serde_json::{Value, json};

use super::backend::sanitize_control_sequences as sanitize;
use super::keys::{Arrow, Key};
use super::quests::{
    QuestDestinationHint, QuestMarker, QuestSuggestedAction, QuestSurfaceMiningStatus,
    QuestTradeNeed, infer_station_port_from_room_entry, parse_room_markers,
};

/// Panel/query name for port trade & stowage telemetry.
pub const PORT_TRADE_FEED: &str = "port.trade";
/// Panel/query name for projected tacsim content packs.
pub const TACSIM_CONTENT_FEED: &str = "tacsim.content";

const WALK_SPEED_MPS: f64 = 5.4;
const TURN_MDEG_PER_S: i64 = 135_000;
const HYSTERESIS_M: f64 = 0.85;
const STAIR_TRIGGER_RADIUS_M: f64 = 1.65;
const STAIR_LANDING_CLEARANCE_M: f64 = 2.2;
const MOVE_STEP_COOLDOWN: Duration = Duration::from_millis(220);
const PENDING_MOVE_TIMEOUT: Duration = Duration::from_millis(2400);
const SPEECH_TTL: Duration = Duration::from_millis(8500);
const MAX_SPEECH_BUBBLES: usize = 64;
const MAX_SPEECH_CHARS: usize = 64;
const DRAWER_LOG_CAP: usize = 64;
const HOLOTANK_PULSE_INTERVAL: Duration = Duration::from_millis(500); // ~2 Hz (`SSH-H-01`)
const MAX_PACK_REQUESTS_PER_SESSION: usize = 64;

const QTY_PRESETS: [i64; 5] = [1, 5, 10, 25, 0]; // 0 = MAX FIT

/// Which interactive Port Drawer tab is open.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortDrawerKind {
    Trade,
    Jobs,
    Industry,
    Sailing,
    Coop,
    Sim,
    Npc,
}

impl PortDrawerKind {
    #[must_use]
    pub fn next(self) -> PortDrawerKind {
        match self {
            PortDrawerKind::Trade => PortDrawerKind::Jobs,
            PortDrawerKind::Jobs => PortDrawerKind::Industry,
            PortDrawerKind::Industry => PortDrawerKind::Sailing,
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
            PortDrawerKind::Jobs => PortDrawerKind::Trade,
            PortDrawerKind::Industry => PortDrawerKind::Jobs,
            PortDrawerKind::Sailing => PortDrawerKind::Industry,
            PortDrawerKind::Coop => PortDrawerKind::Sailing,
            PortDrawerKind::Sim => PortDrawerKind::Coop,
            PortDrawerKind::Npc => PortDrawerKind::Sim,
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            PortDrawerKind::Trade => "Trade",
            PortDrawerKind::Jobs => "Jobs",
            PortDrawerKind::Industry => "Works",
            PortDrawerKind::Sailing => "Sail",
            PortDrawerKind::Coop => "Coop",
            PortDrawerKind::Sim => "Sim",
            PortDrawerKind::Npc => "Talk",
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
    /// When `[E]` targets an NPC with an `available` or `turn_in` quest marker,
    /// hands off directly to the TUI Quest Dialog (`(npc_id, npc_name)`).
    pub open_quest_for_npc: Option<(String, String)>,
}

/// Active quest stowage/hold readiness computed against live ship tanks, stores, or cargo lots.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestStowageReadiness {
    pub commodity_slug: String,
    pub label: String,
    pub current_qty: i64,
    pub target_qty: i64,
    pub unit: &'static str,
    pub ready: bool,
    pub gauge_kind: &'static str, // "propellant" | "provisions" | "cargo"
}

/// Live Step 5 Dray-P refit empty-hold readiness (cargo & crew stores must be empty; propellant tank may remain).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestRefitHoldReadiness {
    pub cargo_empty: bool,
    pub provisions_empty: bool,
    pub cargo_kg: i64,
    pub provisions_days: i64,
    pub propellant_tonnes: i64,
    pub ready: bool,
}

/// Live Sailing Plan departure pre-flight readiness for an active destination quest
/// (checks propellant tank, crew stores, and any required non-tank quest cargo in the hold).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestSailingPreflightReadiness {
    pub propellant_tonnes: i64,
    pub propellant_ok: bool,
    pub provisions_days: i64,
    pub provisions_ok: bool,
    pub cargo_required: bool,
    pub cargo_label: String,
    pub cargo_qty: i64,
    pub cargo_target: i64,
    pub cargo_ok: bool,
    pub ready: bool,
}

/// Immediate next-hop station exit breadcrumb toward the tracked quest's objective room.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestExitBreadcrumb {
    pub next_room_entry: String,
    pub next_room_name: String,
    pub next_exit_command: String,
    pub target_room_name: String,
    pub hops: usize,
}

impl QuestExitBreadcrumb {
    /// True when a `RoomView` exit (by name or alias) matches the first hop of this route.
    #[must_use]
    pub fn matches_exit(&self, exit_name: &str, aliases: &[String]) -> bool {
        let want_cmd = self.next_exit_command.trim().to_ascii_lowercase();
        let want_word = want_cmd.strip_prefix("goto ").unwrap_or(&want_cmd).trim();
        let want_room = self.next_room_name.trim().to_ascii_lowercase();
        let name_norm = exit_name.trim().to_ascii_lowercase();
        if name_norm.is_empty() {
            return false;
        }
        if name_norm == want_word
            || name_norm == want_room
            || format!("goto {name_norm}") == want_cmd
        {
            return true;
        }
        aliases.iter().any(|raw| {
            let a = raw.trim().to_ascii_lowercase();
            !a.is_empty() && (a == want_word || a == want_room || format!("goto {a}") == want_cmd)
        })
    }
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
        let half_w = (self.max_x - self.min_x) * 0.5;
        let half_h = (self.max_y - self.min_y) * 0.5;
        let mx = margin.min((half_w - 0.1).max(0.0));
        let my = margin.min((half_h - 0.1).max(0.0));
        x >= self.min_x + mx && x <= self.max_x - mx && y >= self.min_y + my && y <= self.max_y - my
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
    /// True when the portal connects two non-adjacent rooms (e.g. OOC `welcome_hall -> simulator_lobby`),
    /// so walking onto the doorway transitions directly to the target room's landing point.
    teleport: bool,
}

#[derive(Clone, Debug)]
struct InteractableTarget {
    label: String,
    x_m: f64,
    y_m: f64,
    floor: i64,
    action: InteractAction,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum InteractAction {
    OpenDrawer(PortDrawerKind),
    OpenNpc { id: String, name: String },
    Command(&'static str),
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

#[derive(Clone, Debug)]
struct DockedHullInfo {
    id: String,
    name: String,
    unit: String,
}

#[derive(Clone, Debug, Default)]
struct SimulatorSlotInfo {
    team: String,
    index: u64,
    pilot_name: String,
    unit: String,
    seated: bool,
}

#[derive(Clone, Debug, Default)]
struct SimulatorState {
    arena: String,
    match_state: String,
    my_team: Option<String>,
    my_unit: Option<String>,
    slots: Vec<SimulatorSlotInfo>,
}

#[derive(Clone, Debug, Default)]
pub struct StowageState {
    pub cargo_kg: i64,
    pub max_cargo_kg: i64,
    pub cargo_m3: f64,
    pub max_cargo_m3: f64,
    pub reaction_kg: i64,
    pub max_reaction_kg: i64,
    /// Crew-days of provisions aboard (`stores_days`).
    pub stores_days: i64,
    /// The provisions compartment's limit in crew-days (`stores_limit_days`).
    pub max_stores_days: i64,
}

#[derive(Clone, Debug)]
pub struct AskRow {
    pub listing_id: i64,
    pub good_id: String,
    pub good_name: String,
    pub vendor_name: String,
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
    pub vendor_name: String,
    pub quantity_wanted: i64,
    pub unit_price_minor: i64,
    pub funded: bool,
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
    authorized_total_minor: i64,
    prior_quote_ids: HashSet<String>,
    created_at: Instant,
}

#[derive(Clone, Debug)]
struct OwnQuoteRow {
    quote_id: String,
    listing_id: i64,
    quantity: i64,
    unit_price_minor: i64,
    total_minor: Option<i64>,
    expired: bool,
}

#[derive(Clone, Debug)]
struct PendingInteraction {
    action: InteractAction,
    target_room: String,
    manifest_panels: Vec<String>,
    created_at: Instant,
}

/// The lane's venue words (48e L8, clients:G7 / `H-40`): every `port` entry's
/// `venue` in `crates/tacsim/content/lane/ceres_phobos.json`, with its
/// location's label. `+ship/undock`, `+ship/plan` and `+ship/order depart` take
/// exactly these; `vesta`, `pallas`, `hygiea` and `juno` name no venue and were
/// refused.
const CORRIDORS: [(&str, &str, &str); 13] = [
    ("ceres", "Ceres Freeport", "Ceres port"),
    ("stickney", "Stickney Yards", "Phobos port"),
    ("luna", "Aitken Highport", "Luna port"),
    ("deimos", "Voltaire Anchorage", "Deimos port"),
    ("titan", "Ligeia Terminal", "Titan port"),
    ("commons", "Freeport Commons", "Ceres extraction site"),
    ("kerwan", "Kerwan Floor", "Ceres extraction site"),
    ("ahuna", "Ahuna Flank", "Ceres extraction site"),
    ("juling", "Juling Shadow", "Ceres extraction site"),
    ("occator", "Occator Rim", "Ceres extraction site"),
    ("rheasilvia", "Rheasilvia Commons", "Vesta extraction site"),
    ("urvara", "Urvara Reach", "Ceres extraction site"),
    // Doc 50 cut OP.1: the Isidis Concession, a ground port of the lane.
    ("isidis", "Isidis Concession", "Mars port"),
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

const DOC48_JOB_ACTIONS: [(&str, &str); 9] = [
    ("All Open Port Jobs", "+market/jobs"),
    ("Haulage Contracts", "+market/jobs haul"),
    ("Armed Escort Contracts", "+market/jobs escort"),
    ("Survey Commissions", "+market/jobs survey"),
    ("Salvage Retrieval Jobs", "+market/jobs retrieval"),
    ("Crew Terms In Room", "+sortie/crew"),
    ("Active Crew Agreements", "+sortie/contract"),
    ("Port Locker Status", "+locker"),
    ("Arrears & Obligations", "+debts"),
];

const DOC48_INDUSTRY_ACTIONS: [(&str, &str); 10] = [
    ("Port Works & Orders", "+works"),
    ("Shipyard Queue & Orders", "+yard"),
    (
        "24-Week Volatiles Trend",
        "+market/history tacsim:volatiles",
    ),
    ("24-Week Metals Trend", "+market/history tacsim:metals"),
    (
        "24-Week Alloys Trend",
        "+market/history tacsim:structural_alloys",
    ),
    ("Quote Metal Ore Milling", "+works/quote metals"),
    ("Quote 30mm AP Munitions", "+works/quote ap_30"),
    ("Quote Dray Hull Build", "+yard/quote dray"),
    ("Core Fitting Counter", "+core"),
    ("Titled Facilities List", "@facility/list"),
];

/// Per-connection spatial walk + port services state.
pub struct PortWalkState {
    camera: WalkCamera,
    pack_id: String,
    ship_unit_id: String,
    rooms: Vec<RoomGeom>,
    portals: Vec<PortalGeom>,
    ship_rooms: Vec<RoomGeom>,
    ship_portals: Vec<PortalGeom>,
    neighborhood_cache: HashMap<String, (Vec<RoomGeom>, Vec<PortalGeom>)>,
    unit_cache: HashMap<String, (Vec<RoomGeom>, Vec<PortalGeom>)>,
    manifest_packs: HashMap<String, String>,
    manifest_gen: u64,
    in_flight_packs: HashSet<String>,
    loaded_packs: HashMap<String, String>,
    missing_packs: HashMap<String, u64>,
    total_pack_requests: usize,
    pending_pack_requests: Vec<String>,
    pending_followup_queries: Vec<(String, Value)>,

    aboard_ship: bool,
    seated_at_helm: bool,
    hull_name: String,
    own_hull_id: Option<String>,
    own_hull_name: String,
    own_hull_docked_here: bool,
    berth_docked_hulls: Vec<DockedHullInfo>,
    simulator: Option<SimulatorState>,
    docked: bool,
    settled_pad: bool,
    docked_port: Option<String>,
    server_location_key: String,
    active_location_key: String,
    room_title: String,

    you_x: f64,
    you_y: f64,
    you_floor: i64,
    you_heading_mdeg: i64,
    you_stride: i64,
    pulse_phase: i64,
    last_pulse_at: Option<Instant>,
    scene_gen: u64,

    held_fwd: i8,
    held_strafe: i8,
    held_turn: i8,
    last_tick: Option<Instant>,
    last_step_at: Option<Instant>,
    pending_move_target: Option<(String, Instant)>,
    next_move_allowed_at: Option<Instant>,
    pending_interaction: Option<PendingInteraction>,
    /// Prevents instant floor-0 <-> floor-1 ping-pong right after traversing a stair/warp portal.
    portal_rearm: Option<(f64, f64, i64)>,
    pending_orders: Vec<String>,
    last_room_view: Option<RoomView>,
    last_self_name: Option<String>,

    actors: Vec<ActorEntry>,
    quest_markers: Vec<QuestMarker>,
    quest_trade_need: Option<QuestTradeNeed>,
    quest_trade_needs: Vec<QuestTradeNeed>,
    quest_dest_port: Option<QuestDestinationHint>,
    quest_room_entry: Option<String>,
    quest_room_turn_in: bool,
    quest_ship_target: Option<String>,
    quest_coop_action: Option<QuestSuggestedAction>,
    quest_mining_status: Option<QuestSurfaceMiningStatus>,
    quest_step_badge: String,
    speech_bubbles: HashMap<String, (String, Instant)>,
    interactables: Vec<InteractableTarget>,
    focus_idx: usize,

    // Trade & Goods
    goods_meta: HashMap<String, (String, i64, i64)>, // id -> (label, mass_kg, vol_l)
    credits_minor: i64,
    balance_text: String,
    current_venue: Option<String>,
    port_name: String,
    stowage: StowageState,
    stowage_loaded: bool,
    asks: Vec<AskRow>,
    bids: Vec<BidRow>,
    trade_next_after: Option<u64>,
    awaiting_trade_page: bool,
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
    custom_trade_qty: Option<i64>,
    corridor_idx: usize,
    queue_depart: bool,
    hull_suggestion_idx: usize,
    custom_hull_input: String,
    editing_hull_name: bool,
    selected_npc_name: String,
    drawer_log: VecDeque<String>,
    drawer_log_scroll: usize,
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
        let mut st = PortWalkState {
            camera: WalkCamera::Cutaway25D,
            pack_id: "neighborhood.ceres_freeport".to_string(),
            ship_unit_id: "dray".to_string(),
            rooms,
            portals,
            ship_rooms,
            ship_portals,
            neighborhood_cache: HashMap::new(),
            unit_cache: HashMap::new(),
            manifest_packs: HashMap::new(),
            manifest_gen: 0,
            in_flight_packs: HashSet::new(),
            loaded_packs: HashMap::new(),
            missing_packs: HashMap::new(),
            total_pack_requests: 0,
            pending_pack_requests: Vec::new(),
            pending_followup_queries: Vec::new(),
            aboard_ship: false,
            seated_at_helm: false,
            hull_name: String::new(),
            own_hull_id: None,
            own_hull_name: String::new(),
            own_hull_docked_here: false,
            berth_docked_hulls: Vec::new(),
            simulator: None,
            docked: true,
            settled_pad: false,
            docked_port: None,
            server_location_key: "room.c03_arrivals".to_string(),
            active_location_key: "room.c03_arrivals".to_string(),
            room_title: "Port Watch Arrivals".to_string(),
            you_x: 125.0,
            you_y: 152.5,
            you_floor: 1,
            you_heading_mdeg: 0,
            you_stride: 0,
            pulse_phase: 0,
            last_pulse_at: None,
            scene_gen: 1,
            held_fwd: 0,
            held_strafe: 0,
            held_turn: 0,
            last_tick: None,
            last_step_at: None,
            pending_move_target: None,
            next_move_allowed_at: None,
            pending_interaction: None,
            portal_rearm: None,
            pending_orders: Vec::new(),
            last_room_view: None,
            last_self_name: None,
            actors: Vec::new(),
            quest_markers: Vec::new(),
            quest_trade_need: None,
            quest_trade_needs: Vec::new(),
            quest_dest_port: None,
            quest_room_entry: None,
            quest_room_turn_in: false,
            quest_ship_target: None,
            quest_coop_action: None,
            quest_mining_status: None,
            quest_step_badge: String::new(),
            speech_bubbles: HashMap::new(),
            interactables: Vec::new(),
            focus_idx: 0,
            goods_meta: default_goods_meta(),
            credits_minor: 0,
            balance_text: String::new(),
            current_venue: None,
            port_name: String::new(),
            stowage: StowageState::default(),
            stowage_loaded: false,
            asks: Vec::new(),
            bids: Vec::new(),
            trade_next_after: None,
            awaiting_trade_page: false,
            cargo_lots: Vec::new(),
            own_quotes: Vec::new(),
            pending_quote: None,
            content_asked: false,
            trade_asked_at: None,
            drawer: None,
            trade_subtab: 0,
            cursor_row: 0,
            qty_preset_idx: 1, // default 5 units
            custom_trade_qty: None,
            corridor_idx: 0,
            queue_depart: true,
            hull_suggestion_idx: 0,
            custom_hull_input: HULL_SUGGESTIONS
                .first()
                .copied()
                .unwrap_or("Salt Moth")
                .to_string(),
            editing_hull_name: false,
            selected_npc_name: String::new(),
            drawer_log: VecDeque::new(),
            drawer_log_scroll: 0,
        };
        st.snap_to_room("room.c03_arrivals");
        st
    }

    fn bump_scene(&mut self) {
        self.scene_gen = self.scene_gen.wrapping_add(1);
    }

    /// Monotonic generation counter incremented only when the rendered walk scene changes (`SSH-H-01`).
    #[must_use]
    pub fn scene_generation(&self) -> u64 {
        self.scene_gen
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

    /// Cancel active free-text hull-name editing (e.g. when `Ctrl+G` toggles pixel view, `SSH-M-06`).
    pub fn cancel_hull_edit(&mut self) {
        self.editing_hull_name = false;
    }

    #[must_use]
    pub fn stowage(&self) -> &StowageState {
        &self.stowage
    }

    #[must_use]
    pub fn has_stowage_loaded(&self) -> bool {
        self.stowage_loaded
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

    /// Drain any pending on-demand `tacsim.content` pack requests queued when switching neighborhoods/hulls,
    /// plus any follow-up queries scheduled when a queued interaction resumes after room acknowledgement.
    pub fn drain_pack_queries(&mut self, manifest_panels: &[String]) -> Vec<(String, Value)> {
        let mut out = Vec::new();
        for (feed, args) in std::mem::take(&mut self.pending_followup_queries) {
            if manifest_panels.is_empty() || manifest_panels.iter().any(|p| p == &feed) {
                out.push((feed, args));
            }
        }
        if !manifest_panels.iter().any(|p| p == TACSIM_CONTENT_FEED) {
            self.pending_pack_requests.clear();
            return out;
        }
        for pack_id in std::mem::take(&mut self.pending_pack_requests) {
            if self.try_enqueue_pack(&pack_id) {
                out.push((
                    TACSIM_CONTENT_FEED.to_string(),
                    json!({ "op": "pack", "id": pack_id }),
                ));
            }
        }
        out
    }

    fn is_pack_loaded_current(&self, id: &str) -> bool {
        let Some(loaded_sha) = self.loaded_packs.get(id) else {
            return false;
        };
        if let Some(manifest_sha) = self.manifest_packs.get(id)
            && !manifest_sha.is_empty()
            && !loaded_sha.is_empty()
        {
            return loaded_sha == manifest_sha;
        }
        true
    }

    fn try_enqueue_pack(&mut self, id: &str) -> bool {
        if id.is_empty()
            || self.is_pack_loaded_current(id)
            || self.in_flight_packs.contains(id)
            || self.missing_packs.get(id) == Some(&self.manifest_gen)
            || self.total_pack_requests >= MAX_PACK_REQUESTS_PER_SESSION
        {
            return false;
        }
        self.in_flight_packs.insert(id.to_string());
        self.total_pack_requests = self.total_pack_requests.saturating_add(1);
        true
    }

    fn reset_stowage(&mut self) {
        self.stowage = StowageState::default();
        self.stowage_loaded = false;
        self.cargo_lots.clear();
    }

    /// Tick using internal `last_tick` timestamp and queue any emitted movement commands.
    /// Returns `true` ONLY when the visual scene actually changed (`SSH-H-01`).
    pub fn tick_at(&mut self, now: Instant) -> bool {
        let dt = self.last_tick.map_or(Duration::from_millis(50), |t| {
            now.saturating_duration_since(t)
        });
        self.last_tick = Some(now);
        let before = self.scene_gen;
        let cmds = self.tick(dt, now);
        self.pending_orders.extend(cmds);
        self.scene_gen != before
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
            out.push((TACSIM_CONTENT_FEED.to_string(), json!({ "op": "manifest" })));
        }
        out.extend(self.drain_pack_queries(manifest_panels));
        let has_trade = manifest_panels.iter().any(|p| p == PORT_TRADE_FEED);
        let trade_due = self
            .trade_asked_at
            .is_none_or(|at| now.saturating_duration_since(at) >= Duration::from_secs(3));
        if has_trade && trade_due {
            self.trade_asked_at = Some(now);
            self.awaiting_trade_page = false;
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
            self.close_drawer();
            return;
        }
        let key = self.active_location_key.as_str();
        let default_kind = if self.pack_id.contains("ooc")
            || key.contains("simulator")
            || key.contains("sim_red")
            || key.contains("sim_blue")
        {
            PortDrawerKind::Sim
        } else if key.contains("coop") || key.contains("registry") {
            PortDrawerKind::Coop
        } else if key.contains("dockmaster")
            || key.contains("harbourmaster")
            || key.contains("berth")
            || key == "bridge"
        {
            PortDrawerKind::Sailing
        } else if key.contains("works")
            || key.contains("yard")
            || key.contains("service")
            || key.contains("clearing")
            || key.contains("tycho")
            || key.contains("receiving")
            || key.contains("maintenance")
        {
            PortDrawerKind::Industry
        } else if key.contains("slack")
            || key.contains("shift_board")
            || key.contains("arrivals")
            || key.contains("customs")
        {
            PortDrawerKind::Jobs
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
        self.drawer_log_scroll = 0;
        self.editing_hull_name = false;
        if kind == PortDrawerKind::Sailing {
            self.apply_quest_destination_jump();
        } else if kind == PortDrawerKind::Trade
            && let Some(need) = self.effective_quest_trade_need()
            && !need.wants_buy
            && need.wants_sell
            && !self.bids.is_empty()
        {
            self.trade_subtab = 1;
            if let Some(idx) = self
                .bids
                .iter()
                .position(|b| need.matches_listing(&b.good_id, &b.good_name, true))
            {
                self.cursor_row = idx;
            }
        }
        self.ensure_queries(manifest_panels, queries, now);
    }

    pub fn close_drawer(&mut self) {
        self.drawer = None;
        self.editing_hull_name = false;
        self.drawer_log_scroll = 0;
    }

    /// Sync live quest markers from `"tacsim.quests"` onto 2.5D NPC actors & `[E]` focus labels.
    pub fn observe_quest_markers(&mut self, markers: &[QuestMarker]) {
        if self.quest_markers == markers {
            return;
        }
        self.quest_markers = markers.to_vec();
        self.refresh_npc_quest_labels();
        self.bump_scene();
    }

    /// Sync active quest trade & corridor destination hints for the Port Trade and Sailing drawers.
    pub fn observe_quest_guidance(
        &mut self,
        trade_need: Option<QuestTradeNeed>,
        dest_port: Option<QuestDestinationHint>,
    ) {
        self.quest_trade_need = trade_need.clone();
        self.quest_trade_needs = trade_need.into_iter().collect();
        self.quest_dest_port = dest_port;
        self.sort_trade_listings_for_quest();
    }

    /// Sync all pending trade needs on the tracked quest (e.g. Step 2 provisions + propellant).
    pub fn observe_quest_trade_needs(&mut self, needs: Vec<QuestTradeNeed>) {
        self.quest_trade_need = needs.first().cloned();
        self.quest_trade_needs = needs;
        self.sort_trade_listings_for_quest();
    }

    /// Stably float quest-matching asks and bids to the top of the Port Trade drawer lists.
    fn sort_trade_listings_for_quest(&mut self) {
        let Some(need) = self.effective_quest_trade_need() else {
            return;
        };
        self.asks
            .sort_by_key(|a| !need.matches_listing(&a.good_id, &a.good_name, false));
        self.bids
            .sort_by_key(|b| !need.matches_listing(&b.good_id, &b.good_name, true));
    }

    /// Sync the tracked active quest's next station room target onto the 2.5D station cutaway feed.
    pub fn observe_quest_objective_room(&mut self, objective: Option<(String, bool)>) {
        let (entry, turn_in) = match objective {
            Some((room_entry, is_turn_in)) => (Some(room_entry), is_turn_in),
            None => (None, false),
        };
        if self.quest_room_entry == entry && self.quest_room_turn_in == turn_in {
            return;
        }
        self.quest_room_entry = entry;
        self.quest_room_turn_in = turn_in;
        self.bump_scene();
    }

    /// Sync active Co-op enrollment / Dray-P refit quest action for the Co-op & Works drawers.
    pub fn observe_quest_coop_action(&mut self, action: Option<QuestSuggestedAction>) {
        self.quest_coop_action = action;
    }

    /// Sync active surface prospecting quest status for Vehicle Bay HUD guidance.
    pub fn observe_quest_mining_status(&mut self, status: Option<QuestSurfaceMiningStatus>) {
        self.quest_mining_status = status;
        self.refresh_npc_quest_labels();
    }

    /// Sync active tracked guided quest step badge (e.g., `" [Step 3/7]"`).
    pub fn observe_quest_step_badge(&mut self, badge: String) {
        self.quest_step_badge = badge;
    }

    /// Active quest corridor destination hint (e.g., Stickney Yards), if any.
    #[must_use]
    pub fn quest_dest_hint(&self) -> Option<&QuestDestinationHint> {
        self.quest_dest_port.as_ref()
    }

    fn quest_marker_priority(m: &QuestMarker) -> u8 {
        match m.marker.as_str() {
            "turn_in" => 0,
            "available" => 1,
            _ => 2,
        }
    }

    /// Resolve the highest-priority room quest marker (`turn_in` -> `available` -> `in_progress`)
    /// matching the currently selected NPC in the Talk drawer.
    #[must_use]
    pub fn selected_npc_quest_marker(&self) -> Option<&QuestMarker> {
        let sel = self.selected_npc_name.trim();
        if !sel.is_empty() {
            if let Some(m) = self
                .quest_markers
                .iter()
                .filter(|m| m.npc_name.trim().eq_ignore_ascii_case(sel))
                .min_by_key(|m| Self::quest_marker_priority(m))
            {
                return Some(m);
            }
            let sel_low = sel.to_ascii_lowercase();
            return self
                .quest_markers
                .iter()
                .filter(|m| {
                    let n = m.npc_name.trim().to_ascii_lowercase();
                    !n.is_empty() && (n.contains(&sel_low) || sel_low.contains(&n))
                })
                .min_by_key(|m| Self::quest_marker_priority(m));
        }
        if self.quest_markers.len() == 1 {
            return self.quest_markers.first();
        }
        None
    }

    /// Count distinct quests on the currently selected NPC in the Talk drawer.
    #[must_use]
    pub fn selected_npc_quest_count(&self) -> usize {
        let sel = self.selected_npc_name.trim();
        let mut keys: Vec<String> = Vec::new();
        let push_unique = |keys: &mut Vec<String>, m: &QuestMarker| {
            let k = if m.quest_key.is_empty() {
                format!("{}:{}", m.marker, m.quest_title)
            } else {
                m.quest_key.clone()
            };
            if !keys.contains(&k) {
                keys.push(k);
            }
        };
        if !sel.is_empty() {
            let mut exact_hit = false;
            for m in &self.quest_markers {
                if m.npc_name.trim().eq_ignore_ascii_case(sel) {
                    exact_hit = true;
                    push_unique(&mut keys, m);
                }
            }
            if exact_hit {
                return keys.len();
            }
            let sel_low = sel.to_ascii_lowercase();
            for m in &self.quest_markers {
                let n = m.npc_name.trim().to_ascii_lowercase();
                if !n.is_empty() && (n.contains(&sel_low) || sel_low.contains(&n)) {
                    push_unique(&mut keys, m);
                }
            }
            return keys.len();
        }
        usize::from(self.quest_markers.len() == 1)
    }

    fn stowage_readiness_for(&self, need: &QuestTradeNeed) -> Option<QuestStowageReadiness> {
        if !self.stowage_loaded {
            return None;
        }
        let target_qty = need.target_quantity.max(1);
        let label = need.commodity_slug.replace('_', " ").to_ascii_uppercase();
        if need.commodity_slug == "propellant" {
            let current_qty = self.stowage.reaction_kg.max(0) / 1000;
            return Some(QuestStowageReadiness {
                commodity_slug: need.commodity_slug.clone(),
                label,
                current_qty,
                target_qty,
                unit: "t",
                ready: current_qty >= target_qty,
                gauge_kind: "propellant",
            });
        }
        if need.commodity_slug == "provisions" {
            let current_qty = self.stowage.stores_days.max(0);
            return Some(QuestStowageReadiness {
                commodity_slug: need.commodity_slug.clone(),
                label,
                current_qty,
                target_qty,
                unit: "d",
                ready: current_qty >= target_qty,
                gauge_kind: "provisions",
            });
        }
        let current_qty = self.hold_commodity_quantity(&need.commodity_slug);
        Some(QuestStowageReadiness {
            commodity_slug: need.commodity_slug.clone(),
            label,
            current_qty,
            target_qty,
            unit: "u",
            ready: current_qty >= target_qty,
            gauge_kind: "cargo",
        })
    }

    /// Total live quantity of a normalized commodity slug currently stowed in ship cargo lots.
    #[must_use]
    pub fn hold_commodity_quantity(&self, commodity_slug: &str) -> i64 {
        let norm = |raw: &str| -> String {
            let trimmed = raw.trim().to_ascii_lowercase();
            let tail = trimmed
                .rsplit_once(':')
                .map_or(trimmed.as_str(), |(_, r)| r);
            tail.replace(['-', ' '], "_")
        };
        let want = norm(commodity_slug);
        if want.is_empty() {
            return 0;
        }
        let mut total = 0i64;
        for lot in &self.cargo_lots {
            if norm(&lot.good_id) == want || norm(&lot.good_name) == want {
                total += lot.quantity.max(0);
            }
        }
        total
    }

    /// Resolve the most relevant active quest trade need for the pilot's live stowage & venue book:
    /// - Advances Step 2 past buy-only items already satisfied in live stowage and prefers an
    ///   unsatisfied item present on the current venue's book.
    /// - Narrows dual buy+sell delivery quests (Steps 3 & 4) to SELL-only once the required cargo
    ///   is loaded in the ship's hold.
    #[must_use]
    pub fn effective_quest_trade_need(&self) -> Option<QuestTradeNeed> {
        let needs: &[QuestTradeNeed] = if !self.quest_trade_needs.is_empty() {
            &self.quest_trade_needs
        } else {
            std::slice::from_ref(self.quest_trade_need.as_ref()?)
        };

        let is_buy_satisfied = |n: &QuestTradeNeed| -> bool {
            n.wants_buy && !n.wants_sell && self.stowage_readiness_for(n).is_some_and(|r| r.ready)
        };
        let unsatisfied: Vec<&QuestTradeNeed> =
            needs.iter().filter(|n| !is_buy_satisfied(n)).collect();
        let pool: Vec<&QuestTradeNeed> = if !unsatisfied.is_empty() {
            unsatisfied
        } else {
            needs.iter().collect()
        };

        let matches_book = |n: &QuestTradeNeed| -> bool {
            self.asks
                .iter()
                .any(|a| n.matches_listing(&a.good_id, &a.good_name, false))
                || self
                    .bids
                    .iter()
                    .any(|b| n.matches_listing(&b.good_id, &b.good_name, true))
        };

        let mut picked = if !self.asks.is_empty() || !self.bids.is_empty() {
            pool.iter()
                .copied()
                .find(|n| matches_book(n))
                .unwrap_or(pool[0])
                .clone()
        } else {
            pool[0].clone()
        };

        if picked.wants_buy
            && picked.wants_sell
            && self.stowage_readiness_for(&picked).is_some_and(|r| r.ready)
        {
            picked.wants_buy = false;
            picked.wants_sell = true;
        }
        Some(picked)
    }

    /// Evaluate active quest commodity readiness against live ship tanks, crew stores, or hold cargo lots.
    #[must_use]
    pub fn quest_stowage_readiness(&self) -> Option<QuestStowageReadiness> {
        let need = self.effective_quest_trade_need()?;
        self.stowage_readiness_for(&need)
    }

    /// Returns the quest destination port when the active trade need is SELL-only, already stowed
    /// in the ship's hold, and the pilot is currently at a different port (cross-port haul guard).
    #[must_use]
    pub fn quest_trade_ready_to_sail_dest(&self) -> Option<&QuestDestinationHint> {
        let need = self.effective_quest_trade_need()?;
        if need.wants_buy || !need.wants_sell {
            return None;
        }
        if !self.stowage_readiness_for(&need).is_some_and(|r| r.ready) {
            return None;
        }
        let dest = self.quest_dest_port.as_ref()?;
        let cur = self.current_port_slug()?;
        if cur.eq_ignore_ascii_case(&dest.port_id) {
            return None;
        }
        Some(dest)
    }

    /// Live stowage telemetry for F11 Quest Overlay trade & Step 5 refit readiness lines.
    #[must_use]
    pub fn quest_stowage_telemetry(&self) -> Option<super::quests::QuestStowageTelemetry> {
        if !self.stowage_loaded {
            return None;
        }
        Some(super::quests::QuestStowageTelemetry {
            cargo_kg: self.stowage.cargo_kg.max(0),
            propellant_kg: self.stowage.reaction_kg.max(0),
            provisions_days: self.stowage.stores_days.max(0),
            cargo_lots: self
                .cargo_lots
                .iter()
                .map(|l| (l.good_id.clone(), l.good_name.clone(), l.quantity))
                .collect(),
        })
    }

    /// Evaluate Step 5 Dray-P refit empty-hold readiness (cargo & crew stores must be empty; propellant tank OK).
    #[must_use]
    pub fn quest_refit_hold_readiness(&self) -> Option<QuestRefitHoldReadiness> {
        if !self.stowage_loaded {
            return None;
        }
        let act = self.quest_coop_action.as_ref()?;
        if !act.command.starts_with("+coop/refit") {
            return None;
        }
        let cargo_kg = self.stowage.cargo_kg.max(0);
        let has_cargo_lots = self.cargo_lots.iter().any(|l| l.quantity > 0);
        let cargo_empty = cargo_kg == 0 && !has_cargo_lots;
        let provisions_days = self.stowage.stores_days.max(0);
        let provisions_empty = provisions_days == 0;
        let propellant_tonnes = self.stowage.reaction_kg.max(0) / 1000;
        Some(QuestRefitHoldReadiness {
            cargo_empty,
            provisions_empty,
            cargo_kg,
            provisions_days,
            propellant_tonnes,
            ready: cargo_empty && provisions_empty,
        })
    }

    /// True when Step 5 Dray-P refit requires clearing the hold and `bid` buys a stowed cargo
    /// commodity or leftover provisions that must be unloaded before `+coop/refit prospector`.
    #[must_use]
    pub fn is_refit_clear_bid(&self, bid: &BidRow) -> bool {
        let Some(refit) = self.quest_refit_hold_readiness() else {
            return false;
        };
        if refit.ready {
            return false;
        }
        let gid = super::quests::normalize_good_slug(&bid.good_id);
        let gname = super::quests::normalize_good_slug(&bid.good_name);
        if !refit.provisions_empty && (gid == "provisions" || gname == "provisions") {
            return true;
        }
        if !refit.cargo_empty {
            return self.cargo_lots.iter().any(|l| {
                if l.quantity <= 0 {
                    return false;
                }
                let lg = super::quests::normalize_good_slug(&l.good_id);
                let ln = super::quests::normalize_good_slug(&l.good_name);
                (!gid.is_empty() && (lg == gid || ln == gid))
                    || (!gname.is_empty() && (lg == gname || ln == gname))
            });
        }
        false
    }

    /// Evaluate Sailing Plan departure pre-flight readiness for an active destination quest
    /// (propellant tank, crew stores, and any required non-tank quest cargo in the hold).
    #[must_use]
    pub fn quest_sailing_preflight_readiness(&self) -> Option<QuestSailingPreflightReadiness> {
        if !self.stowage_loaded || self.quest_dest_port.is_none() {
            return None;
        }
        let propellant_tonnes = self.stowage.reaction_kg.max(0) / 1000;
        let propellant_ok = self.stowage.reaction_kg > 0;
        let provisions_days = self.stowage.stores_days.max(0);
        let provisions_ok = provisions_days > 0;

        let mut cargo_required = false;
        let mut cargo_label = String::new();
        let mut cargo_qty = 0i64;
        let mut cargo_target = 0i64;
        let mut cargo_ok = true;
        if let Some(need) = self.quest_trade_need.as_ref()
            && need.commodity_slug != "propellant"
            && need.commodity_slug != "provisions"
        {
            cargo_required = true;
            cargo_label = need.commodity_slug.replace('_', " ");
            cargo_target = need.target_quantity.max(1);
            let norm = |raw: &str| -> String {
                let trimmed = raw.trim().to_ascii_lowercase();
                let tail = trimmed
                    .rsplit_once(':')
                    .map_or(trimmed.as_str(), |(_, r)| r);
                tail.replace(['-', ' '], "_")
            };
            for lot in &self.cargo_lots {
                if norm(&lot.good_id) == need.commodity_slug
                    || norm(&lot.good_name) == need.commodity_slug
                {
                    cargo_qty += lot.quantity.max(0);
                }
            }
            cargo_ok = cargo_qty >= cargo_target;
        }

        Some(QuestSailingPreflightReadiness {
            propellant_tonnes,
            propellant_ok,
            provisions_days,
            provisions_ok,
            cargo_required,
            cargo_label,
            cargo_qty,
            cargo_target,
            cargo_ok,
            ready: propellant_ok && provisions_ok && cargo_ok,
        })
    }

    /// True when the tracked quest is a cross-port buy+deliver trade run whose required cargo
    /// is not yet loaded in live ship stowage, so wayfinding should route to the local Exchange
    /// before heading to Berth Gallery.
    #[must_use]
    pub fn needs_origin_port_procurement(&self) -> bool {
        if !self.stowage_loaded || self.quest_room_turn_in {
            return false;
        }
        let Some(need) = self.quest_trade_need.as_ref() else {
            return false;
        };
        if !need.wants_buy || !need.wants_sell {
            return false;
        }
        if need.commodity_slug == "propellant" || need.commodity_slug == "provisions" {
            return false;
        }
        self.quest_stowage_readiness().is_some_and(|r| !r.ready)
    }

    /// Normalized corridor port slug while aboard a station-docked ship (e.g. `"ceres"`, `"stickney"`).
    /// Returns `None` while settled on a planetary surface landing pad.
    #[must_use]
    pub fn docked_port_slug(&self) -> Option<&str> {
        if self.aboard_ship && self.docked && !self.settled_pad {
            self.docked_port.as_deref()
        } else {
            None
        }
    }

    /// Best-effort current port slug across trade venue, docked ship port, or active station pack.
    #[must_use]
    pub fn current_port_slug(&self) -> Option<&str> {
        if let Some(v) = self.current_venue.as_deref().filter(|s| !s.is_empty()) {
            return Some(v);
        }
        if let Some(d) = self.docked_port_slug() {
            return Some(d);
        }
        if !self.aboard_ship {
            if self.pack_id.contains("stickney") {
                return Some("stickney");
            }
            if self.pack_id.contains("ceres") {
                return Some("ceres");
            }
            if self.pack_id.contains("luna") || self.pack_id.contains("aitken") {
                return Some("luna");
            }
            if self.pack_id.contains("deimos") || self.pack_id.contains("voltaire") {
                return Some("deimos");
            }
            if self.pack_id.contains("titan") || self.pack_id.contains("ligeia") {
                return Some("titan");
            }
        }
        None
    }

    /// True while aboard a ship settled on a planetary surface landing pad (not docked at a station).
    #[must_use]
    pub fn settled_pad(&self) -> bool {
        self.aboard_ship && self.settled_pad
    }

    /// True when the tracked quest requires an active surface mining expedition (`!mine_done`),
    /// so a player ashore in a station should head to Berth Gallery to board their Dray-P.
    #[must_use]
    pub fn needs_surface_expedition_boarding(&self) -> bool {
        !self.quest_room_turn_in
            && self
                .quest_mining_status
                .as_ref()
                .is_some_and(|m| !m.mine_done)
    }

    fn normalize_port_slug(raw: &str) -> Option<String> {
        let low = raw.trim().to_ascii_lowercase();
        if low.is_empty() {
            return None;
        }
        if low.contains("stickney") || low.contains("phobos") {
            return Some("stickney".to_string());
        }
        if low.contains("aitken") || low.contains("luna") {
            return Some("luna".to_string());
        }
        if low.contains("voltaire") || low.contains("deimos") {
            return Some("deimos".to_string());
        }
        if low.contains("ligeia") || low.contains("titan") {
            return Some("titan".to_string());
        }
        if low.contains("ceres") || low.contains("freeport") {
            return Some("ceres".to_string());
        }
        None
    }

    /// Sync the active quest's target ship interior compartment (`"airlock"`, `"bridge"`, or `"bay"`).
    pub fn observe_quest_ship_target(&mut self, target: Option<&str>) {
        let next = target.filter(|s| !s.is_empty()).map(str::to_string);
        if self.quest_ship_target != next {
            self.quest_ship_target = next;
            self.refresh_npc_quest_labels();
            self.bump_scene();
        }
    }

    /// Compute the immediate next station exit OR ship interior compartment hatch
    /// along the shortest BFS path to the tracked quest's objective room/compartment.
    /// Supports cross-port Berth Gallery boarding (`+ship/board`), docked Airlock
    /// disembark (`+ship/leave`), and Step 6 surface expedition / settled-pad routing.
    #[must_use]
    pub fn next_quest_hop(&self) -> Option<QuestExitBreadcrumb> {
        let mut extra_step_label: Option<&'static str> = None;
        let (target_id, portals): (String, &[PortalGeom]) = if self.aboard_ship {
            if self.seated_at_helm {
                return None;
            }
            let docked_at_station = self.docked_port_slug().is_some();
            let docked_at_objective_port = self
                .docked_port_slug()
                .zip(
                    self.quest_room_entry
                        .as_deref()
                        .and_then(infer_station_port_from_room_entry),
                )
                .is_some_and(|(docked, want)| docked.eq_ignore_ascii_case(want));
            let mining_blocking = self
                .quest_mining_status
                .as_ref()
                .is_some_and(|m| !m.mine_done);
            let raw_target = if mining_blocking {
                if docked_at_station && !self.settled_pad {
                    "bridge"
                } else {
                    "bay"
                }
            } else if self.quest_ship_target.as_deref() == Some("airlock")
                || docked_at_objective_port
            {
                "airlock"
            } else if self.quest_mining_status.is_some() {
                "bridge"
            } else {
                self.quest_ship_target
                    .as_deref()
                    .or_else(|| self.quest_dest_port.as_ref().map(|_| "bridge"))?
            };
            if raw_target == "airlock" && self.active_location_key == "airlock" {
                return Some(QuestExitBreadcrumb {
                    next_room_entry: "ashore".into(),
                    next_room_name: "Cycle Ashore".into(),
                    next_exit_command: "+ship/leave".into(),
                    target_room_name: "Cycle Ashore".into(),
                    hops: 1,
                });
            }
            let resolved = if self.ship_rooms.iter().any(|r| r.entry_id == raw_target) {
                raw_target
            } else if raw_target == "bay" && self.ship_rooms.iter().any(|r| r.entry_id == "hold") {
                "hold"
            } else {
                return None;
            };
            if resolved == "airlock" {
                extra_step_label = Some("Cycle Ashore");
            }
            (resolved.to_string(), &self.ship_portals)
        } else if self.needs_surface_expedition_boarding() {
            let berth = self
                .rooms
                .iter()
                .find(|r| r.entry_id.to_ascii_lowercase().contains("berth_gallery"))?;
            if self.active_location_key == berth.entry_id {
                return Some(QuestExitBreadcrumb {
                    next_room_entry: "ship".into(),
                    next_room_name: "Board Ship".into(),
                    next_exit_command: "+ship/board".into(),
                    target_room_name: "Board Ship".into(),
                    hops: 1,
                });
            }
            extra_step_label = Some("Board Ship");
            (berth.entry_id.clone(), &self.portals)
        } else {
            let raw_target = self.quest_room_entry.as_deref()?;
            if self.rooms.iter().any(|r| r.entry_id == raw_target) {
                (raw_target.to_string(), &self.portals)
            } else if let Some(role_room) = self.find_same_station_role_room(raw_target) {
                (role_room.entry_id.clone(), &self.portals)
            } else if infer_station_port_from_room_entry(raw_target).is_some() {
                if self.needs_origin_port_procurement()
                    && let Some(exchange) = self
                        .rooms
                        .iter()
                        .find(|r| r.entry_id.to_ascii_lowercase().contains("exchange"))
                        .or_else(|| {
                            self.rooms.iter().find(|r| {
                                let low = r.entry_id.to_ascii_lowercase();
                                low.contains("chandlery") || low.contains("service")
                            })
                        })
                {
                    if self.active_location_key == exchange.entry_id {
                        return None;
                    }
                    (exchange.entry_id.clone(), &self.portals)
                } else {
                    let berth = self
                        .rooms
                        .iter()
                        .find(|r| r.entry_id.to_ascii_lowercase().contains("berth_gallery"))?;
                    if self.active_location_key == berth.entry_id {
                        return Some(QuestExitBreadcrumb {
                            next_room_entry: "ship".into(),
                            next_room_name: "Board Ship".into(),
                            next_exit_command: "+ship/board".into(),
                            target_room_name: "Board Ship".into(),
                            hops: 1,
                        });
                    }
                    extra_step_label = Some("Board Ship");
                    (berth.entry_id.clone(), &self.portals)
                }
            } else {
                return None;
            }
        };
        if target_id.is_empty() || self.active_location_key == target_id {
            return None;
        }
        type BfsHop = (String, String);
        let mut queue: VecDeque<(String, Option<BfsHop>, usize)> = VecDeque::new();
        let mut visited: HashSet<String> = HashSet::new();
        queue.push_back((self.active_location_key.clone(), None, 0));
        visited.insert(self.active_location_key.clone());

        while let Some((cur, first_hop, depth)) = queue.pop_front() {
            for p in portals {
                let valid_cmd = p.command.starts_with("goto ")
                    || (self.aboard_ship && p.command.starts_with("+tv/bay "));
                if p.from_room != cur || visited.contains(&p.to_room) || !valid_cmd {
                    continue;
                }
                let hop = first_hop
                    .clone()
                    .unwrap_or_else(|| (p.command.clone(), p.to_room.clone()));
                let next_depth = depth + 1;
                if p.to_room == target_id {
                    let (cmd, next_room_entry) = hop;
                    let next_room_name = self
                        .find_room(&next_room_entry)
                        .map_or_else(|| next_room_entry.clone(), |r| r.name.clone());
                    let base_target_name = self
                        .find_room(&target_id)
                        .map_or_else(|| target_id.clone(), |r| r.name.clone());
                    let (target_room_name, hops) = if let Some(suffix) = extra_step_label {
                        (format!("{base_target_name} ({suffix})"), next_depth + 1)
                    } else {
                        (base_target_name, next_depth)
                    };
                    return Some(QuestExitBreadcrumb {
                        next_room_entry,
                        next_room_name,
                        next_exit_command: cmd,
                        target_room_name,
                        hops,
                    });
                }
                visited.insert(p.to_room.clone());
                queue.push_back((p.to_room.clone(), Some(hop), next_depth));
            }
        }
        None
    }

    fn active_trade_qty_setting(&self) -> i64 {
        self.custom_trade_qty
            .unwrap_or_else(|| QTY_PRESETS.get(self.qty_preset_idx).copied().unwrap_or(1))
    }

    fn apply_quest_trade_jump(&mut self) -> bool {
        let Some(need) = self.effective_quest_trade_need() else {
            return false;
        };
        let target_qty = need.target_quantity.max(1);
        if let Some(pidx) = QTY_PRESETS.iter().position(|&p| p == target_qty) {
            self.qty_preset_idx = pidx;
            self.custom_trade_qty = None;
        } else {
            self.custom_trade_qty = Some(target_qty);
        }
        if need.wants_buy
            && let Some(idx) = self
                .asks
                .iter()
                .position(|a| need.matches_listing(&a.good_id, &a.good_name, false))
        {
            self.trade_subtab = 0;
            self.cursor_row = idx;
        } else if need.wants_sell
            && let Some(idx) = self
                .bids
                .iter()
                .position(|b| need.matches_listing(&b.good_id, &b.good_name, true))
        {
            self.trade_subtab = 1;
            self.cursor_row = idx;
        }
        self.push_drawer_log(format!(
            "[QUEST] Target {} x {target_qty} ({})",
            need.commodity_slug.replace('_', " "),
            need.quest_title
        ));
        true
    }

    fn sailing_quest_target_code(&self) -> Option<&str> {
        if let Some(ref hint) = self.quest_dest_port {
            return Some(hint.port_id.as_str());
        }
        if self
            .quest_mining_status
            .as_ref()
            .is_some_and(|m| !m.mine_done)
        {
            return Some("commons");
        }
        None
    }

    fn ordered_sailing_corridor_indices(&self) -> Vec<usize> {
        let mut indices: Vec<usize> = (0..CORRIDORS.len()).collect();
        if let Some(target) = self.sailing_quest_target_code()
            && let Some(pos) = CORRIDORS
                .iter()
                .position(|(id, _, _)| id.eq_ignore_ascii_case(target))
        {
            indices.remove(pos);
            indices.insert(0, pos);
        }
        indices
    }

    fn apply_quest_destination_jump(&mut self) -> bool {
        if let Some(hint) = self.quest_dest_port.clone() {
            if let Some(idx) = CORRIDORS
                .iter()
                .position(|(id, _, _)| id.eq_ignore_ascii_case(&hint.port_id))
            {
                self.corridor_idx = idx;
                self.push_drawer_log(format!(
                    "[QUEST] Selected corridor destination: {} ({})",
                    hint.port_label, hint.quest_title
                ));
                return true;
            }
            return false;
        }
        if let Some(qm) = self.quest_mining_status.clone()
            && !qm.mine_done
            && let Some(idx) = CORRIDORS
                .iter()
                .position(|(id, _, _)| id.eq_ignore_ascii_case("commons"))
        {
            self.corridor_idx = idx;
            self.push_drawer_log(format!(
                "[QUEST] Selected surface extraction site: Freeport Commons ({})",
                qm.target_ore_slug.replace('_', " ")
            ));
            return true;
        }
        false
    }

    fn find_quest_marker(&self, npc_id: &str, npc_name: &str) -> Option<&QuestMarker> {
        self.quest_markers
            .iter()
            .filter(|m| {
                (!npc_id.is_empty() && m.npc_id == npc_id)
                    || (!npc_name.is_empty() && m.npc_name.eq_ignore_ascii_case(npc_name))
            })
            .min_by_key(|m| Self::quest_marker_priority(m))
    }

    fn count_npc_quest_markers(&self, npc_id: &str, npc_name: &str) -> usize {
        let mut keys: Vec<String> = Vec::new();
        for m in &self.quest_markers {
            let matches = (!npc_id.is_empty() && m.npc_id == npc_id)
                || (!npc_name.is_empty() && m.npc_name.eq_ignore_ascii_case(npc_name));
            if matches {
                let k = if m.quest_key.is_empty() {
                    format!("{}:{}", m.marker, m.quest_title)
                } else {
                    m.quest_key.clone()
                };
                if !keys.contains(&k) {
                    keys.push(k);
                }
            }
        }
        keys.len()
    }

    fn format_npc_interact_label(&self, id: &str, name: &str, npc: bool) -> String {
        if npc && let Some(m) = self.find_quest_marker(id, name) {
            let extra = self.count_npc_quest_markers(id, name).saturating_sub(1);
            let more = if extra > 0 {
                format!(" (+{extra} more)")
            } else {
                String::new()
            };
            return match m.marker.as_str() {
                "turn_in" => format!("[?] {name} — Turn In Quest{more}"),
                "available" => format!("[!] {name} — Quest Available{more}"),
                _ => format!("[?] Talk to {name}{more}"),
            };
        }
        if npc {
            format!("Talk to {name}")
        } else {
            format!("Greet {name}")
        }
    }

    fn interactable_quest_priority(&self, target: &InteractableTarget) -> u8 {
        match &target.action {
            InteractAction::OpenNpc { id, name } => self
                .find_quest_marker(id, name)
                .map_or(3, Self::quest_marker_priority),
            InteractAction::Command(cmd) => {
                let is_bay_quest = *cmd == "+tv/bay in"
                    && (self
                        .quest_mining_status
                        .as_ref()
                        .is_some_and(|m| !m.mine_done)
                        || self.quest_ship_target.as_deref() == Some("bay"));
                let is_hop_cmd = (*cmd == "+ship/board" || *cmd == "+ship/leave")
                    && self
                        .next_quest_hop()
                        .is_some_and(|hop| hop.next_exit_command == *cmd);
                if is_bay_quest || is_hop_cmd { 1 } else { 3 }
            }
            _ => 3,
        }
    }

    fn sort_interactables_quest_first(&mut self, preserve_focus: bool) {
        if self.interactables.len() <= 1 {
            return;
        }
        let focused_action = if preserve_focus {
            self.interactables
                .get(self.focus_idx)
                .map(|t| t.action.clone())
        } else {
            None
        };
        let priorities: Vec<u8> = self
            .interactables
            .iter()
            .map(|t| self.interactable_quest_priority(t))
            .collect();
        let mut indexed: Vec<(u8, usize, InteractableTarget)> = self
            .interactables
            .drain(..)
            .enumerate()
            .map(|(i, t)| (priorities[i], i, t))
            .collect();
        indexed.sort_by_key(|(prio, idx, _)| (*prio, *idx));
        self.interactables = indexed.into_iter().map(|(_, _, t)| t).collect();
        if let Some(prev) = focused_action
            && let Some(new_idx) = self.interactables.iter().position(|t| t.action == prev)
        {
            self.focus_idx = new_idx;
        } else if !self.interactables.is_empty() {
            self.focus_idx = self.focus_idx.min(self.interactables.len() - 1);
        }
    }

    fn refresh_npc_quest_labels(&mut self) {
        let bay_quest = self
            .quest_mining_status
            .as_ref()
            .is_some_and(|m| !m.mine_done)
            || self.quest_ship_target.as_deref() == Some("bay");
        let mut updates: Vec<(usize, String)> = Vec::new();
        for (idx, target) in self.interactables.iter().enumerate() {
            match &target.action {
                InteractAction::OpenNpc { id, name } => {
                    let is_npc = self
                        .actors
                        .iter()
                        .find(|a| {
                            (!id.is_empty() && a.id == *id) || a.name.eq_ignore_ascii_case(name)
                        })
                        .is_none_or(|a| a.npc);
                    let next_label = self.format_npc_interact_label(id, name, is_npc);
                    if target.label != next_label {
                        updates.push((idx, next_label));
                    }
                }
                InteractAction::Command("+tv/bay in") => {
                    let next_label = if bay_quest {
                        "[Q] Enter Vehicle Bay (+tv/bay in)".to_string()
                    } else {
                        "Enter Vehicle Bay (+tv/bay in)".to_string()
                    };
                    if target.label != next_label {
                        updates.push((idx, next_label));
                    }
                }
                _ => {}
            }
        }
        for (idx, lbl) in updates {
            if let Some(target) = self.interactables.get_mut(idx) {
                target.label = lbl;
            }
        }
        self.sort_interactables_quest_first(true);
    }

    /// Observe a live `RoomView` from the server.
    pub fn observe_room(&mut self, view: &RoomView, self_name: Option<&str>) {
        let was_aboard = self.aboard_ship;
        let clean_name = sanitize(&view.name);
        self.room_title = clean_name.clone();
        let clean_self = self_name.map(sanitize);
        self.last_room_view = Some(view.clone());
        self.last_self_name.clone_from(&clean_self);
        self.quest_markers = parse_room_markers(view);

        let tac = view.ambient.get("tacsim").and_then(Value::as_object);
        let atlas = view.ambient.get("atlas").and_then(Value::as_object);

        let mut received_room_stowage = false;
        if let Some(st) = tac
            .and_then(|t| t.get("stowage").or_else(|| t.get("own_stowage")))
            .and_then(Value::as_object)
        {
            self.decode_stowage_obj(st);
            received_room_stowage = true;
        }

        if let Some(t) = tac {
            if let Some(own_h_val) = t.get("own_hull_name") {
                if let Some(own_h) = own_h_val.as_str() {
                    let clean_h = sanitize(own_h);
                    if !self.own_hull_name.is_empty()
                        && self.own_hull_name != clean_h
                        && !received_room_stowage
                    {
                        self.own_hull_id = None;
                        self.reset_stowage();
                    }
                    self.own_hull_name = clean_h;
                } else if own_h_val.is_null() {
                    self.own_hull_id = None;
                    self.own_hull_name.clear();
                    if !received_room_stowage {
                        self.reset_stowage();
                    }
                }
            }
            if let Some(docked_here) = t.get("own_hull_docked_here").and_then(Value::as_bool) {
                self.own_hull_docked_here = docked_here;
            }
        }

        // Detect ship interior vs station room authoritatively (`SSH-M-06`):
        // Trust `ambient.tacsim.mode == "interior"` when ambient metadata is present;
        // never let `" - "` inside a station room title falsely trigger ship interior mode.
        let mode_str = tac
            .and_then(|t| t.get("mode"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let is_interior = if mode_str == "interior" {
            true
        } else if tac.is_some() || atlas.is_some() {
            false
        } else {
            clean_name
                .rsplit_once(" - ")
                .and_then(|(_, right)| self.match_ship_compartment(right))
                .is_some()
        };

        if is_interior {
            // Load unit-specific interior layout if known & cached, or queue its pack.
            if let Some(unit_raw) = tac.and_then(|t| t.get("unit")).and_then(Value::as_str) {
                let slug = normalize_unit_slug(&sanitize(unit_raw));
                if !slug.is_empty() && slug != self.ship_unit_id {
                    self.ship_unit_id = slug.clone();
                    self.missing_packs.remove(&slug);
                    if let Some((sr, sp)) = self.unit_cache.get(&slug).cloned() {
                        self.ship_rooms = sr;
                        self.ship_portals = sp;
                    } else {
                        let (sr, sp) = builtin_dray_layout();
                        self.ship_rooms = sr;
                        self.ship_portals = sp;
                        if !self.is_pack_loaded_current(&slug) {
                            self.pending_pack_requests.push(slug);
                        }
                    }
                }
            }

            let comp_from_tac = tac
                .and_then(|t| t.get("room"))
                .and_then(Value::as_str)
                .map(sanitize)
                .and_then(|r| self.match_ship_compartment(&r));
            let comp_from_title = clean_name
                .rsplit_once(" - ")
                .and_then(|(_, right)| self.match_ship_compartment(right));
            let comp = comp_from_tac
                .or(comp_from_title)
                .unwrap_or_else(|| "airlock".to_string());

            self.aboard_ship = true;
            let new_hull_name = tac
                .and_then(|t| t.get("hull_name"))
                .and_then(Value::as_str)
                .map(sanitize)
                .or_else(|| clean_name.split_once(" - ").map(|(h, _)| h.to_string()))
                .unwrap_or_else(|| "Charter Hull".to_string());
            if !self.hull_name.is_empty()
                && self.hull_name != new_hull_name
                && !received_room_stowage
            {
                self.own_hull_id = None;
                self.reset_stowage();
            }
            self.hull_name = new_hull_name;
            self.docked = tac
                .and_then(|t| t.get("docked"))
                .and_then(Value::as_bool)
                .unwrap_or(true);
            self.settled_pad = tac
                .and_then(|t| t.get("settled_pad"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !self.docked || self.settled_pad {
                self.docked_port = None;
            } else if let Some(raw_port) = tac
                .and_then(|t| t.get("port").or_else(|| t.get("port_entry")))
                .and_then(Value::as_str)
            {
                self.docked_port = Self::normalize_port_slug(&sanitize(raw_port));
            }
            if comp != "bridge" {
                self.seated_at_helm = false;
            }
            if !was_aboard {
                self.pending_move_target = None;
                self.pending_interaction = None;
            }
            self.server_location_key.clone_from(&comp);
            if let Some((pt, _)) = &self.pending_move_target
                && pt == &comp
            {
                self.pending_move_target = None;
            }
            let now = self.last_tick.unwrap_or_else(Instant::now);
            self.advance_pending_interaction_move(now);
            if !was_aboard
                || !self
                    .ship_rooms
                    .iter()
                    .any(|r| r.entry_id == self.active_location_key)
                || (self.active_location_key != comp && self.pending_move_target.is_none())
            {
                self.snap_to_room(&comp);
            }
            self.rebuild_actors_and_interactables(view, clean_self.as_deref());
            self.resume_pending_interaction_if_arrived(now);
            self.bump_scene();
            return;
        }

        // Station / OOC room
        self.aboard_ship = false;
        self.seated_at_helm = false;
        if was_aboard {
            self.pending_move_target = None;
            self.pending_interaction = None;
        }

        let atlas_pack = atlas
            .and_then(|a| {
                a.get("pack")
                    .or_else(|| a.get("neighborhood_pack"))
                    .and_then(Value::as_str)
            })
            .map(sanitize)
            .filter(|s| !s.is_empty());

        let want_pack = if let Some(p) = atlas_pack {
            p
        } else if mode_str == "simulator"
            || clean_name.to_ascii_lowercase().contains("simulator")
            || clean_name.to_ascii_lowercase().contains("red room")
            || clean_name.to_ascii_lowercase().contains("blue room")
        {
            "neighborhood.ooc_area".to_string()
        } else {
            "neighborhood.ceres_freeport".to_string()
        };

        let atlas_entry = atlas
            .and_then(|a| a.get("entry"))
            .and_then(Value::as_str)
            .map(sanitize)
            .filter(|s| !s.is_empty());

        if self.pack_id != want_pack {
            self.pack_id = want_pack.clone();
            self.missing_packs.remove(&want_pack);
            if let Some((r, p)) = self.neighborhood_cache.get(&want_pack).cloned() {
                self.rooms = r;
                self.portals = p;
            } else if want_pack == "neighborhood.ooc_area" || want_pack.contains("ooc") {
                let (r, p) = builtin_ooc_layout();
                self.rooms = r;
                self.portals = p;
                if !self.is_pack_loaded_current(&want_pack) {
                    self.pending_pack_requests.push(want_pack.clone());
                }
            } else if want_pack == "neighborhood.ceres_freeport" {
                let (r, p) = builtin_ceres_layout();
                self.rooms = r;
                self.portals = p;
                if !self.is_pack_loaded_current(&want_pack) {
                    self.pending_pack_requests.push(want_pack.clone());
                }
            } else {
                // Unknown/non-Ceres port whose neighborhood pack hasn't arrived yet:
                // create a placeholder room for the real entry (never falsely render Ceres!)
                // and queue an immediate `tacsim.content` pack request.
                let fallback_entry = atlas_entry
                    .clone()
                    .unwrap_or_else(|| "room.current".to_string());
                let kind = classify_room_kind(&fallback_entry, false, "indoor");
                self.rooms = vec![RoomGeom {
                    entry_id: fallback_entry,
                    name: clean_name.clone(),
                    floor: 0,
                    min_x: -12.0,
                    min_y: -10.0,
                    max_x: 12.0,
                    max_y: 10.0,
                    kind,
                }];
                self.portals = Vec::new();
                if !self.is_pack_loaded_current(&want_pack) {
                    self.pending_pack_requests.push(want_pack.clone());
                }
            }
        }

        // If the player moves to another room inside an uncached non-Ceres/non-OOC port
        // before its neighborhood pack arrives, keep the single placeholder synced to the
        // new `atlas_entry` so `server_location_key` snaps cleanly once the pack lands.
        if !self.neighborhood_cache.contains_key(&self.pack_id)
            && self.pack_id != "neighborhood.ceres_freeport"
            && self.pack_id != "neighborhood.ooc_area"
            && !self.pack_id.contains("ooc")
            && let Some(ref e) = atlas_entry
            && !self.rooms.iter().any(|r| r.entry_id == *e)
            && self.rooms.len() == 1
        {
            let kind = classify_room_kind(e, false, "indoor");
            self.rooms[0].entry_id.clone_from(e);
            self.rooms[0].name.clone_from(&clean_name);
            self.rooms[0].kind = kind;
        }

        let entry = atlas_entry
            .as_deref()
            .and_then(|e| {
                self.rooms
                    .iter()
                    .find(|r| r.entry_id == e)
                    .map(|r| r.entry_id.clone())
            })
            .or_else(|| {
                let lower = clean_name.trim().to_ascii_lowercase();
                self.rooms
                    .iter()
                    .find(|r| {
                        let rlow = r.name.to_ascii_lowercase();
                        rlow == lower || (!rlow.is_empty() && lower.contains(&rlow))
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
        let now = self.last_tick.unwrap_or_else(Instant::now);
        self.advance_pending_interaction_move(now);
        if was_aboard
            || !self
                .rooms
                .iter()
                .any(|r| r.entry_id == self.active_location_key)
        {
            self.snap_to_room(&entry);
        } else if self.active_location_key != entry && self.pending_move_target.is_none() {
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

        self.rebuild_actors_and_interactables(view, clean_self.as_deref());
        self.resume_pending_interaction_if_arrived(now);
        self.bump_scene();
    }

    /// Observe narrative speech for 2.5D/1P actor speech bubbles and drawer transcript.
    /// Always prunes expired bubbles and caps size even when `StationWalk` is closed (`SSH-H-03`).
    pub fn observe_narrative(
        &mut self,
        _kind: NarrativeKind,
        actor: Option<&ObjRef>,
        plain_text: &str,
        now: Instant,
    ) {
        self.speech_bubbles.retain(|_, (_, exp)| *exp > now);
        let clean = sanitize(plain_text);
        let trimmed = clean.trim();
        if trimmed.is_empty() {
            return;
        }
        if let Some(act) = actor {
            let clean_actor = sanitize(&act.name);
            let clean_id = sanitize(&act.id.to_string());
            let bubble = truncate_str(trimmed, MAX_SPEECH_CHARS);
            let exp = now + SPEECH_TTL;
            self.insert_speech_bubble(clean_id, bubble.clone(), exp);
            if !clean_actor.is_empty() {
                self.insert_speech_bubble(clean_actor.to_ascii_lowercase(), bubble, exp);
            }
            self.bump_scene();
        }
        if self.drawer.is_some() {
            let actor_prefix = actor.map(|a| sanitize(&a.name));
            for part in trimmed.lines() {
                let p = part.trim();
                if p.is_empty() {
                    continue;
                }
                let line = match actor_prefix.as_deref() {
                    Some(name) if !name.is_empty() => format!("{name}: {p}"),
                    _ => p.to_string(),
                };
                self.push_drawer_log(line);
            }
        }
    }

    fn insert_speech_bubble(&mut self, key: String, text: String, exp: Instant) {
        if key.is_empty() {
            return;
        }
        if !self.speech_bubbles.contains_key(&key) {
            while self.speech_bubbles.len() >= MAX_SPEECH_BUBBLES {
                let oldest = self
                    .speech_bubbles
                    .iter()
                    .min_by_key(|(_, (_, e))| *e)
                    .map(|(k, _)| k.clone());
                match oldest {
                    Some(k) => {
                        self.speech_bubbles.remove(&k);
                    }
                    None => break,
                }
            }
        }
        self.speech_bubbles.insert(key, (text, exp));
    }

    pub fn observe_sys(&mut self, text: &str) {
        if self.drawer.is_none() {
            return;
        }
        for raw_line in text.lines() {
            let clean = sanitize(raw_line);
            let trimmed = clean.trim();
            if !trimmed.is_empty() {
                self.push_drawer_log(trimmed.to_string());
            }
        }
    }

    /// Observe `port.trade` panel/query keyframe against live `crates/tacsim/src/client_sync.rs`
    /// (`build_port_trade_frame`) and run 1-click Quote-to-Accept verification (`SSH-H-02`, `C-20`).
    ///
    /// Any auto-accepted `+market/accept` command is queued once into `self.pending_orders`
    /// (retrieved via [`take_orders`](Self::take_orders)); returns only follow-up queries (`SSH-L-02`).
    pub fn observe_port_trade(&mut self, data: &Value, now: Instant) -> Vec<(String, Value)> {
        let mut queries = Vec::new();
        let Some(obj) = data.as_object() else {
            return queries;
        };
        let mut visual_changed = false;

        if let Some(cr) = obj
            .get("balance_minor")
            .or_else(|| obj.get("credits_minor"))
            .and_then(Value::as_i64)
        {
            self.credits_minor = cr;
        }
        if let Some(bt) = obj.get("balance_text").and_then(Value::as_str) {
            self.balance_text = sanitize(bt);
        }
        if let Some(venue_val) = obj.get("venue") {
            if venue_val.is_null() {
                self.current_venue = None;
            } else if let Some(v) = venue_val.as_str() {
                let clean_v = sanitize(v);
                let trimmed = clean_v.trim();
                self.current_venue =
                    if trimmed.is_empty() || trimmed.chars().any(char::is_whitespace) {
                        None
                    } else {
                        Some(trimmed.to_string())
                    };
            }
        }
        if let Some(pn_val) = obj.get("port_name") {
            if pn_val.is_null() {
                self.port_name.clear();
            } else if let Some(pn) = pn_val.as_str() {
                self.port_name = sanitize(pn);
            }
        }
        if let Some(adp_val) = obj.get("aboard_docked_port") {
            if adp_val.is_null() {
                if !self.docked {
                    self.docked_port = None;
                }
            } else if let Some(adp) = adp_val.as_str() {
                self.docked_port = Self::normalize_port_slug(&sanitize(adp));
            }
        }
        self.trade_next_after = obj.get("next_after").and_then(Value::as_u64);

        // Live `port.trade.ship` (`client_sync.rs:1296-1327`)
        if let Some(ship_val) = obj.get("ship") {
            if ship_val.is_null() {
                if !self.own_hull_name.is_empty()
                    || self.own_hull_id.is_some()
                    || self.stowage_loaded
                    || self.stowage.cargo_kg != 0
                    || !self.cargo_lots.is_empty()
                {
                    visual_changed = true;
                }
                self.own_hull_id = None;
                self.own_hull_name.clear();
                self.reset_stowage();
            } else if let Some(ship_obj) = ship_val.as_object() {
                let new_hull_id = ship_obj
                    .get("hull_id")
                    .map(|v| match v {
                        Value::String(s) => sanitize(s),
                        Value::Number(n) => n.to_string(),
                        _ => String::new(),
                    })
                    .filter(|s| !s.is_empty());
                let new_hull_name = ship_obj
                    .get("hull_name")
                    .and_then(Value::as_str)
                    .map(sanitize);
                let ship_changed = match (&self.own_hull_id, &new_hull_id) {
                    (Some(old), Some(new)) => old != new,
                    (None, Some(_)) => !self.own_hull_name.is_empty() || self.stowage_loaded,
                    _ => new_hull_name
                        .as_deref()
                        .is_some_and(|n| !self.own_hull_name.is_empty() && self.own_hull_name != n),
                };
                if let Some(hid) = new_hull_id {
                    if self.own_hull_id.as_deref() != Some(hid.as_str()) {
                        visual_changed = true;
                    }
                    self.own_hull_id = Some(hid);
                }
                if let Some(clean_h) = new_hull_name
                    && self.own_hull_name != clean_h
                {
                    self.own_hull_name = clean_h;
                    visual_changed = true;
                }
                if let Some(unit) = ship_obj.get("unit").and_then(Value::as_str) {
                    let slug = normalize_unit_slug(&sanitize(unit));
                    if !slug.is_empty() && !self.is_pack_loaded_current(&slug) {
                        self.pending_pack_requests.push(slug);
                    }
                }
                let stowage_val = ship_obj.get("stowage");
                if let Some(st) = stowage_val.and_then(Value::as_object) {
                    let old_kg = self.stowage.cargo_kg;
                    if ship_changed {
                        self.reset_stowage();
                    }
                    self.decode_stowage_obj(st);
                    if ship_changed || self.stowage.cargo_kg != old_kg {
                        visual_changed = true;
                    }
                } else if ship_changed || stowage_val.is_some_and(Value::is_null) {
                    self.reset_stowage();
                    visual_changed = true;
                }
            }
        }

        // Legacy top-level `stowage` fallback
        if let Some(stowage_val) = obj.get("stowage") {
            if stowage_val.is_null() {
                if self.stowage_loaded || self.stowage.cargo_kg != 0 || !self.cargo_lots.is_empty()
                {
                    visual_changed = true;
                }
                self.reset_stowage();
            } else if let Some(st) = stowage_val.as_object() {
                let old_kg = self.stowage.cargo_kg;
                self.decode_stowage_obj(st);
                if self.stowage.cargo_kg != old_kg {
                    visual_changed = true;
                }
            }
        }

        // Live `port.trade.berth` (`port/ship.rs:1593-1647`)
        if let Some(berth_val) = obj.get("berth") {
            if berth_val.is_null() {
                if !self.berth_docked_hulls.is_empty() {
                    self.berth_docked_hulls.clear();
                    visual_changed = true;
                }
            } else if let Some(berth) = berth_val.as_object() {
                if let Some(dh_arr) = berth.get("docked_hulls").and_then(Value::as_array) {
                    let mut parsed = Vec::new();
                    for item in dh_arr.iter().take(16) {
                        let id = item
                            .get("id")
                            .and_then(Value::as_str)
                            .map(sanitize)
                            .unwrap_or_default();
                        let name = item
                            .get("name")
                            .and_then(Value::as_str)
                            .map(sanitize)
                            .unwrap_or_default();
                        let unit = item
                            .get("unit")
                            .and_then(Value::as_str)
                            .map(sanitize)
                            .unwrap_or_default();
                        if !name.is_empty() {
                            parsed.push(DockedHullInfo { id, name, unit });
                        }
                    }
                    self.berth_docked_hulls = parsed;
                    visual_changed = true;
                }
                if let Some(own_name_val) = berth.get("own_hull_name") {
                    if let Some(n) = own_name_val.as_str() {
                        let clean = sanitize(n);
                        if self.own_hull_name != clean {
                            self.own_hull_name = clean;
                            visual_changed = true;
                        }
                    } else if own_name_val.is_null() && obj.get("ship").is_none() {
                        self.own_hull_name.clear();
                        visual_changed = true;
                    }
                }
                if let Some(dh) = berth.get("own_hull_docked_here").and_then(Value::as_bool) {
                    self.own_hull_docked_here = dh;
                }
            }
        }

        // Live `port.trade.simulator` (`port/ship.rs:1652-1706`)
        if let Some(sim_val) = obj.get("simulator") {
            if sim_val.is_null() {
                self.simulator = None;
            } else if let Some(sim_obj) = sim_val.as_object() {
                let mut slots = Vec::new();
                if let Some(s_arr) = sim_obj.get("slots").and_then(Value::as_array) {
                    for s in s_arr.iter().take(16) {
                        slots.push(SimulatorSlotInfo {
                            team: s
                                .get("team")
                                .and_then(Value::as_str)
                                .map(sanitize)
                                .unwrap_or_default(),
                            index: s.get("index").and_then(Value::as_u64).unwrap_or(0),
                            pilot_name: s
                                .get("pilot_name")
                                .and_then(Value::as_str)
                                .map(sanitize)
                                .unwrap_or_default(),
                            unit: s
                                .get("unit")
                                .and_then(Value::as_str)
                                .map(sanitize)
                                .unwrap_or_default(),
                            seated: s.get("seated").and_then(Value::as_bool).unwrap_or(false),
                        });
                    }
                }
                self.simulator = Some(SimulatorState {
                    arena: sim_obj
                        .get("arena")
                        .and_then(Value::as_str)
                        .map(sanitize)
                        .unwrap_or_default(),
                    match_state: sim_obj
                        .get("match_state")
                        .and_then(Value::as_str)
                        .map(sanitize)
                        .unwrap_or_default(),
                    my_team: sim_obj.get("my_team").and_then(Value::as_str).map(sanitize),
                    my_unit: sim_obj.get("my_unit").and_then(Value::as_str).map(sanitize),
                    slots,
                });
            }
        }

        // Live `port.trade.listings[]` (`client_sync.rs:1280-1292`; 48e L8, clients:G3)
        if let Some(listings_arr) = obj.get("listings").and_then(Value::as_array) {
            let append = std::mem::take(&mut self.awaiting_trade_page);
            if !append {
                self.asks.clear();
                self.bids.clear();
            }
            for item in listings_arr.iter().take(128) {
                let lid = item
                    .get("id")
                    .or_else(|| item.get("listing_id"))
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                let gid = item
                    .get("good")
                    .and_then(Value::as_str)
                    .map(sanitize)
                    .unwrap_or_default();
                if lid <= 0 || gid.is_empty() {
                    continue;
                }
                let side = item.get("side").and_then(Value::as_str).unwrap_or("ask");
                let (meta_name, mass_kg, vol_dm3) = self.lookup_goods_meta(&gid);
                let gname = item
                    .get("good_label")
                    .and_then(Value::as_str)
                    .map(sanitize)
                    .filter(|s| !s.is_empty())
                    .unwrap_or(meta_name);
                let vendor_name = item
                    .get("vendor_name")
                    .and_then(Value::as_str)
                    .map(sanitize)
                    .unwrap_or_default();
                let qty = item
                    .get("available_qty")
                    .or_else(|| item.get("quantity"))
                    .or_else(|| item.get("qty"))
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                let price = item
                    .get("unit_price_minor")
                    .or_else(|| item.get("price_minor"))
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                let funded = item.get("funded").and_then(Value::as_bool).unwrap_or(true);
                if side == "bid" {
                    if !self.bids.iter().any(|b| b.listing_id == lid) && self.bids.len() < 256 {
                        self.bids.push(BidRow {
                            listing_id: lid,
                            good_id: gid,
                            good_name: gname,
                            vendor_name,
                            quantity_wanted: qty,
                            unit_price_minor: price,
                            funded,
                        });
                    }
                } else if !self.asks.iter().any(|a| a.listing_id == lid) && self.asks.len() < 256 {
                    self.asks.push(AskRow {
                        listing_id: lid,
                        good_id: gid,
                        good_name: gname,
                        vendor_name,
                        quantity: qty,
                        unit_price_minor: price,
                        unit_mass_kg: mass_kg,
                        unit_vol_dm3: vol_dm3,
                    });
                }
            }
        } else {
            // Legacy separate `asks` / `bids` fallback
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
                        .map(sanitize)
                        .unwrap_or_default();
                    if lid <= 0 || gid.is_empty() {
                        continue;
                    }
                    let (gname, mass_kg, vol_dm3) = self.lookup_goods_meta(&gid);
                    let qty = item
                        .get("available_qty")
                        .or_else(|| item.get("quantity"))
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
                        vendor_name: String::new(),
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
                        .map(sanitize)
                        .unwrap_or_default();
                    if lid <= 0 || gid.is_empty() {
                        continue;
                    }
                    let (gname, _, _) = self.lookup_goods_meta(&gid);
                    let qty = item
                        .get("available_qty")
                        .or_else(|| item.get("quantity_wanted"))
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
                        vendor_name: String::new(),
                        quantity_wanted: qty,
                        unit_price_minor: price,
                        funded: true,
                    });
                }
            }
        }

        // Legacy top-level `cargo` / `lots` fallback if `ship.stowage.items` was not present
        if let Some(cargo_arr) = obj
            .get("cargo")
            .or_else(|| obj.get("lots"))
            .and_then(Value::as_array)
        {
            self.decode_cargo_items_arr(cargo_arr);
        }

        // Live `port.trade.open_quotes[]` (`client_sync.rs:1188-1220`), fallback `own.quotes` / `quotes`
        if let Some(q_arr) = obj
            .get("open_quotes")
            .or_else(|| obj.get("own").and_then(|o| o.get("quotes")))
            .or_else(|| obj.get("quotes"))
            .and_then(Value::as_array)
        {
            self.own_quotes.clear();
            for q in q_arr.iter().take(64) {
                let qid = q
                    .get("quote_id")
                    .or_else(|| q.get("id"))
                    .map(|v| match v {
                        Value::String(s) => sanitize(s),
                        Value::Number(n) => n.to_string(),
                        _ => String::new(),
                    })
                    .unwrap_or_default();
                let lid = q.get("listing_id").and_then(Value::as_i64).unwrap_or(0);
                let qty = q
                    .get("quantity")
                    .or_else(|| q.get("qty"))
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                let uprice = q
                    .get("unit_price_minor")
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                let total_minor = q.get("total_minor").and_then(Value::as_i64);
                let expired = q.get("expired").and_then(Value::as_bool).unwrap_or(false);
                if !qid.is_empty() {
                    // The server sends only live quotes (`build_open_quotes_json`).
                    self.own_quotes.push(OwnQuoteRow {
                        quote_id: qid,
                        listing_id: lid,
                        quantity: qty,
                        unit_price_minor: uprice,
                        total_minor,
                        expired,
                    });
                }
            }
        }

        // Verify pending 1-click Quote -> Accept: require exact `listing_id`, exact `quantity`,
        // a fresh `quote_id` not present prior to `+market/quote`, and total <= authorized total.
        if let Some(pq) = self.pending_quote.clone() {
            if now.saturating_duration_since(pq.created_at) > Duration::from_secs(12) {
                self.pending_quote = None;
            } else if let Some(matched) = self.own_quotes.iter().find(|q| {
                !q.expired
                    && q.listing_id > 0
                    && q.listing_id == pq.listing_id
                    && q.quantity > 0
                    && q.quantity == pq.quantity
                    && !pq.prior_quote_ids.contains(&q.quote_id)
            }) {
                self.pending_quote = None;
                let effective_total = matched
                    .total_minor
                    .or_else(|| matched.unit_price_minor.checked_mul(matched.quantity));
                let within_authorized = matched.unit_price_minor >= 0
                    && matched.unit_price_minor <= pq.max_unit_price_minor
                    && effective_total
                        .is_some_and(|tot| tot >= 0 && tot <= pq.authorized_total_minor);
                if within_authorized {
                    let accept = format!("+market/accept {}", matched.quote_id);
                    self.push_drawer_log(format!(
                        "[QUOTE VERIFIED] Accepting #{} ({} units)...",
                        matched.quote_id, matched.quantity
                    ));
                    self.pending_orders.push(accept);
                    self.awaiting_trade_page = false;
                    queries.push((PORT_TRADE_FEED.to_string(), json!({})));
                } else {
                    self.push_drawer_log(format!(
                        "[PRICE MOVED] Quote {} (total {}) > expected {} (max total {}); purchase halted.",
                        format_credits(matched.unit_price_minor),
                        effective_total.map_or_else(|| "overflow".to_string(), format_credits),
                        format_credits(pq.max_unit_price_minor),
                        format_credits(pq.authorized_total_minor),
                    ));
                }
            }
        }

        self.sort_trade_listings_for_quest();
        if visual_changed {
            self.bump_scene();
        }
        queries
    }

    /// Observe `tacsim.content` manifest or pack JSON keyframe (`SSH-C-02`, `C-18`).
    /// Always requests packs with `{"op":"pack","id":id}` and deduplicates requests so
    /// manifests never trigger an infinite request storm.
    pub fn observe_tacsim_content(&mut self, data: &Value) -> Vec<(String, Value)> {
        let mut followups = Vec::new();
        let Some(obj) = data.as_object() else {
            return followups;
        };
        let op = obj.get("op").and_then(Value::as_str);
        if op == Some("manifest") || (op != Some("pack") && obj.get("packs").is_some()) {
            if let Some(packs) = obj.get("packs").and_then(Value::as_array) {
                let mut next_manifest = HashMap::new();
                for p in packs {
                    let id = p
                        .get("id")
                        .and_then(Value::as_str)
                        .map(sanitize)
                        .unwrap_or_default();
                    if id.is_empty() {
                        continue;
                    }
                    let sha = p
                        .get("sha256")
                        .and_then(Value::as_str)
                        .map(sanitize)
                        .unwrap_or_default();
                    next_manifest.insert(id, sha);
                }
                if next_manifest != self.manifest_packs {
                    self.manifest_packs = next_manifest;
                    self.manifest_gen = self.manifest_gen.wrapping_add(1);
                }
            }
            // Request only the packs we actually need right now (goods, active neighborhood, active ship unit).
            let wanted = [
                "goods".to_string(),
                self.pack_id.clone(),
                self.ship_unit_id.clone(),
            ];
            for id in wanted {
                let available =
                    self.manifest_packs.is_empty() || self.manifest_packs.contains_key(&id);
                if available && self.try_enqueue_pack(&id) {
                    followups.push((
                        TACSIM_CONTENT_FEED.to_string(),
                        json!({ "op": "pack", "id": id }),
                    ));
                }
            }
            return followups;
        }

        let id = obj
            .get("id")
            .and_then(Value::as_str)
            .map(sanitize)
            .unwrap_or_default();
        let requested_id = obj
            .get("requested_id")
            .and_then(Value::as_str)
            .map(sanitize)
            .unwrap_or_default();
        let effective_id = if !id.is_empty() {
            id.clone()
        } else {
            requested_id.clone()
        };
        if !id.is_empty() {
            self.in_flight_packs.remove(&id);
        }
        if !requested_id.is_empty() {
            self.in_flight_packs.remove(&requested_id);
        }

        if obj.get("found").and_then(Value::as_bool) == Some(false) {
            // Do NOT mark missing packs as loaded; record the current manifest generation so
            // duplicate identical manifests don't spin, while a new manifest/revision or revisit retries.
            if !effective_id.is_empty() {
                self.missing_packs
                    .insert(effective_id.clone(), self.manifest_gen);
            }
            if !requested_id.is_empty() && requested_id != effective_id {
                self.missing_packs.insert(requested_id, self.manifest_gen);
            }
            return followups;
        }

        let category = obj.get("category").and_then(Value::as_str).unwrap_or("");
        let parsed_pack = match obj.get("json") {
            Some(Value::String(s)) => serde_json::from_str::<Value>(s).ok(),
            Some(v @ Value::Object(_)) => Some(v.clone()),
            _ => obj.get("pack").cloned(),
        };
        let Some(pack_val) = parsed_pack else {
            return followups;
        };

        let pack_sha = obj
            .get("sha256")
            .and_then(Value::as_str)
            .map(sanitize)
            .filter(|s| !s.is_empty())
            .or_else(|| {
                self.manifest_packs
                    .get(&effective_id)
                    .cloned()
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or_default();
        if !effective_id.is_empty() {
            self.loaded_packs
                .insert(effective_id.clone(), pack_sha.clone());
            self.missing_packs.remove(&effective_id);
        }
        if !requested_id.is_empty() && requested_id != effective_id {
            self.loaded_packs.insert(requested_id.clone(), pack_sha);
            self.missing_packs.remove(&requested_id);
        }

        // 1. Goods pack (`goods.json`: `goods[]` with `measures["si.mass_kg"]`, `measures["si.volume_l"]`)
        if effective_id == "goods" || category == "market" || pack_val.get("goods").is_some() {
            self.parse_goods_pack(&pack_val);
            return followups;
        }

        // 2. Neighborhood pack (`neighborhood.*`: `entries` with `kind: "room"` & `kind: "exit"`)
        if category == "neighborhood" || effective_id.starts_with("neighborhood.") {
            if let Some((rooms, portals)) = parse_neighborhood_pack(&pack_val) {
                let key = if effective_id.is_empty() {
                    self.pack_id.clone()
                } else {
                    effective_id.clone()
                };
                self.neighborhood_cache
                    .insert(key.clone(), (rooms.clone(), portals.clone()));
                if !self.aboard_ship && key == self.pack_id {
                    self.rooms = rooms;
                    self.portals = portals;
                    let target = self.server_location_key.clone();
                    if self.rooms.iter().any(|r| r.entry_id == target) {
                        self.snap_to_room(&target);
                    } else if let Some(by_name) = self
                        .rooms
                        .iter()
                        .find(|r| {
                            let rlow = r.name.to_ascii_lowercase();
                            let tlow = self.room_title.trim().to_ascii_lowercase();
                            rlow == tlow || (!rlow.is_empty() && tlow.contains(&rlow))
                        })
                        .map(|r| r.entry_id.clone())
                    {
                        self.server_location_key.clone_from(&by_name);
                        self.snap_to_room(&by_name);
                    } else if let Some(first) = self.rooms.first().map(|r| r.entry_id.clone()) {
                        self.snap_to_room(&first);
                    }
                    self.rebuild_last_room_interactions();
                    self.bump_scene();
                }
            }
            return followups;
        }

        // 3. Ship Unit interior pack (`entries.*.def.interior`)
        if (category == "unit" || has_unit_interior(&pack_val))
            && let Some((sr, sp)) = parse_unit_interior_pack(&pack_val)
        {
            let slug = normalize_unit_slug(&effective_id);
            if !slug.is_empty() {
                self.unit_cache
                    .insert(slug.clone(), (sr.clone(), sp.clone()));
            }
            if slug == self.ship_unit_id || (self.aboard_ship && slug.is_empty()) {
                self.ship_rooms = sr;
                self.ship_portals = sp;
                if self.aboard_ship {
                    let target = self.server_location_key.clone();
                    if self.ship_rooms.iter().any(|r| r.entry_id == target) {
                        self.snap_to_room(&target);
                    }
                    self.rebuild_last_room_interactions();
                }
                self.bump_scene();
            }
        }

        followups
    }

    fn rebuild_last_room_interactions(&mut self) {
        if let Some(view) = self.last_room_view.clone() {
            let self_name = self.last_self_name.clone();
            self.rebuild_actors_and_interactables(&view, self_name.as_deref());
        }
    }

    fn parse_goods_pack(&mut self, pack_val: &Value) {
        if let Some(goods_arr) = pack_val.get("goods").and_then(Value::as_array) {
            for item in goods_arr {
                let Some(gid_raw) = item.get("id").and_then(Value::as_str) else {
                    continue;
                };
                let gid = sanitize(gid_raw);
                if gid.is_empty() {
                    continue;
                }
                let label = item
                    .get("label")
                    .or_else(|| item.get("name"))
                    .and_then(Value::as_str)
                    .map(sanitize)
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| gid.clone());
                let measures = item.get("measures").and_then(Value::as_object);
                let mass = measures
                    .and_then(|m| m.get("si.mass_kg"))
                    .or_else(|| item.get("unit_mass_kg"))
                    .or_else(|| item.get("mass_kg"))
                    .and_then(Value::as_i64)
                    .unwrap_or(1000)
                    .max(1);
                let vol = measures
                    .and_then(|m| m.get("si.volume_l"))
                    .or_else(|| item.get("unit_volume_dm3"))
                    .or_else(|| item.get("volume_dm3"))
                    .and_then(Value::as_i64)
                    .unwrap_or(1000)
                    .max(1);
                self.goods_meta.insert(gid, (label, mass, vol));
            }
            self.refresh_trade_rows_from_goods_meta();
            return;
        }
        // Legacy `entries` fallback
        if let Some(entries) = pack_val.get("entries").and_then(Value::as_object) {
            for (gid, gval) in entries {
                let clean_gid = sanitize(gid);
                let name = gval
                    .get("label")
                    .or_else(|| gval.get("name"))
                    .and_then(Value::as_str)
                    .map(sanitize)
                    .unwrap_or_else(|| clean_gid.clone());
                let mass = gval
                    .get("unit_mass_kg")
                    .or_else(|| gval.get("mass_kg"))
                    .and_then(Value::as_i64)
                    .unwrap_or(1000)
                    .max(1);
                let vol = gval
                    .get("unit_volume_dm3")
                    .or_else(|| gval.get("volume_dm3"))
                    .and_then(Value::as_i64)
                    .unwrap_or(1000)
                    .max(1);
                self.goods_meta.insert(clean_gid, (name, mass, vol));
            }
            self.refresh_trade_rows_from_goods_meta();
        }
    }

    fn refresh_trade_rows_from_goods_meta(&mut self) {
        let mut changed = false;
        let asks_meta: Vec<Option<(String, i64, i64)>> = self
            .asks
            .iter()
            .map(|a| self.lookup_goods_meta_opt(&a.good_id))
            .collect();
        for (ask, meta) in self.asks.iter_mut().zip(asks_meta) {
            if let Some((label, mass_kg, vol_dm3)) = meta {
                if ask.unit_mass_kg != mass_kg || ask.unit_vol_dm3 != vol_dm3 {
                    ask.unit_mass_kg = mass_kg;
                    ask.unit_vol_dm3 = vol_dm3;
                    changed = true;
                }
                if (ask.good_name.is_empty() || ask.good_name == ask.good_id)
                    && ask.good_name != label
                {
                    ask.good_name = label;
                    changed = true;
                }
            }
        }
        let bids_meta: Vec<Option<(String, i64, i64)>> = self
            .bids
            .iter()
            .map(|b| self.lookup_goods_meta_opt(&b.good_id))
            .collect();
        for (bid, meta) in self.bids.iter_mut().zip(bids_meta) {
            if let Some((label, _, _)) = meta
                && (bid.good_name.is_empty() || bid.good_name == bid.good_id)
                && bid.good_name != label
            {
                bid.good_name = label;
                changed = true;
            }
        }
        let lots_meta: Vec<Option<(String, i64, i64)>> = self
            .cargo_lots
            .iter()
            .map(|l| self.lookup_goods_meta_opt(&l.good_id))
            .collect();
        for (lot, meta) in self.cargo_lots.iter_mut().zip(lots_meta) {
            if let Some((label, _, _)) = meta
                && (lot.good_name.is_empty() || lot.good_name == lot.good_id)
                && lot.good_name != label
            {
                lot.good_name = label;
                changed = true;
            }
        }
        if changed {
            self.bump_scene();
        }
    }

    fn lookup_goods_meta_opt(&self, gid: &str) -> Option<(String, i64, i64)> {
        if let Some(v) = self.goods_meta.get(gid) {
            return Some(v.clone());
        }
        let stripped = gid
            .strip_prefix("tacsim:")
            .or_else(|| gid.strip_prefix("tacsim."))
            .unwrap_or(gid);
        if let Some(v) = self.goods_meta.get(stripped) {
            return Some(v.clone());
        }
        None
    }

    fn lookup_goods_meta(&self, gid: &str) -> (String, i64, i64) {
        self.lookup_goods_meta_opt(gid)
            .unwrap_or_else(|| (gid.to_string(), 1000, 1000))
    }

    /// Advance 20 Hz continuous avatar walking, turning, doorway collision, and BFS room sync.
    pub fn tick(&mut self, dt: Duration, now: Instant) -> Vec<String> {
        let mut cmds = Vec::new();
        let before_bubbles = self.speech_bubbles.len();
        self.speech_bubbles.retain(|_, (_, exp)| *exp > now);
        if self.speech_bubbles.len() != before_bubbles {
            self.bump_scene();
        }

        // Throttle bridge holotank pulse animation to ~2 Hz and ONLY when on a ship bridge
        // in 2.5D cutaway mode so idle SSH sessions produce zero PNG re-encodes (`SSH-H-01`).
        if self.aboard_ship
            && self.camera == WalkCamera::Cutaway25D
            && self.active_location_key == "bridge"
        {
            match self.last_pulse_at {
                None => {
                    self.last_pulse_at = Some(now);
                }
                Some(t) if now.saturating_duration_since(t) >= HOLOTANK_PULSE_INTERVAL => {
                    self.last_pulse_at = Some(now);
                    self.pulse_phase = (self.pulse_phase + 1) % 3;
                    self.bump_scene();
                }
                Some(_) => {}
            }
        }

        if let Some((_, sent_at)) = &self.pending_move_target
            && now.saturating_duration_since(*sent_at) > PENDING_MOVE_TIMEOUT
        {
            self.pending_move_target = None;
            self.pending_interaction = None;
        }
        if let Some(pi) = &self.pending_interaction
            && now.saturating_duration_since(pi.created_at) > PENDING_MOVE_TIMEOUT * 2
        {
            self.pending_interaction = None;
        }

        if self.drawer.is_some() {
            return cmds;
        }

        let secs = dt.as_secs_f64().clamp(0.0, 0.25);
        if self.held_turn != 0 {
            let delta =
                ((TURN_MDEG_PER_S as f64) * secs * f64::from(self.held_turn)).round() as i64;
            if delta != 0 {
                self.you_heading_mdeg = (self.you_heading_mdeg + delta).rem_euclid(360_000);
                self.bump_scene();
            }
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
    ///
    /// `arrows_held` is true when Kitty keyboard protocol is pushed (arrow press/release supported);
    /// `letters_held` is true when Kitty flag 8 is active (letter press/release supported, `SSH-M-01`).
    pub fn handle_key(
        &mut self,
        key: Key,
        arrows_held: bool,
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
                self.pending_interaction = None;
                out.close_pixel_view = true;
            }
            Key::Char('v' | 'V') => {
                self.camera = self.camera.next();
                self.bump_scene();
            }
            Key::Tab => {
                if !self.interactables.is_empty() {
                    self.focus_idx = (self.focus_idx + 1) % self.interactables.len();
                    self.bump_scene();
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
            Key::Char('j' | 'J') => {
                self.open_drawer(PortDrawerKind::Jobs, manifest_panels, &mut out.queries, now);
            }
            Key::Char('i' | 'I') => {
                self.open_drawer(
                    PortDrawerKind::Industry,
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
                out.redraw = false;
            }
            Key::Release(Arrow::Down) | Key::CharRelease('s' | 'S') => {
                if self.held_fwd < 0 {
                    self.held_fwd = 0;
                }
                out.redraw = false;
            }
            Key::Release(Arrow::Left) | Key::CharRelease('a' | 'A') => {
                if self.held_turn < 0 {
                    self.held_turn = 0;
                }
                out.redraw = false;
            }
            Key::Release(Arrow::Right) | Key::CharRelease('d' | 'D') => {
                if self.held_turn > 0 {
                    self.held_turn = 0;
                }
                out.redraw = false;
            }
            Key::CharRelease('q' | 'Q') => {
                if self.held_strafe < 0 {
                    self.held_strafe = 0;
                }
                out.redraw = false;
            }
            Key::CharRelease('x' | 'X') => {
                if self.held_strafe > 0 {
                    self.held_strafe = 0;
                }
                out.redraw = false;
            }
            // Suppress OS key-repeats when Kitty hold-release is active (`SSH-M-01`)
            Key::Repeat(Arrow::Up | Arrow::Down | Arrow::Left | Arrow::Right) if arrows_held => {
                out.redraw = false;
            }
            Key::CharRepeat(
                'w' | 'W' | 's' | 'S' | 'a' | 'A' | 'd' | 'D' | 'q' | 'Q' | 'x' | 'X',
            ) if letters_held => {
                out.redraw = false;
            }
            Key::Up | Key::Repeat(Arrow::Up) => {
                if arrows_held {
                    self.held_fwd = 1;
                    out.redraw = false;
                } else {
                    self.step_avatar(1.35, 0.0, now, &mut out.commands);
                }
            }
            Key::Down | Key::Repeat(Arrow::Down) => {
                if arrows_held {
                    self.held_fwd = -1;
                    out.redraw = false;
                } else {
                    self.step_avatar(-1.15, 0.0, now, &mut out.commands);
                }
            }
            Key::Char('w' | 'W') | Key::CharRepeat('w' | 'W') => {
                if letters_held {
                    self.held_fwd = 1;
                    out.redraw = false;
                } else {
                    self.step_avatar(1.35, 0.0, now, &mut out.commands);
                }
            }
            Key::Char('s' | 'S') | Key::CharRepeat('s' | 'S') => {
                if letters_held {
                    self.held_fwd = -1;
                    out.redraw = false;
                } else {
                    self.step_avatar(-1.15, 0.0, now, &mut out.commands);
                }
            }
            Key::Left | Key::Repeat(Arrow::Left) => {
                if arrows_held {
                    self.held_turn = -1;
                    out.redraw = false;
                } else {
                    self.you_heading_mdeg = (self.you_heading_mdeg - 22_500).rem_euclid(360_000);
                    self.bump_scene();
                }
            }
            Key::Right | Key::Repeat(Arrow::Right) => {
                if arrows_held {
                    self.held_turn = 1;
                    out.redraw = false;
                } else {
                    self.you_heading_mdeg = (self.you_heading_mdeg + 22_500).rem_euclid(360_000);
                    self.bump_scene();
                }
            }
            Key::Char('a' | 'A') | Key::CharRepeat('a' | 'A') => {
                if letters_held {
                    self.held_turn = -1;
                    out.redraw = false;
                } else {
                    self.you_heading_mdeg = (self.you_heading_mdeg - 22_500).rem_euclid(360_000);
                    self.bump_scene();
                }
            }
            Key::Char('d' | 'D') | Key::CharRepeat('d' | 'D') => {
                if letters_held {
                    self.held_turn = 1;
                    out.redraw = false;
                } else {
                    self.you_heading_mdeg = (self.you_heading_mdeg + 22_500).rem_euclid(360_000);
                    self.bump_scene();
                }
            }
            Key::Char('q' | 'Q') | Key::CharRepeat('q' | 'Q') => {
                if letters_held {
                    self.held_strafe = -1;
                    out.redraw = false;
                } else {
                    self.step_avatar(0.0, -1.2, now, &mut out.commands);
                }
            }
            Key::Char('x' | 'X') | Key::CharRepeat('x' | 'X') => {
                if letters_held {
                    self.held_strafe = 1;
                    out.redraw = false;
                } else {
                    self.step_avatar(0.0, 1.2, now, &mut out.commands);
                }
            }
            _ => {
                out.consumed = false;
                out.redraw = false;
            }
        }

        out
    }

    fn resolved_feed_quest_target(&self) -> Option<String> {
        if self.aboard_ship {
            if self.seated_at_helm {
                return None;
            }
            let docked_at_station = self.docked_port_slug().is_some();
            let docked_at_objective_port = self
                .docked_port_slug()
                .zip(
                    self.quest_room_entry
                        .as_deref()
                        .and_then(infer_station_port_from_room_entry),
                )
                .is_some_and(|(docked, want)| docked.eq_ignore_ascii_case(want));
            let mining_blocking = self
                .quest_mining_status
                .as_ref()
                .is_some_and(|m| !m.mine_done);
            let raw_target = if mining_blocking {
                if docked_at_station && !self.settled_pad {
                    "bridge"
                } else {
                    "bay"
                }
            } else if self.quest_ship_target.as_deref() == Some("airlock")
                || docked_at_objective_port
            {
                "airlock"
            } else if self.quest_mining_status.is_some() {
                "bridge"
            } else {
                self.quest_ship_target
                    .as_deref()
                    .or_else(|| self.quest_dest_port.as_ref().map(|_| "bridge"))?
            };
            if self.ship_rooms.iter().any(|r| r.entry_id == raw_target) {
                Some(raw_target.to_string())
            } else if raw_target == "bay" && self.ship_rooms.iter().any(|r| r.entry_id == "hold") {
                Some("hold".to_string())
            } else {
                None
            }
        } else {
            self.resolved_station_quest_target().map(|(id, _)| id)
        }
    }

    fn parse_station_room_prefix_role(entry: &str) -> Option<(&str, &str)> {
        let rest = entry.strip_prefix("room.")?;
        let prefix_char = rest.chars().next()?;
        if !matches!(
            prefix_char.to_ascii_lowercase(),
            'c' | 'p' | 'l' | 'd' | 't'
        ) {
            return None;
        }
        let after_prefix = &rest[prefix_char.len_utf8()..];
        let digits_len = after_prefix.bytes().take_while(u8::is_ascii_digit).count();
        if digits_len == 0 {
            return None;
        }
        let role = after_prefix[digits_len..].strip_prefix('_')?;
        if role.is_empty() {
            return None;
        }
        let prefix_end = "room.".len() + prefix_char.len_utf8();
        Some((&entry[..prefix_end], role))
    }

    /// Match a canonical corridor station room entry (e.g. `room.c08_chandlery`) onto a loaded
    /// room sharing the exact same station prefix and role suffix (e.g. `room.c06_chandlery`)
    /// without ever crossing stations.
    fn find_same_station_role_room(&self, raw_target: &str) -> Option<&RoomGeom> {
        let (prefix, role) = Self::parse_station_room_prefix_role(raw_target)?;
        self.rooms.iter().find(|r| {
            Self::parse_station_room_prefix_role(&r.entry_id).is_some_and(|(p, r_role)| {
                p.eq_ignore_ascii_case(prefix) && r_role.eq_ignore_ascii_case(role)
            })
        })
    }

    /// Resolve the active quest's target room onto the currently loaded station map
    /// (returns `None` while aboard ship), returning `(local_room_entry, is_boarding_target)`
    /// for 2.5D station markers and room-arrival callouts.
    #[must_use]
    pub fn resolved_station_quest_target(&self) -> Option<(String, bool)> {
        if self.aboard_ship {
            return None;
        }
        if self.needs_surface_expedition_boarding() {
            return self
                .rooms
                .iter()
                .find(|r| r.entry_id.to_ascii_lowercase().contains("berth_gallery"))
                .map(|r| (r.entry_id.clone(), true));
        }
        let raw_target = self.quest_room_entry.as_deref()?;
        if self.rooms.iter().any(|r| r.entry_id == raw_target) {
            return Some((raw_target.to_string(), false));
        }
        if let Some(role_room) = self.find_same_station_role_room(raw_target) {
            return Some((role_room.entry_id.clone(), false));
        }
        if infer_station_port_from_room_entry(raw_target).is_some() {
            if self.needs_origin_port_procurement()
                && let Some(exchange) = self
                    .rooms
                    .iter()
                    .find(|r| r.entry_id.to_ascii_lowercase().contains("exchange"))
                    .or_else(|| {
                        self.rooms.iter().find(|r| {
                            let low = r.entry_id.to_ascii_lowercase();
                            low.contains("chandlery") || low.contains("service")
                        })
                    })
            {
                return Some((exchange.entry_id.clone(), false));
            }
            return self
                .rooms
                .iter()
                .find(|r| r.entry_id.to_ascii_lowercase().contains("berth_gallery"))
                .map(|r| (r.entry_id.clone(), true));
        }
        None
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

        let feed_quest_target = self.resolved_feed_quest_target();
        let rooms_json: Vec<Value> = active_rooms
            .iter()
            .map(|r| {
                let is_quest_target = feed_quest_target
                    .as_deref()
                    .is_some_and(|e| e == r.entry_id);
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
                    "quest_target": is_quest_target,
                    "quest_turn_in": is_quest_target && !self.aboard_ship && self.quest_room_turn_in,
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

        // Render REAL moored ships from `port.trade.berth.docked_hulls` outside the active
        // berth gallery's exterior glass side (never fabricated ships, `SSH-H-02`).
        let mut moored_json = Vec::new();
        if !self.aboard_ship
            && let Some(berth_room) = self.rooms.iter().find(|r| r.kind == "berth")
        {
            let south_glass = self.room_glass_is_south(berth_room);
            let moored_y = if south_glass {
                berth_room.min_y - 10.0
            } else {
                berth_room.max_y + 10.0
            };
            let span_x = (berth_room.max_x - berth_room.min_x).max(12.0);
            if !self.berth_docked_hulls.is_empty() {
                let count = self.berth_docked_hulls.len().min(8);
                for (idx, hull) in self.berth_docked_hulls.iter().take(count).enumerate() {
                    let frac = ((idx as f64) + 1.0) / ((count as f64) + 1.0);
                    let hx = berth_room.min_x + span_x * frac;
                    let u_low = hull.unit.to_ascii_lowercase();
                    let style = if u_low.contains("corvette")
                        || u_low.contains("cutter")
                        || u_low.contains("raider")
                    {
                        "corvette"
                    } else {
                        "freighter"
                    };
                    let is_own = self
                        .own_hull_id
                        .as_deref()
                        .is_some_and(|oid| !oid.is_empty() && oid == hull.id)
                        || (!self.own_hull_name.is_empty() && hull.name == self.own_hull_name);
                    moored_json.push(json!({
                        "x_m": hx,
                        "y_m": moored_y,
                        "name": hull.name,
                        "style": style,
                        "own": is_own,
                    }));
                }
            } else if self.own_hull_docked_here && !self.own_hull_name.is_empty() {
                let (cx, _) = berth_room.center();
                moored_json.push(json!({
                    "x_m": cx,
                    "y_m": moored_y,
                    "name": self.own_hull_name,
                    "style": "freighter",
                    "own": true,
                }));
            }
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
                let qm = if a.npc {
                    self.find_quest_marker(&a.id, &a.name)
                } else {
                    None
                };
                let (quest_marker, quest_gold) = match qm {
                    Some(m) => (
                        Value::String(m.symbol.clone()),
                        Value::Bool(m.marker != "in_progress"),
                    ),
                    None => (Value::Null, Value::Bool(false)),
                };
                json!({
                    "x_m": a.x_m,
                    "y_m": a.y_m,
                    "floor": a.floor,
                    "name": a.name,
                    "npc": a.npc,
                    "speech": speech,
                    "quest_marker": quest_marker,
                    "quest_gold": quest_gold,
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
            (self.stowage.cargo_kg.saturating_mul(1000) / self.stowage.max_cargo_kg).clamp(0, 1000)
        } else {
            0
        };

        let subtitle = if self.aboard_ship {
            format!(
                "{}  |  {}",
                self.hull_name,
                if self.settled_pad {
                    "Settled Pad"
                } else if self.docked {
                    "Docked"
                } else {
                    "Under Way"
                }
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

    fn room_glass_is_south(&self, room: &RoomGeom) -> bool {
        let mut has_north_door = false;
        let mut has_south_door = false;
        for p in &self.portals {
            if p.floor != room.floor && p.stairs_to != Some(room.floor) {
                continue;
            }
            if p.x_m >= room.min_x - 1.0 && p.x_m <= room.max_x + 1.0 {
                if (p.y_m - room.max_y).abs() <= 1.6 {
                    has_north_door = true;
                }
                if (p.y_m - room.min_y).abs() <= 1.6 {
                    has_south_door = true;
                }
            }
        }
        has_north_door && !has_south_door
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
        let wallet = if self.balance_text.is_empty() {
            format_credits(self.credits_minor)
        } else {
            self.balance_text.clone()
        };
        let mut top = format!(
            "{mode_word} ({cam_name})  |  {}  |  {wallet}  |  drawing {encoder_name}",
            self.room_title,
        );
        if let Some(hop) = self.next_quest_hop() {
            let hop_word = if hop.hops == 1 { "hop" } else { "hops" };
            top.push_str(&format!(
                "  |  [QUEST -> {} ({} {hop_word})]",
                hop.next_room_name, hop.hops
            ));
        }
        if self.aboard_ship
            && let Some(ref target_comp) = self.quest_ship_target
            && !target_comp.eq_ignore_ascii_case(&self.active_location_key)
        {
            let lower = target_comp.to_ascii_lowercase();
            let deck_lbl = match lower.as_str() {
                "bridge" => "Bridge",
                "bay" => "Vehicle Bay (+tv/bay in)",
                "airlock" => "Airlock (+ship/leave)",
                other => other,
            };
            top.push_str(&format!("  |  [QUEST -> Deck: {deck_lbl}]"));
        }
        if self.aboard_ship
            && self.active_location_key == "bay"
            && let Some(qm) = &self.quest_mining_status
        {
            let chk = |done: bool| if done { "[x]" } else { "[ ]" };
            let rig_cmd = if qm.mine_done {
                "+tv/1 recover go"
            } else {
                "+tv/1 launch go"
            };
            top.push_str(&format!(
                "  |  [QUEST] Rig Bay: {} Scan {} Sound {} Mine {} ({rig_cmd})",
                chk(qm.scan_done),
                chk(qm.sound_done),
                chk(qm.mine_done),
                qm.target_ore_slug
            ));
            if self.stowage_loaded {
                let ore_qty = self.hold_commodity_quantity(&qm.target_ore_slug);
                top.push_str(&format!(
                    "  |  Hold: {ore_qty}t {}",
                    qm.target_ore_slug.replace('_', " ")
                ));
            }
            if qm.mine_done {
                if let Some(dest) = &self.quest_dest_port {
                    top.push_str(&format!(
                        "  |  Return: {} (+ship/plan {})",
                        dest.port_label, dest.port_id
                    ));
                } else {
                    let ret_slug = self
                        .quest_room_entry
                        .as_deref()
                        .and_then(infer_station_port_from_room_entry)
                        .unwrap_or("ceres");
                    let ret_label = CORRIDORS
                        .iter()
                        .find(|(id, _, _)| id.eq_ignore_ascii_case(ret_slug))
                        .map_or("Ceres Freeport", |(_, lbl, _)| *lbl);
                    top.push_str(&format!("  |  Return: {ret_label} (+ship/plan {ret_slug})"));
                }
            }
        } else if self.aboard_ship
            && let Some(dest) = &self.quest_dest_port
        {
            top.push_str(&format!("  |  [QUEST] Dest: {}", dest.port_label));
        }
        let bottom = if self.drawer.is_some() {
            "Tab: tab  Up/Dn: row  [/]: qty  PgUp/PgDn: log  Enter: run  Esc: close".to_string()
        } else {
            "WASD/Arr: walk  v: 2.5D/1P  Tab/e: [E]  m:trade j:jobs i:works k:sail o:coop  Enter:chat  Esc:close"
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
            let win = visible_window(self.interactables.len(), self.focus_idx, 6);
            for idx in win {
                if let Some(it) = self.interactables.get(idx) {
                    let focused = idx == self.focus_idx;
                    let prefix = if focused { "[E] > " } else { "      " };
                    let is_quest = it.label.starts_with("[?]")
                        || it.label.starts_with("[!]")
                        || it.label.starts_with("[Q]");
                    let style = if focused {
                        Style::default()
                            .fg(if is_quest {
                                Color::Yellow
                            } else {
                                Color::Green
                            })
                            .add_modifier(Modifier::BOLD)
                    } else if is_quest {
                        Style::default()
                            .fg(Color::Yellow)
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
        if self.drawer.is_none() || body.width < 30 || body.height < 8 {
            return;
        }
        let w = body.width.saturating_sub(4).min(108);
        let h = body.height.saturating_sub(2).min(26);
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
                self.drawer_log_scroll = 0;
                return out;
            }
            Key::BackTab => {
                self.drawer = Some(kind.prev());
                self.cursor_row = 0;
                self.drawer_log_scroll = 0;
                return out;
            }
            Key::PgUp => {
                let max_scroll = self.drawer_log.len().saturating_sub(1);
                self.drawer_log_scroll = self.drawer_log_scroll.saturating_add(4).min(max_scroll);
                return out;
            }
            Key::PgDn => {
                self.drawer_log_scroll = self.drawer_log_scroll.saturating_sub(4);
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
                        self.custom_trade_qty = None;
                        self.qty_preset_idx = self.qty_preset_idx.saturating_sub(1);
                    }
                    Key::Char(']') => {
                        self.custom_trade_qty = None;
                        self.qty_preset_idx = (self.qty_preset_idx + 1).min(QTY_PRESETS.len() - 1);
                    }
                    Key::Char('g' | 'G') => {
                        if !self.apply_quest_trade_jump()
                            && self.quest_refit_hold_readiness().is_some_and(|r| !r.ready)
                            && let Some(idx) =
                                self.bids.iter().position(|b| self.is_refit_clear_bid(b))
                        {
                            self.trade_subtab = 1;
                            self.cursor_row = idx;
                        }
                    }
                    Key::Char('c' | 'C') => {
                        if self.quest_refit_hold_readiness().is_some_and(|r| r.ready) {
                            self.drawer = Some(PortDrawerKind::Coop);
                            self.cursor_row = 0;
                            self.drawer_log_scroll = 0;
                        }
                    }
                    Key::Char('d' | 'D') => {
                        if self.quest_dest_port.is_some() {
                            self.drawer = Some(PortDrawerKind::Sailing);
                            self.cursor_row = 0;
                            self.drawer_log_scroll = 0;
                            self.apply_quest_destination_jump();
                        }
                    }
                    Key::Char('r') => {
                        self.trade_asked_at = None;
                        self.awaiting_trade_page = false;
                        self.ensure_queries(manifest_panels, &mut out.queries, now);
                    }
                    Key::Char('>' | 'p') => {
                        if let Some(after) = self.trade_next_after {
                            self.awaiting_trade_page = true;
                            out.queries
                                .push((PORT_TRADE_FEED.to_string(), json!({ "after": after })));
                            self.push_drawer_log(format!(
                                "[TRADE] Loading next page after #{after}..."
                            ));
                        }
                    }
                    // Strictly bind `b` to Buy Asks, `s` to Sell Bids, and `Enter` to active subtab (`SSH-M-05`).
                    Key::Char('b' | 'B') => {
                        if self.trade_subtab == 0 {
                            self.execute_trade_buy(now, &mut out);
                        }
                    }
                    Key::Char('s' | 'S') => {
                        if self.trade_subtab == 1 {
                            self.execute_trade_sell(&mut out);
                        }
                    }
                    Key::Enter => {
                        if self.trade_subtab == 0 {
                            self.execute_trade_buy(now, &mut out);
                        } else {
                            self.execute_trade_sell(&mut out);
                        }
                    }
                    _ => {}
                }
            }
            PortDrawerKind::Jobs => match key {
                Key::Up | Key::Char('k') => {
                    self.cursor_row = self.cursor_row.saturating_sub(1);
                }
                Key::Down | Key::Char('j') => {
                    self.cursor_row = (self.cursor_row + 1).min(DOC48_JOB_ACTIONS.len() - 1);
                }
                Key::Char('o') => {
                    out.commands.push("+locker/open".to_string());
                    self.push_drawer_log("> +locker/open".to_string());
                }
                Key::Char('f') => {
                    if self.quest_mining_status.is_some() {
                        out.commands.push("+market/jobs survey".to_string());
                        self.push_drawer_log("> +market/jobs survey".to_string());
                    } else if self.quest_dest_port.is_some() {
                        out.commands.push("+market/jobs haul".to_string());
                        self.push_drawer_log("> +market/jobs haul".to_string());
                    }
                }
                Key::Enter => {
                    if let Some(&(label, cmd)) = DOC48_JOB_ACTIONS.get(self.cursor_row) {
                        out.commands.push(cmd.to_string());
                        self.push_drawer_log(format!("[Jobs] {label}: {cmd}"));
                    }
                }
                _ => {}
            },
            PortDrawerKind::Industry => match key {
                Key::Up | Key::Char('k') => {
                    self.cursor_row = self.cursor_row.saturating_sub(1);
                }
                Key::Down | Key::Char('j') => {
                    self.cursor_row = (self.cursor_row + 1).min(DOC48_INDUSTRY_ACTIONS.len() - 1);
                }
                Key::Char('m') => {
                    out.commands.push("+market/mine".to_string());
                    self.push_drawer_log("> +market/mine".to_string());
                }
                Key::Char('p') => {
                    if let Some(ref act) = self.quest_coop_action
                        && act.command.starts_with("+coop/refit")
                    {
                        out.commands.push(act.command.clone());
                        self.push_drawer_log(format!("> {}", act.command));
                    }
                }
                Key::Char('h') => {
                    if let Some(need) = self.effective_quest_trade_need() {
                        let cmd = format!("+market/history tacsim:{}", need.commodity_slug);
                        out.commands.push(cmd.clone());
                        self.push_drawer_log(format!("> {cmd}"));
                    }
                }
                Key::Enter => {
                    if let Some(&(label, cmd)) = DOC48_INDUSTRY_ACTIONS.get(self.cursor_row) {
                        out.commands.push(cmd.to_string());
                        self.push_drawer_log(format!("[Works] {label}: {cmd}"));
                    }
                }
                _ => {}
            },
            PortDrawerKind::Sailing => match key {
                Key::Up | Key::Char('k') => {
                    let ordered = self.ordered_sailing_corridor_indices();
                    let cur_pos = ordered
                        .iter()
                        .position(|&ci| ci == self.corridor_idx)
                        .unwrap_or(0);
                    let next_pos = cur_pos.saturating_sub(1);
                    if let Some(&ci) = ordered.get(next_pos) {
                        self.corridor_idx = ci;
                    }
                }
                Key::Down | Key::Char('j') => {
                    let ordered = self.ordered_sailing_corridor_indices();
                    let cur_pos = ordered
                        .iter()
                        .position(|&ci| ci == self.corridor_idx)
                        .unwrap_or(0);
                    let next_pos = (cur_pos + 1).min(ordered.len().saturating_sub(1));
                    if let Some(&ci) = ordered.get(next_pos) {
                        self.corridor_idx = ci;
                    }
                }
                Key::Char('r') => {
                    self.queue_depart = !self.queue_depart;
                }
                Key::Char('g' | 'G') => {
                    self.apply_quest_destination_jump();
                }
                Key::Char('c' | 'C') => {
                    out.commands.push("+chart/ground".to_string());
                    self.push_drawer_log("[Sailing] Ground Chart: +chart/ground".to_string());
                }
                Key::Char('b' | 'B') => {
                    self.drawer = Some(PortDrawerKind::Trade);
                }
                Key::Char('u') => {
                    if let Some((dest_id, _, _)) = CORRIDORS.get(self.corridor_idx) {
                        out.commands.push(format!("+ship/undock {dest_id}"));
                        self.push_drawer_log(format!("> +ship/undock {dest_id}"));
                    }
                }
                Key::Char('d') => {
                    // Guard `+helm/dock` while ashore (`SSH-L-01`); while underway in space
                    // (`!self.docked`), prefer the selected corridor (`H-40`) over any stale venue.
                    if !self.aboard_ship {
                        self.push_drawer_log(
                            "[Sailing] Board your ship before issuing +helm/dock.".to_string(),
                        );
                    } else if let Some(target_id) = if !self.docked {
                        CORRIDORS
                            .get(self.corridor_idx)
                            .map(|(id, _, _)| *id)
                            .or(self.current_venue.as_deref())
                    } else {
                        self.current_venue
                            .as_deref()
                            .or_else(|| CORRIDORS.get(self.corridor_idx).map(|(id, _, _)| *id))
                    } {
                        out.commands.push(format!("+helm/dock {target_id}"));
                        self.push_drawer_log(format!("> +helm/dock {target_id}"));
                    }
                }
                Key::Enter => {
                    if let Some((dest_id, label, _)) = CORRIDORS.get(self.corridor_idx) {
                        let cmd = if self.queue_depart {
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
                Key::Char('p') => {
                    out.commands.push("+coop/refit prospector".to_string());
                    self.push_drawer_log("> +coop/refit prospector".to_string());
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
                        if cmd == "SIM START" {
                            // Close drawer proactively so entering cockpit never traps keys (`SSH-C-04`).
                            self.close_drawer();
                        }
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
                Key::Char('q' | 'Q') => {
                    if let Some(m) = self.selected_npc_quest_marker().cloned() {
                        self.close_drawer();
                        out.open_quest_for_npc = Some((m.npc_id, m.npc_name));
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

    fn execute_trade_buy(&mut self, now: Instant, out: &mut WalkKeyOutcome) {
        let Some(ask) = self.asks.get(self.cursor_row).cloned() else {
            return;
        };
        if ask.quantity <= 0 {
            self.push_drawer_log(format!("[BUY] #{} has no available stock.", ask.listing_id));
            return;
        }
        let preset = self.active_trade_qty_setting();
        let want_qty = if preset == 0 {
            if !self.stowage_loaded {
                self.push_drawer_log(
                    "[BUY] Hold stowage unavailable; press r to refresh before using Max Fit."
                        .to_string(),
                );
                return;
            }
            let max_fit = self.max_fit_for_ask(&ask);
            if max_fit <= 0 {
                self.push_drawer_log(format!(
                    "[BUY] Cannot fit or afford 1 unit of {} (max fit: 0).",
                    ask.good_name
                ));
                return;
            }
            max_fit
        } else {
            preset.min(ask.quantity)
        };
        if want_qty <= 0 {
            return;
        }
        let Some(authorized_total_minor) = ask.unit_price_minor.checked_mul(want_qty) else {
            self.push_drawer_log("[BUY] Quote total overflow; purchase halted.".to_string());
            return;
        };
        let prior_quote_ids = self.own_quotes.iter().map(|q| q.quote_id.clone()).collect();
        self.pending_quote = Some(PendingQuote {
            listing_id: ask.listing_id,
            quantity: want_qty,
            max_unit_price_minor: ask.unit_price_minor,
            authorized_total_minor,
            prior_quote_ids,
            created_at: now,
        });
        // Canonical server syntax (`mush-market/src/commands.rs:700-711`): `+market/quote <id>=<qty>` (`C-20`).
        out.commands
            .push(format!("+market/quote {}={want_qty}", ask.listing_id));
        self.awaiting_trade_page = false;
        out.queries.push((PORT_TRADE_FEED.to_string(), json!({})));
        self.push_drawer_log(format!(
            "[QUOTE] Requesting {want_qty} x {} @ {}...",
            ask.good_name,
            format_credits(ask.unit_price_minor)
        ));
    }

    fn execute_trade_sell(&mut self, out: &mut WalkKeyOutcome) {
        let Some(bid) = self.bids.get(self.cursor_row).cloned() else {
            return;
        };
        if bid.quantity_wanted <= 0 {
            self.push_drawer_log(format!("[SELL] Bid #{} is already filled.", bid.listing_id));
            return;
        }
        if !self.stowage_loaded {
            self.push_drawer_log(
                "[SELL] Hold stowage unavailable; press r to refresh before selling.".to_string(),
            );
            return;
        }
        let held = self
            .cargo_lots
            .iter()
            .filter(|c| c.good_id == bid.good_id)
            .map(|c| c.quantity)
            .sum::<i64>();
        if held <= 0 {
            self.push_drawer_log(format!("[SELL] Hold carries 0 units of {}.", bid.good_name));
            return;
        }
        let preset = self.active_trade_qty_setting();
        let sell_qty = if preset == 0 {
            held.min(bid.quantity_wanted)
        } else {
            preset.min(held).min(bid.quantity_wanted)
        };
        if sell_qty <= 0 {
            return;
        }
        // Canonical server syntax (`mush-market/src/commands.rs:889-910`): `+market/sell <id>=<qty>` (`C-20`).
        out.commands
            .push(format!("+market/sell {}={sell_qty}", bid.listing_id));
        self.awaiting_trade_page = false;
        out.queries.push((PORT_TRADE_FEED.to_string(), json!({})));
        self.push_drawer_log(format!(
            "[SELL] Selling {sell_qty} x {} into bid #{}...",
            bid.good_name, bid.listing_id
        ));
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
        // Keep at most ONE movement command in flight; if a room transition is still
        // unacknowledged, defer the interaction until authoritative `OutputEvent::Room` lands.
        if self.pending_move_target.is_some()
            || self.active_location_key != self.server_location_key
        {
            if self.pending_move_target.is_none() {
                let from = self.server_location_key.clone();
                let dest = self.active_location_key.clone();
                let _ = self.try_send_single_move_hop(&from, &dest, now, &mut out.commands);
            }
            self.pending_interaction = Some(PendingInteraction {
                action: target.action,
                target_room: self.active_location_key.clone(),
                manifest_panels: manifest_panels.to_vec(),
                created_at: now,
            });
            return;
        }

        self.execute_interact_action(target.action, manifest_panels, now, out);
    }

    fn execute_interact_action(
        &mut self,
        action: InteractAction,
        manifest_panels: &[String],
        now: Instant,
        out: &mut WalkKeyOutcome,
    ) {
        match action {
            InteractAction::OpenDrawer(kind) => {
                self.open_drawer(kind, manifest_panels, &mut out.queries, now);
            }
            InteractAction::OpenNpc { id, name } => {
                if let Some(m) = self.find_quest_marker(&id, &name)
                    && matches!(m.marker.as_str(), "available" | "turn_in")
                {
                    out.open_quest_for_npc = Some((id, name));
                    return;
                }
                self.selected_npc_name = name;
                self.open_drawer(PortDrawerKind::Npc, manifest_panels, &mut out.queries, now);
            }
            InteractAction::Command(cmd) => {
                out.commands.push(cmd.to_string());
            }
            InteractAction::TakeBridgeHelm => {
                self.seated_at_helm = true;
                out.commands.push("+helm/station command".to_string());
                out.enter_helm_cockpit = true;
            }
        }
    }

    fn try_send_single_move_hop(
        &mut self,
        from_id: &str,
        target_id: &str,
        now: Instant,
        cmds: &mut Vec<String>,
    ) -> bool {
        if from_id == target_id || self.pending_move_target.is_some() {
            return false;
        }
        if let Some((cmd, hop_room)) = self.first_bfs_step(from_id, target_id) {
            self.pending_move_target = Some((hop_room, now));
            self.next_move_allowed_at = Some(now + MOVE_STEP_COOLDOWN);
            cmds.push(cmd);
            true
        } else {
            false
        }
    }

    fn advance_pending_interaction_move(&mut self, now: Instant) {
        let Some(target_room) = self
            .pending_interaction
            .as_ref()
            .map(|pi| pi.target_room.clone())
        else {
            return;
        };
        if self.server_location_key == target_room || self.pending_move_target.is_some() {
            return;
        }
        let from = self.server_location_key.clone();
        let mut step_cmds = Vec::new();
        if self.try_send_single_move_hop(&from, &target_room, now, &mut step_cmds) {
            self.pending_orders.extend(step_cmds);
        } else {
            self.pending_interaction = None;
        }
    }

    fn resume_pending_interaction_if_arrived(&mut self, now: Instant) {
        let arrived = self.pending_interaction.as_ref().is_some_and(|pi| {
            self.pending_move_target.is_none() && self.server_location_key == pi.target_room
        });
        if !arrived {
            return;
        }
        let Some(pi) = self.pending_interaction.take() else {
            return;
        };
        let mut out = WalkKeyOutcome::default();
        self.execute_interact_action(pi.action, &pi.manifest_panels, now, &mut out);
        self.pending_orders.extend(out.commands);
        self.pending_followup_queries.extend(out.queries);
    }

    fn step_avatar(&mut self, fwd_m: f64, strafe_m: f64, now: Instant, cmds: &mut Vec<String>) {
        let old_x = self.you_x;
        let old_y = self.you_y;
        let old_floor = self.you_floor;
        let old_loc = self.active_location_key.clone();

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

        // Clear stair/teleport re-arm latch once the avatar has walked clear of the return portal.
        if let Some((rx, ry, rfloor)) = self.portal_rearm {
            let d = ((self.you_x - rx).powi(2) + (self.you_y - ry).powi(2)).sqrt();
            if rfloor != self.you_floor || d >= STAIR_LANDING_CLEARANCE_M {
                self.portal_rearm = None;
            }
        }

        // Check stairway & non-adjacent doorway portals first (`SSH-C-02`: guarded against ping-pong!)
        let active_portals = if self.aboard_ship {
            &self.ship_portals
        } else {
            &self.portals
        };
        let mut stepped_portal: Option<(i64, String, f64, f64)> = None;
        for p in active_portals {
            if p.from_room != self.active_location_key {
                continue;
            }
            let is_trigger_portal = p.stairs_to.is_some() || p.teleport;
            if !is_trigger_portal {
                continue;
            }
            if let Some((rx, ry, rfloor)) = self.portal_rearm
                && rfloor == p.floor
                && ((p.x_m - rx).powi(2) + (p.y_m - ry).powi(2)).sqrt() < 1.0
            {
                continue;
            }
            let dist = ((p.x_m - self.you_x).powi(2) + (p.y_m - self.you_y).powi(2)).sqrt();
            if dist <= STAIR_TRIGGER_RADIUS_M {
                let dest_floor = p.stairs_to.unwrap_or(p.floor);
                stepped_portal = Some((dest_floor, p.to_room.clone(), p.x_m, p.y_m));
                break;
            }
        }
        if let Some((dest_floor, dest_room, from_px, from_py)) = stepped_portal {
            let prev_room = self.active_location_key.clone();
            self.you_floor = dest_floor;
            self.active_location_key = dest_room.clone();
            // Place avatar cleanly inside `dest_room`, strictly > STAIR_TRIGGER_RADIUS_M from
            // any return portal leaving `dest_room` so stepping onto a stair NEVER ping-pongs (`SSH-C-02`).
            let return_portal = self.active_portals().iter().find(|rp| {
                rp.from_room == dest_room
                    && rp.to_room == prev_room
                    && (rp.stairs_to.is_some() || rp.teleport)
            });
            let (ret_x, ret_y) = return_portal.map_or((from_px, from_py), |rp| (rp.x_m, rp.y_m));
            if let Some(rm) = self.find_room(&dest_room).cloned() {
                let (lx, ly) = safe_stair_landing(&rm, ret_x, ret_y);
                self.you_x = lx;
                self.you_y = ly;
            }
            self.portal_rearm = Some((ret_x, ret_y, dest_floor));
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

        if self.active_location_key != old_loc
            && self
                .pending_interaction
                .as_ref()
                .is_some_and(|pi| pi.target_room != self.active_location_key)
        {
            self.pending_interaction = None;
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
        if (self.you_x - old_x).abs() > 1e-6
            || (self.you_y - old_y).abs() > 1e-6
            || self.you_floor != old_floor
            || self.active_location_key != old_loc
        {
            self.bump_scene();
        }
    }

    fn active_portals(&self) -> &[PortalGeom] {
        if self.aboard_ship {
            &self.ship_portals
        } else {
            &self.portals
        }
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
        let portals = self.active_portals();
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
        self.portal_rearm = None;
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
        let portals = self.active_portals();
        let mut queue: VecDeque<(String, Option<(String, String)>)> = VecDeque::new();
        let mut visited: HashSet<String> = HashSet::new();
        queue.push_back((from_id.to_string(), None));
        visited.insert(from_id.to_string());

        while let Some((cur, first_hop)) = queue.pop_front() {
            for p in portals {
                if p.from_room != cur
                    || visited.contains(&p.to_room)
                    || !p.command.starts_with("goto ")
                {
                    continue;
                }
                let hop = first_hop
                    .clone()
                    .unwrap_or_else(|| (p.command.clone(), p.to_room.clone()));
                if p.to_room == to_id {
                    return Some(hop);
                }
                visited.insert(p.to_room.clone());
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
        } else if lower.contains("airlock") || lower.contains("lock") {
            Some("airlock".to_string())
        } else if lower.contains("passage") || lower.contains("spine") {
            Some("passage".to_string())
        } else if lower.contains("crew")
            || lower.contains("bunk")
            || lower.contains("galley")
            || lower.contains("quarter")
            || lower.contains("mess")
        {
            Some("crew".to_string())
        } else if lower.contains("hold") || lower.contains("cargo") {
            Some("hold".to_string())
        } else if lower.contains("eng") || lower.contains("drive") || lower.contains("crawlway") {
            Some("engineering".to_string())
        } else {
            None
        }
    }

    fn rebuild_actors_and_interactables(&mut self, view: &RoomView, self_name: Option<&str>) {
        self.actors.clear();
        self.interactables.clear();

        let (rcx, rcy, rfloor, rmin_x, rmax_x, rmin_y, rmax_y) = self
            .find_room(&self.active_location_key)
            .map_or((0.0, 0.0, 0, -6.0, 6.0, -6.0, 6.0), |r| {
                let (cx, cy) = r.center();
                (cx, cy, r.floor, r.min_x, r.max_x, r.min_y, r.max_y)
            });

        let clamp_in_room = |x: f64, y: f64| -> (f64, f64) {
            let x0 = (rmin_x + 1.2).min(rcx);
            let x1 = (rmax_x - 1.2).max(rcx);
            let y0 = (rmin_y + 1.2).min(rcy);
            let y1 = (rmax_y - 1.2).max(rcy);
            (x.clamp(x0, x1), y.clamp(y0, y1))
        };

        // Live server schema (`crates/mush-npc/src/module.rs:4500-4592`):
        // `RoomView.ambient["occupants"] = { "npcs": ["#12"], "dormant": ["#13"], "players": ["#14"], "things": ["#15"] }`.
        let mut idx = 0usize;
        if let Some(occ) = view.ambient.get("occupants").and_then(Value::as_object) {
            let id_set = |key: &str| -> HashSet<String> {
                occ.get(key)
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(Value::as_str)
                            .map(|s| sanitize(s).to_ascii_lowercase())
                            .collect()
                    })
                    .unwrap_or_default()
            };
            let npc_ids = id_set("npcs");
            let dormant_ids = id_set("dormant");
            let player_ids = id_set("players");
            let thing_ids = id_set("things");

            for item in &view.contents {
                let id_str = sanitize(&item.id.to_string());
                let id_low = id_str.to_ascii_lowercase();
                let name = sanitize(&item.name);
                if name.is_empty() || dormant_ids.contains(&id_low) || thing_ids.contains(&id_low) {
                    continue;
                }
                if npc_ids.contains(&id_low) {
                    let angle = (idx as f64) * 1.25 + 0.5;
                    let (ax, ay) = clamp_in_room(rcx + angle.cos() * 2.6, rcy + angle.sin() * 2.2);
                    let label = self.format_npc_interact_label(&id_str, &name, true);
                    self.actors.push(ActorEntry {
                        id: id_str.clone(),
                        name: name.clone(),
                        npc: true,
                        x_m: ax,
                        y_m: ay,
                        floor: rfloor,
                    });
                    self.interactables.push(InteractableTarget {
                        label,
                        x_m: ax,
                        y_m: ay,
                        floor: rfloor,
                        action: InteractAction::OpenNpc { id: id_str, name },
                    });
                    idx += 1;
                } else if player_ids.contains(&id_low)
                    && !self_name.is_some_and(|me| me.eq_ignore_ascii_case(&name))
                {
                    let angle = (idx as f64) * 1.25 + 0.9;
                    let (ax, ay) = clamp_in_room(rcx + angle.cos() * 2.4, rcy + angle.sin() * 2.0);
                    self.actors.push(ActorEntry {
                        id: id_str.clone(),
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
                        action: InteractAction::OpenNpc { id: id_str, name },
                    });
                    idx += 1;
                }
            }
        } else {
            // Legacy fallback when `ambient["occupants"]` is absent (e.g. older fixtures)
            let mut dormant = HashSet::new();
            if let Some(d_arr) = view.ambient.get("dormant_npcs").and_then(Value::as_array) {
                for d in d_arr {
                    if let Some(s) = d.as_str() {
                        dormant.insert(sanitize(s).to_ascii_lowercase());
                    }
                }
            }
            let has_explicit =
                view.ambient.contains_key("npcs") || view.ambient.contains_key("players");
            if let Some(npcs) = view.ambient.get("npcs").and_then(Value::as_array) {
                for n in npcs.iter().take(12) {
                    let id = n
                        .get("id")
                        .and_then(Value::as_str)
                        .map(sanitize)
                        .unwrap_or_default();
                    let name = n
                        .get("name")
                        .and_then(Value::as_str)
                        .map(sanitize)
                        .unwrap_or_default();
                    if name.is_empty()
                        || dormant.contains(&id.to_ascii_lowercase())
                        || dormant.contains(&name.to_ascii_lowercase())
                    {
                        continue;
                    }
                    let angle = (idx as f64) * 1.25 + 0.5;
                    let (ax, ay) = clamp_in_room(rcx + angle.cos() * 2.6, rcy + angle.sin() * 2.2);
                    let label = self.format_npc_interact_label(&id, &name, true);
                    self.actors.push(ActorEntry {
                        id: id.clone(),
                        name: name.clone(),
                        npc: true,
                        x_m: ax,
                        y_m: ay,
                        floor: rfloor,
                    });
                    self.interactables.push(InteractableTarget {
                        label,
                        x_m: ax,
                        y_m: ay,
                        floor: rfloor,
                        action: InteractAction::OpenNpc { id, name },
                    });
                    idx += 1;
                }
            }
            if let Some(players) = view.ambient.get("players").and_then(Value::as_array) {
                for p in players.iter().take(12) {
                    let id = p
                        .get("id")
                        .and_then(Value::as_str)
                        .map(sanitize)
                        .unwrap_or_default();
                    let name = p
                        .get("name")
                        .and_then(Value::as_str)
                        .map(sanitize)
                        .unwrap_or_default();
                    if name.is_empty() || self_name.is_some_and(|me| me.eq_ignore_ascii_case(&name))
                    {
                        continue;
                    }
                    let angle = (idx as f64) * 1.25 + 0.9;
                    let (ax, ay) = clamp_in_room(rcx + angle.cos() * 2.4, rcy + angle.sin() * 2.0);
                    self.actors.push(ActorEntry {
                        id: id.clone(),
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
                        action: InteractAction::OpenNpc { id, name },
                    });
                    idx += 1;
                }
            }
            if !has_explicit {
                for item in view.contents.iter().take(8) {
                    let name = sanitize(&item.name);
                    if name.is_empty()
                        || self_name.is_some_and(|me| me.eq_ignore_ascii_case(&name))
                        || dormant.contains(&name.to_ascii_lowercase())
                    {
                        continue;
                    }
                    let id = sanitize(&item.id.to_string());
                    let angle = (idx as f64) * 1.3 + 0.6;
                    let (ax, ay) = clamp_in_room(rcx + angle.cos() * 2.5, rcy + angle.sin() * 2.0);
                    let label = self.format_npc_interact_label(&id, &name, true);
                    self.actors.push(ActorEntry {
                        id: id.clone(),
                        name: name.clone(),
                        npc: true,
                        x_m: ax,
                        y_m: ay,
                        floor: rfloor,
                    });
                    self.interactables.push(InteractableTarget {
                        label,
                        x_m: ax,
                        y_m: ay,
                        floor: rfloor,
                        action: InteractAction::OpenNpc { id, name },
                    });
                    idx += 1;
                }
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
                            y_m: rmax_y - 1.0,
                            floor: rfloor,
                            action: InteractAction::Command("+ship/leave"),
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
                    if self.ship_rooms.iter().any(|r| r.entry_id == "bay") {
                        let bay_quest = self
                            .quest_mining_status
                            .as_ref()
                            .is_some_and(|m| !m.mine_done)
                            || self.quest_ship_target.as_deref() == Some("bay");
                        self.interactables.push(InteractableTarget {
                            label: if bay_quest {
                                "[Q] Enter Vehicle Bay (+tv/bay in)".to_string()
                            } else {
                                "Enter Vehicle Bay (+tv/bay in)".to_string()
                            },
                            x_m: rcx,
                            y_m: rmin_y + 1.2,
                            floor: rfloor,
                            action: InteractAction::Command("+tv/bay in"),
                        });
                    }
                }
                "bay" => {
                    if let Some(qm) = &self.quest_mining_status {
                        let (lbl, cmd) = if qm.mine_done {
                            ("[Q] Recover Rig (+tv/1 recover go)", "+tv/1 recover go")
                        } else {
                            (
                                "[Q] Launch Surface Rig (+tv/1 launch go)",
                                "+tv/1 launch go",
                            )
                        };
                        self.interactables.insert(
                            0,
                            InteractableTarget {
                                label: lbl.to_string(),
                                x_m: rcx,
                                y_m: rmax_y - 1.0,
                                floor: rfloor,
                                action: InteractAction::Command(cmd),
                            },
                        );
                    }
                    self.interactables.push(InteractableTarget {
                        label: "Rig Status & Launch Check (+tv)".to_string(),
                        x_m: rcx,
                        y_m: rmax_y - 1.4,
                        floor: rfloor,
                        action: InteractAction::Command("+tv"),
                    });
                    self.interactables.push(InteractableTarget {
                        label: "Return to Cargo Hold (+tv/bay out)".to_string(),
                        x_m: rcx,
                        y_m: rmin_y + 1.2,
                        floor: rfloor,
                        action: InteractAction::Command("+tv/bay out"),
                    });
                }
                _ => {}
            }
        } else {
            let key = self.active_location_key.as_str();
            if key.contains("coop") || key.contains("registry") {
                self.interactables.insert(
                    0,
                    InteractableTarget {
                        label: "Co-op Charter & Registry Desk".to_string(),
                        x_m: rcx,
                        y_m: rmax_y - 1.8,
                        floor: rfloor,
                        action: InteractAction::OpenDrawer(PortDrawerKind::Coop),
                    },
                );
            } else if key.contains("exchange")
                || key.contains("chandlery")
                || key.contains("commissary")
            {
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
                self.interactables.push(InteractableTarget {
                    label: "Port Works, Shipyard & 24-Wk Trends".to_string(),
                    x_m: (rcx + 3.0).min(rmax_x - 1.5),
                    y_m: rmax_y - 1.8,
                    floor: rfloor,
                    action: InteractAction::OpenDrawer(PortDrawerKind::Industry),
                });
            } else if key.contains("slack")
                || key.contains("shift_board")
                || key.contains("arrivals")
                || key.contains("customs")
            {
                self.interactables.insert(
                    0,
                    InteractableTarget {
                        label: "Open Jobs, Haulage & Crew Board".to_string(),
                        x_m: rcx,
                        y_m: rmax_y - 1.8,
                        floor: rfloor,
                        action: InteractAction::OpenDrawer(PortDrawerKind::Jobs),
                    },
                );
            } else if key.contains("works")
                || key.contains("yard")
                || key.contains("service")
                || key.contains("clearing")
                || key.contains("tycho")
                || key.contains("receiving")
                || key.contains("maintenance")
            {
                self.interactables.insert(
                    0,
                    InteractableTarget {
                        label: "Port Works, Shipyard & Core Counter".to_string(),
                        x_m: rcx,
                        y_m: rmax_y - 1.8,
                        floor: rfloor,
                        action: InteractAction::OpenDrawer(PortDrawerKind::Industry),
                    },
                );
            } else if key.contains("concourse") {
                self.interactables.insert(
                    0,
                    InteractableTarget {
                        label: "Port Works, Jobs & Directory Kiosk".to_string(),
                        x_m: rcx,
                        y_m: rcy,
                        floor: rfloor,
                        action: InteractAction::OpenDrawer(PortDrawerKind::Industry),
                    },
                );
                self.interactables.push(InteractableTarget {
                    label: "Open Jobs, Haulage & Crew Board".to_string(),
                    x_m: (rcx - 3.0).max(rmin_x + 1.5),
                    y_m: rcy,
                    floor: rfloor,
                    action: InteractAction::OpenDrawer(PortDrawerKind::Jobs),
                });
            } else if key.contains("dockmaster") || key.contains("harbourmaster") {
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
                let hatch_y = if self
                    .find_room(key)
                    .is_some_and(|r| self.room_glass_is_south(r))
                {
                    rmin_y + 1.2
                } else {
                    rmax_y - 1.2
                };
                self.interactables.insert(
                    0,
                    InteractableTarget {
                        label: "Board Cradled Hull (+ship/board)".to_string(),
                        x_m: rcx,
                        y_m: hatch_y,
                        floor: rfloor,
                        action: InteractAction::Command("+ship/board"),
                    },
                );
            } else if self.pack_id.contains("ooc")
                || key.contains("simulator")
                || key.contains("sim_red")
                || key.contains("sim_blue")
            {
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

        self.sort_interactables_quest_first(false);
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

    /// Decode live `hull_stowage_json` (`crates/tacsim/src/port/ship.rs:1763-1773`) with legacy fallbacks.
    fn decode_stowage_obj(&mut self, st: &serde_json::Map<String, Value>) {
        self.stowage_loaded = true;
        if let Some(v) = st.get("cargo_kg").and_then(Value::as_i64) {
            self.stowage.cargo_kg = v;
        }
        if let Some(v) = st
            .get("cargo_limit_kg")
            .or_else(|| st.get("max_cargo_kg"))
            .and_then(Value::as_i64)
            .filter(|&n| n > 0)
        {
            self.stowage.max_cargo_kg = v;
        }
        if let Some(v) = st.get("cargo_l").and_then(Value::as_f64) {
            self.stowage.cargo_m3 = v / 1000.0;
        } else if let Some(v) = st.get("cargo_m3").and_then(Value::as_f64) {
            self.stowage.cargo_m3 = v;
        }
        if let Some(v) = st
            .get("cargo_limit_l")
            .and_then(Value::as_f64)
            .filter(|&n| n > 0.0)
        {
            self.stowage.max_cargo_m3 = v / 1000.0;
        } else if let Some(v) = st
            .get("max_cargo_m3")
            .and_then(Value::as_f64)
            .filter(|&n| n > 0.0)
        {
            self.stowage.max_cargo_m3 = v;
        }
        if let Some(v) = st
            .get("tank_kg")
            .or_else(|| st.get("reaction_kg"))
            .or_else(|| st.get("fuel_kg"))
            .and_then(Value::as_i64)
        {
            self.stowage.reaction_kg = v;
        }
        if let Some(v) = st
            .get("tank_limit_kg")
            .or_else(|| st.get("max_reaction_kg"))
            .or_else(|| st.get("max_fuel_kg"))
            .and_then(Value::as_i64)
            .filter(|&n| n > 0)
        {
            self.stowage.max_reaction_kg = v;
        }
        if let Some(v) = st.get("stores_days").and_then(Value::as_i64).or_else(|| {
            st.get("stores_person_days")
                .and_then(Value::as_f64)
                .map(|f| f.round() as i64)
        }) {
            self.stowage.stores_days = v;
        }
        if let Some(v) = st
            .get("stores_limit_days")
            .and_then(Value::as_i64)
            .filter(|&n| n > 0)
        {
            self.stowage.max_stores_days = v;
        }
        if let Some(items) = st.get("items").and_then(Value::as_array) {
            self.decode_cargo_items_arr(items);
        }
    }

    fn decode_cargo_items_arr(&mut self, arr: &[Value]) {
        self.cargo_lots.clear();
        for item in arr.iter().take(64) {
            let gid = item
                .get("good")
                .or_else(|| item.get("good_id"))
                .and_then(Value::as_str)
                .map(sanitize)
                .unwrap_or_default();
            let qty = item
                .get("qty")
                .or_else(|| item.get("quantity"))
                .and_then(Value::as_i64)
                .unwrap_or(0);
            if gid.is_empty() || qty <= 0 {
                continue;
            }
            let (gname, _, _) = self.lookup_goods_meta(&gid);
            self.cargo_lots.push(CargoLotRow {
                good_id: gid,
                good_name: gname,
                quantity: qty,
            });
        }
    }

    fn max_fit_for_ask(&self, ask: &AskRow) -> i64 {
        if !self.stowage_loaded
            || self.stowage.max_cargo_kg <= 0
            || self.stowage.max_cargo_m3 <= 0.0
        {
            return 0;
        }
        let (unit_mass_kg, unit_vol_dm3) = self
            .lookup_goods_meta_opt(&ask.good_id)
            .map_or((ask.unit_mass_kg, ask.unit_vol_dm3), |(_, m, v)| (m, v));
        let by_avail = ask.quantity.max(0);
        let by_cash = if ask.unit_price_minor > 0 {
            (self.credits_minor.max(0)) / ask.unit_price_minor
        } else {
            by_avail
        };
        let free_kg = (self.stowage.max_cargo_kg - self.stowage.cargo_kg).max(0);
        let by_mass = if unit_mass_kg > 0 {
            free_kg / unit_mass_kg
        } else {
            by_avail
        };
        let free_dm3 =
            ((self.stowage.max_cargo_m3 - self.stowage.cargo_m3).max(0.0) * 1000.0).floor() as i64;
        let by_vol = if unit_vol_dm3 > 0 {
            free_dm3 / unit_vol_dm3
        } else {
            by_avail
        };
        by_avail.min(by_cash).min(by_mass).min(by_vol).max(0)
    }

    fn push_drawer_log(&mut self, line: String) {
        let clean = sanitize(&line);
        if clean.is_empty() {
            return;
        }
        self.drawer_log.push_back(clean);
        while self.drawer_log.len() > DRAWER_LOG_CAP {
            self.drawer_log.pop_front();
        }
        self.drawer_log_scroll = 0;
    }

    fn stowage_gauge_lines(&self, utf8: bool) -> Vec<Line<'static>> {
        let st = &self.stowage;
        let wallet = if self.balance_text.is_empty() {
            format_credits(self.credits_minor)
        } else {
            self.balance_text.clone()
        };
        if !self.stowage_loaded {
            return vec![
                Line::from(vec![
                    Span::styled("Wallet: ", Style::default().fg(Color::Gray)),
                    Span::styled(
                        wallet,
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                gauge_row("Hold Mass", 0.0, 1.0, "--".to_string(), utf8),
                gauge_row("Packed Vol", 0.0, 1.0, "--".to_string(), utf8),
                gauge_row("Reaction", 0.0, 1.0, "--".to_string(), utf8),
                gauge_row("Stores", 0.0, 1.0, "--".to_string(), utf8),
            ];
        }
        let readiness = self.quest_stowage_readiness();
        let hold_label = if readiness.as_ref().is_some_and(|r| r.gauge_kind == "cargo") {
            "*Hold Mass"
        } else {
            "Hold Mass"
        };
        let react_label = if readiness
            .as_ref()
            .is_some_and(|r| r.gauge_kind == "propellant")
        {
            "*Reaction"
        } else {
            "Reaction"
        };
        let stores_label = if readiness
            .as_ref()
            .is_some_and(|r| r.gauge_kind == "provisions")
        {
            "*Stores"
        } else {
            "Stores"
        };
        let mut rows = vec![
            Line::from(vec![
                Span::styled("Wallet: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    wallet,
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            gauge_row(
                hold_label,
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
                react_label,
                st.reaction_kg as f64,
                st.max_reaction_kg.max(1) as f64,
                format!("{:.1} t", st.reaction_kg as f64 / 1000.0),
                utf8,
            ),
            gauge_row(
                stores_label,
                st.stores_days as f64,
                st.max_stores_days.max(1) as f64,
                format!("{}/{} crew-days", st.stores_days, st.max_stores_days),
                utf8,
            ),
        ];
        if let Some(r) = readiness {
            let (tag, col) = if r.ready {
                ("[READY]", Color::Green)
            } else {
                ("[QUEST]", Color::Yellow)
            };
            rows.push(Line::from(Span::styled(
                format!(
                    "{tag} {}: {}/{} {}",
                    r.label, r.current_qty, r.target_qty, r.unit
                ),
                Style::default().fg(col).add_modifier(Modifier::BOLD),
            )));
        }
        rows
    }

    fn draw_drawer_box(&self, frame: &mut Frame<'_>, area: Rect, utf8: bool) {
        let Some(active) = self.drawer else {
            return;
        };
        let all_tabs = [
            PortDrawerKind::Trade,
            PortDrawerKind::Jobs,
            PortDrawerKind::Industry,
            PortDrawerKind::Sailing,
            PortDrawerKind::Coop,
            PortDrawerKind::Sim,
            PortDrawerKind::Npc,
        ];
        // Compact tab bar fits cleanly inside 69-col inner width at the 120-col floor (`SSH-M-04`).
        let mut title_spans = vec![Span::raw(" Port: ")];
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
                    format!("{} ", t.label()),
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

        let inner_h = usize::from(inner.height);
        let inner_w = usize::from(inner.width);

        // Reserve dedicated bottom viewport for `drawer_log` so 24-week `+market/history` and
        // multi-line output are ALWAYS visible and scrollable via PgUp/PgDn (`SSH-H-05`).
        let log_total = self.drawer_log.len();
        let (log_rows, log_hdr_rows) = if log_total == 0 {
            (0usize, 0usize)
        } else if inner_h >= 14 {
            let want = if self.drawer_log_scroll > 0 {
                (inner_h / 2).clamp(4, 12)
            } else {
                (inner_h / 3).clamp(3, 8)
            };
            (want.min(log_total), 1usize)
        } else if inner_h >= 7 {
            (2usize.min(log_total), 1usize)
        } else {
            (1usize.min(log_total), 0usize)
        };
        let main_budget = inner_h.saturating_sub(log_rows + log_hdr_rows).max(1);

        let mut lines: Vec<Line<'static>> = Vec::new();
        let step_suffix = self.quest_step_badge.as_str();
        match active {
            PortDrawerKind::Trade => {
                let gauge_count = if main_budget >= 7 { 3 } else { 1 };
                for g in self.stowage_gauge_lines(utf8).into_iter().take(gauge_count) {
                    lines.push(g);
                }
                let preset = self.active_trade_qty_setting();
                let qty_word = if preset == 0 {
                    "Max".to_string()
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
                let more_tag = if self.trade_next_after.is_some() {
                    " >:more"
                } else {
                    ""
                };
                let count_tag = if self.trade_subtab == 0 {
                    format!(
                        "{}/{}",
                        self.cursor_row.saturating_add(1).min(self.asks.len()),
                        self.asks.len()
                    )
                } else {
                    format!(
                        "{}/{}",
                        self.cursor_row.saturating_add(1).min(self.bids.len()),
                        self.bids.len()
                    )
                };
                lines.push(Line::from(vec![
                    Span::styled("<", Style::default().fg(Color::DarkGray)),
                    Span::styled("Buy(b)", buy_style),
                    Span::raw("|"),
                    Span::styled("Sell(s)", sell_style),
                    Span::styled("> ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        format!("Qty[/]:{qty_word} [{count_tag}] r:ref{more_tag}"),
                        Style::default().fg(Color::Cyan),
                    ),
                ]));

                let effective_need = self.effective_quest_trade_need();
                if let Some(ref need) = effective_need {
                    let side_tag = if need.wants_buy && need.wants_sell {
                        "BUY/SELL"
                    } else if need.wants_sell {
                        "SELL"
                    } else {
                        "BUY"
                    };
                    let readiness = self.stowage_readiness_for(need);
                    let hold_suffix = if let Some(ref r) = readiness {
                        if r.ready {
                            format!(" | {}/{} [READY]", r.current_qty, r.target_qty)
                        } else {
                            format!(" | {}/{}", r.current_qty, r.target_qty)
                        }
                    } else {
                        String::new()
                    };
                    let extra_suffix = if self.quest_trade_needs.len() > 1 {
                        let extras: Vec<String> = self
                            .quest_trade_needs
                            .iter()
                            .filter(|n| n.commodity_slug != need.commodity_slug)
                            .map(|n| {
                                let ok = self.stowage_readiness_for(n).is_some_and(|r| r.ready);
                                format!(
                                    "{} {}{}",
                                    n.target_quantity,
                                    n.commodity_slug.replace('_', " "),
                                    if ok { " [OK]" } else { "" }
                                )
                            })
                            .collect();
                        if extras.is_empty() {
                            String::new()
                        } else {
                            format!(" | +{}", extras.join(", "))
                        }
                    } else {
                        String::new()
                    };
                    let qline = format!(
                        "[QUEST] {side_tag} {} x {} (g:jump+qty){hold_suffix}{extra_suffix}{step_suffix}",
                        need.target_quantity,
                        need.commodity_slug.replace('_', " ")
                    );
                    let banner_col = if readiness.as_ref().is_some_and(|r| r.ready) {
                        Color::Green
                    } else {
                        Color::Yellow
                    };
                    lines.push(Line::from(Span::styled(
                        truncate_str(&qline, inner_w),
                        Style::default().fg(banner_col).add_modifier(Modifier::BOLD),
                    )));
                    if let Some(haul_dest) = self.quest_trade_ready_to_sail_dest() {
                        let (haul_msg, haul_col) = if let Some(pf) =
                            self.quest_sailing_preflight_readiness()
                            && (!pf.propellant_ok || !pf.provisions_ok)
                        {
                            let topup = match (pf.propellant_ok, pf.provisions_ok) {
                                (false, false) => "Fuel & Stores",
                                (false, true) => "Fuel",
                                _ => "Stores",
                            };
                            (
                                format!(
                                    "[HAUL] -> {} (d:Sail | Top up {topup} first | do not sell here)",
                                    haul_dest.port_label
                                ),
                                Color::LightYellow,
                            )
                        } else {
                            (
                                format!(
                                    "[HAUL] -> {} (d:Sail | do not sell here)",
                                    haul_dest.port_label
                                ),
                                Color::Green,
                            )
                        };
                        lines.push(Line::from(Span::styled(
                            truncate_str(&haul_msg, inner_w),
                            Style::default().fg(haul_col).add_modifier(Modifier::BOLD),
                        )));
                    }
                } else if let Some(refit) = self.quest_refit_hold_readiness() {
                    let (banner, col) = if refit.ready {
                        (
                            format!(
                                "[QUEST] [REFIT HOLD CLEAR] Ready -> c:Co-op Desk (+coop/refit prospector){step_suffix}"
                            ),
                            Color::Green,
                        )
                    } else {
                        let cargo_t = format!("{:.1}t", (refit.cargo_kg as f64) / 1000.0);
                        let rem = match (refit.cargo_empty, refit.provisions_empty) {
                            (false, false) => {
                                format!("Cargo {cargo_t} & Stores {}d", refit.provisions_days)
                            }
                            (false, true) => format!("Cargo {cargo_t}"),
                            _ => format!("Stores {}d", refit.provisions_days),
                        };
                        (
                            format!(
                                "[QUEST] [REFIT PREP] Empty hold: {rem} (g:Bids | propellant OK){step_suffix}"
                            ),
                            Color::Yellow,
                        )
                    };
                    lines.push(Line::from(Span::styled(
                        truncate_str(&banner, inner_w),
                        Style::default().fg(col).add_modifier(Modifier::BOLD),
                    )));
                }

                let list_rows = main_budget.saturating_sub(lines.len()).max(1);
                if self.trade_subtab == 0 {
                    if self.asks.is_empty() {
                        lines.push(Line::from(Span::styled(
                            "No asks loaded (press r at a port exchange).",
                            Style::default().fg(Color::DarkGray),
                        )));
                    } else {
                        for idx in visible_window(self.asks.len(), self.cursor_row, list_rows) {
                            if let Some(ask) = self.asks.get(idx) {
                                let sel = idx == self.cursor_row;
                                let q_match = effective_need.as_ref().is_some_and(|n| {
                                    n.matches_listing(&ask.good_id, &ask.good_name, false)
                                });
                                let fit_text = if self.stowage_loaded {
                                    self.max_fit_for_ask(ask).to_string()
                                } else {
                                    "--".to_string()
                                };
                                let mark = if sel { "> " } else { "  " };
                                let qtag = if q_match { "[Q] " } else { "" };
                                let st = if sel && q_match {
                                    Style::default()
                                        .fg(Color::Yellow)
                                        .add_modifier(Modifier::BOLD)
                                } else if sel {
                                    Style::default()
                                        .fg(Color::White)
                                        .add_modifier(Modifier::BOLD)
                                } else if q_match {
                                    Style::default().fg(Color::Yellow)
                                } else {
                                    Style::default().fg(Color::Gray)
                                };
                                let row = format!(
                                    "{mark}{qtag}#{:<4} {:<16} {:>5} {:>10} fit:{fit_text}",
                                    ask.listing_id,
                                    truncate_str(&ask.good_name, 16),
                                    ask.quantity,
                                    format_credits(ask.unit_price_minor)
                                );
                                lines.push(Line::from(Span::styled(
                                    truncate_str(&row, inner_w),
                                    st,
                                )));
                            }
                        }
                    }
                } else if self.bids.is_empty() {
                    lines.push(Line::from(Span::styled(
                        "No bids loaded (press r at a port exchange).",
                        Style::default().fg(Color::DarkGray),
                    )));
                } else {
                    for idx in visible_window(self.bids.len(), self.cursor_row, list_rows) {
                        if let Some(bid) = self.bids.get(idx) {
                            let sel = idx == self.cursor_row;
                            let q_match = effective_need.as_ref().is_some_and(|n| {
                                n.matches_listing(&bid.good_id, &bid.good_name, true)
                            });
                            let refit_clear = !q_match && self.is_refit_clear_bid(bid);
                            let highlighted = q_match || refit_clear;
                            let hold_text = if self.stowage_loaded {
                                self.cargo_lots
                                    .iter()
                                    .filter(|c| c.good_id == bid.good_id)
                                    .map(|c| c.quantity)
                                    .sum::<i64>()
                                    .to_string()
                            } else {
                                "--".to_string()
                            };
                            let mark = if sel { "> " } else { "  " };
                            let qtag = if refit_clear {
                                "[CLEAR] "
                            } else if q_match {
                                "[Q] "
                            } else {
                                ""
                            };
                            let st = if sel && highlighted {
                                Style::default()
                                    .fg(Color::Yellow)
                                    .add_modifier(Modifier::BOLD)
                            } else if sel {
                                Style::default()
                                    .fg(Color::White)
                                    .add_modifier(Modifier::BOLD)
                            } else if highlighted {
                                Style::default().fg(Color::Yellow)
                            } else {
                                Style::default().fg(Color::Gray)
                            };
                            let row = format!(
                                "{mark}{qtag}#{:<4} {:<16} {:>5} {:>10} hold:{hold_text}",
                                bid.listing_id,
                                truncate_str(&bid.good_name, 16),
                                bid.quantity_wanted,
                                format_credits(bid.unit_price_minor)
                            );
                            lines.push(Line::from(Span::styled(truncate_str(&row, inner_w), st)));
                        }
                    }
                }
            }
            PortDrawerKind::Jobs => {
                let quest_job_cmd: Option<&str> = if self.quest_mining_status.is_some() {
                    lines.push(Line::from(Span::styled(
                        truncate_str(
                            &format!(
                                "[QUEST] Surface Surveys (f: +market/jobs survey){step_suffix}"
                            ),
                            inner_w,
                        ),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    )));
                    Some("+market/jobs survey")
                } else if let Some(ref dest) = self.quest_dest_port {
                    lines.push(Line::from(Span::styled(
                        truncate_str(
                            &format!(
                                "[QUEST] Corridor Haul -> {} (f: +market/jobs haul){step_suffix}",
                                dest.port_label
                            ),
                            inner_w,
                        ),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    )));
                    Some("+market/jobs haul")
                } else {
                    None
                };
                lines.push(Line::from(Span::styled(
                    truncate_str("Jobs & Crew — Up/Dn Enter:run  o:+locker/open", inner_w),
                    Style::default().fg(Color::Cyan),
                )));
                let list_rows = main_budget.saturating_sub(lines.len()).max(1);
                let cur = self.cursor_row.min(DOC48_JOB_ACTIONS.len() - 1);
                for idx in visible_window(DOC48_JOB_ACTIONS.len(), cur, list_rows) {
                    if let Some(&(label, cmd)) = DOC48_JOB_ACTIONS.get(idx) {
                        let sel = idx == cur;
                        let is_quest_row = quest_job_cmd == Some(cmd);
                        let mark = if sel { "> " } else { "  " };
                        let tag = if is_quest_row { "[Q] " } else { "" };
                        let st = if sel {
                            Style::default()
                                .fg(Color::Green)
                                .add_modifier(Modifier::BOLD)
                        } else if is_quest_row {
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::Gray)
                        };
                        let disp = format!("{tag}{label}");
                        let row = format!("{mark}{disp:<26} ({cmd})");
                        lines.push(Line::from(Span::styled(truncate_str(&row, inner_w), st)));
                    }
                }
            }
            PortDrawerKind::Industry => {
                if let Some(ref act) = self.quest_coop_action
                    && act.command.starts_with("+coop/refit")
                {
                    lines.push(Line::from(Span::styled(
                        truncate_str(
                            &format!(
                                "[QUEST] Refit Dray-P Prospector (press p: +coop/refit prospector){step_suffix}"
                            ),
                            inner_w,
                        ),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    )));
                    if let Some(refit) = self.quest_refit_hold_readiness() {
                        let (msg, col) = if refit.ready {
                            (
                                format!(
                                    "[READY] Hold empty | Stores empty | Tank {}t OK",
                                    refit.propellant_tonnes
                                ),
                                Color::Green,
                            )
                        } else {
                            let cargo_txt = if refit.cargo_empty {
                                "OK".to_string()
                            } else {
                                format!("{}t", (refit.cargo_kg + 999) / 1000)
                            };
                            let stores_txt = if refit.provisions_empty {
                                "OK".to_string()
                            } else {
                                format!("{}d", refit.provisions_days)
                            };
                            (
                                format!(
                                    "[CLEAR HOLD] Cargo: {cargo_txt} | Stores: {stores_txt} (Tank {}t OK)",
                                    refit.propellant_tonnes
                                ),
                                Color::LightYellow,
                            )
                        };
                        lines.push(Line::from(Span::styled(
                            truncate_str(&msg, inner_w),
                            Style::default().fg(col).add_modifier(Modifier::BOLD),
                        )));
                    }
                }
                let quest_hist_need = self.effective_quest_trade_need();
                if let Some(ref need) = quest_hist_need {
                    let lbl = need.commodity_slug.replace('_', " ").to_ascii_uppercase();
                    lines.push(Line::from(Span::styled(
                        truncate_str(
                            &format!(
                                "[QUEST] 24w Trend: {lbl} (h: +market/history tacsim:{}){step_suffix}",
                                need.commodity_slug
                            ),
                            inner_w,
                        ),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    )));
                }
                lines.push(Line::from(Span::styled(
                    truncate_str(
                        "Works, Yard & 24wk — Up/Dn Enter:run  m:+market/mine",
                        inner_w,
                    ),
                    Style::default().fg(Color::Cyan),
                )));
                let list_rows = main_budget.saturating_sub(lines.len()).max(1);
                let cur = self.cursor_row.min(DOC48_INDUSTRY_ACTIONS.len() - 1);
                for idx in visible_window(DOC48_INDUSTRY_ACTIONS.len(), cur, list_rows) {
                    if let Some(&(label, cmd)) = DOC48_INDUSTRY_ACTIONS.get(idx) {
                        let sel = idx == cur;
                        let is_quest_row = quest_hist_need.as_ref().is_some_and(|need| {
                            cmd == format!("+market/history tacsim:{}", need.commodity_slug)
                        });
                        let mark = if sel { "> " } else { "  " };
                        let st = if sel {
                            Style::default()
                                .fg(Color::Green)
                                .add_modifier(Modifier::BOLD)
                        } else if is_quest_row {
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::Gray)
                        };
                        let disp = if is_quest_row {
                            format!("[Q] {label}")
                        } else {
                            label.to_string()
                        };
                        let row = format!("{mark}{disp:<26} ({cmd})");
                        lines.push(Line::from(Span::styled(truncate_str(&row, inner_w), st)));
                    }
                }
            }
            PortDrawerKind::Sailing => {
                let mode_lbl = if self.queue_depart {
                    "Depart (+ship/order)"
                } else {
                    "Dry-Run (+ship/plan)"
                };
                lines.push(Line::from(Span::styled(
                    truncate_str(&format!("Mode: {mode_lbl} (r toggle)  Enter:run"), inner_w),
                    Style::default().fg(Color::Cyan),
                )));
                lines.push(Line::from(Span::styled(
                    truncate_str("u: +ship/undock   d: +helm/dock (aboard)", inner_w),
                    Style::default().fg(Color::DarkGray),
                )));
                if let Some(ref hint) = self.quest_dest_port {
                    let qline =
                        format!("[QUEST] Dest: {} (g:select){step_suffix}", hint.port_label);
                    lines.push(Line::from(Span::styled(
                        truncate_str(&qline, inner_w),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    )));
                    if let Some(pf) = self.quest_sailing_preflight_readiness() {
                        let cargo_part = if pf.cargo_required {
                            let mark = if pf.cargo_ok { "OK" } else { "!" };
                            format!(
                                " | {} {}/{} {}",
                                pf.cargo_label, pf.cargo_qty, pf.cargo_target, mark
                            )
                        } else {
                            String::new()
                        };
                        let (msg, col) = if pf.ready {
                            (
                                format!(
                                    "[PRE-FLIGHT OK] Fuel {}t | Stores {}d{cargo_part}",
                                    pf.propellant_tonnes, pf.provisions_days
                                ),
                                Color::Green,
                            )
                        } else {
                            let fuel_txt = if pf.propellant_ok {
                                format!("{}t OK", pf.propellant_tonnes)
                            } else {
                                "0t!".to_string()
                            };
                            let stores_txt = if pf.provisions_ok {
                                format!("{}d OK", pf.provisions_days)
                            } else {
                                "0d!".to_string()
                            };
                            (
                                format!(
                                    "[CHECK] Fuel {fuel_txt} | Stores {stores_txt}{cargo_part} (b:Exchange)"
                                ),
                                Color::LightYellow,
                            )
                        };
                        lines.push(Line::from(Span::styled(
                            truncate_str(&msg, inner_w),
                            Style::default().fg(col).add_modifier(Modifier::BOLD),
                        )));
                    }
                } else if let Some(ref qm) = self.quest_mining_status
                    && !qm.mine_done
                {
                    let qline = format!(
                        "[QUEST] Surface Extraction: {} (g:Commons  c:+chart/ground){step_suffix}",
                        qm.target_ore_slug.replace('_', " ")
                    );
                    lines.push(Line::from(Span::styled(
                        truncate_str(&qline, inner_w),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    )));
                }
                let list_rows = main_budget.saturating_sub(lines.len()).max(1);
                let ordered = self.ordered_sailing_corridor_indices();
                let cur_idx = self.cursor_idx_clamped(CORRIDORS.len());
                let cur_pos = ordered.iter().position(|&ci| ci == cur_idx).unwrap_or(0);
                let quest_target_code = self.sailing_quest_target_code().map(str::to_string);
                for pos in visible_window(ordered.len(), cur_pos, list_rows) {
                    let idx = ordered[pos];
                    if let Some(&(code, name, desc)) = CORRIDORS.get(idx) {
                        let sel = idx == cur_idx;
                        let q_match = quest_target_code
                            .as_deref()
                            .is_some_and(|t| t.eq_ignore_ascii_case(code));
                        let is_here = self
                            .current_venue
                            .as_deref()
                            .is_some_and(|v| v.eq_ignore_ascii_case(code));
                        let mark = if sel { "> " } else { "  " };
                        let qtag = if q_match { "[Q] " } else { "" };
                        let here_tag = if is_here { " [HERE]" } else { "" };
                        let st = if sel && q_match {
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD)
                        } else if sel {
                            Style::default()
                                .fg(Color::Green)
                                .add_modifier(Modifier::BOLD)
                        } else if q_match {
                            Style::default().fg(Color::Yellow)
                        } else if is_here {
                            Style::default().fg(Color::Cyan)
                        } else {
                            Style::default().fg(Color::Gray)
                        };
                        let row = format!("{mark}{qtag}{name:<19} {desc}{here_tag}");
                        lines.push(Line::from(Span::styled(truncate_str(&row, inner_w), st)));
                    }
                }
            }
            PortDrawerKind::Coop => {
                if let Some(ref act) = self.quest_coop_action {
                    let banner = if act.command.starts_with("+coop/refit") {
                        format!(
                            "[QUEST] Refit Dray-P Prospector (press p: +coop/refit prospector){step_suffix}"
                        )
                    } else {
                        format!(
                            "[QUEST] Enroll Charter Hull (press Enter: +coop/enroll){step_suffix}"
                        )
                    };
                    lines.push(Line::from(Span::styled(
                        truncate_str(&banner, inner_w),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    )));
                    if let Some(refit) = self.quest_refit_hold_readiness() {
                        let (msg, col) = if refit.ready {
                            (
                                format!(
                                    "[READY] Hold empty | Stores empty | Tank {}t OK",
                                    refit.propellant_tonnes
                                ),
                                Color::Green,
                            )
                        } else {
                            let cargo_txt = if refit.cargo_empty {
                                "OK".to_string()
                            } else {
                                format!("{}t", (refit.cargo_kg + 999) / 1000)
                            };
                            let stores_txt = if refit.provisions_empty {
                                "OK".to_string()
                            } else {
                                format!("{}d", refit.provisions_days)
                            };
                            (
                                format!(
                                    "[CLEAR HOLD] Cargo: {cargo_txt} | Stores: {stores_txt} (Tank {}t OK)",
                                    refit.propellant_tonnes
                                ),
                                Color::LightYellow,
                            )
                        };
                        lines.push(Line::from(Span::styled(
                            truncate_str(&msg, inner_w),
                            Style::default().fg(col).add_modifier(Modifier::BOLD),
                        )));
                    }
                }
                lines.push(Line::from(Span::styled(
                    truncate_str(
                        "First Captains' Lay: zero cash upfront; lay settles from margin.",
                        inner_w,
                    ),
                    Style::default().fg(Color::Gray),
                )));
                let edit_tag = if self.editing_hull_name {
                    " [TYPING: Enter=ok Esc=cancel]"
                } else {
                    " (r:next  n:edit  Enter:enroll)"
                };
                let hull_line = format!("Hull: {}{edit_tag}", self.custom_hull_input);
                lines.push(Line::from(Span::styled(
                    truncate_str(&hull_line, inner_w),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(Span::styled(
                    truncate_str(
                        "Keys: Enter:+coop/enroll  p:+coop/refit prospector  t:+coop  s:+coop/statement",
                        inner_w,
                    ),
                    Style::default().fg(Color::Green),
                )));
            }
            PortDrawerKind::Sim => {
                let sim_hdr = if let Some(ref sim) = self.simulator {
                    let team = sim.my_team.as_deref().unwrap_or("none");
                    let unit = sim.my_unit.as_deref().unwrap_or("-");
                    format!(
                        "Sim [{} / {}] team:{team} unit:{unit} ({} slots)",
                        sim.arena,
                        sim.match_state,
                        sim.slots.len()
                    )
                } else {
                    "Simulator Pod Bay — select slot and press Enter:".to_string()
                };
                lines.push(Line::from(Span::styled(
                    truncate_str(&sim_hdr, inner_w),
                    Style::default().fg(Color::Cyan),
                )));
                if let Some(ref sim) = self.simulator
                    && !sim.slots.is_empty()
                {
                    let summary: Vec<String> = sim
                        .slots
                        .iter()
                        .take(6)
                        .map(|s| {
                            let tag = s
                                .team
                                .chars()
                                .next()
                                .map_or('?', |c| c.to_ascii_uppercase());
                            let pilot = if s.pilot_name.is_empty() {
                                "open"
                            } else {
                                s.pilot_name.as_str()
                            };
                            let seated = if s.seated { "*" } else { "" };
                            if s.unit.is_empty() {
                                format!("[{tag}{} {pilot}{seated}]", s.index)
                            } else {
                                format!("[{tag}{} {pilot}/{}{seated}]", s.index, s.unit)
                            }
                        })
                        .collect();
                    lines.push(Line::from(Span::styled(
                        truncate_str(&format!("Roster: {}", summary.join(" ")), inner_w),
                        Style::default().fg(Color::DarkGray),
                    )));
                }
                let list_rows = main_budget.saturating_sub(lines.len()).max(1);
                let cur = self.cursor_row.min(SIM_LOADOUTS.len() - 1);
                for idx in visible_window(SIM_LOADOUTS.len(), cur, list_rows) {
                    if let Some(&(label, cmd)) = SIM_LOADOUTS.get(idx) {
                        let sel = idx == cur;
                        let mark = if sel { "> " } else { "  " };
                        let st = if sel {
                            Style::default()
                                .fg(Color::Green)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::Gray)
                        };
                        let row = format!("{mark}{label:<28} ({cmd})");
                        lines.push(Line::from(Span::styled(truncate_str(&row, inner_w), st)));
                    }
                }
            }
            PortDrawerKind::Npc => {
                let speaker = if self.selected_npc_name.is_empty() {
                    "Room".to_string()
                } else {
                    format!("{} (n: next)", self.selected_npc_name)
                };
                lines.push(Line::from(Span::styled(
                    truncate_str(&format!("Speaking with: {speaker}"), inner_w),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )));
                if let Some(m) = self.selected_npc_quest_marker() {
                    let (prefix, col) = match m.marker.as_str() {
                        "available" => ("[!] Quest Available", Color::Yellow),
                        "turn_in" => ("[?] Turn In Quest", Color::Green),
                        _ => ("[?] Quest In Progress", Color::Yellow),
                    };
                    let title_part = if m.quest_title.is_empty() {
                        String::new()
                    } else {
                        format!(": {}", m.quest_title)
                    };
                    let extra = self.selected_npc_quest_count().saturating_sub(1);
                    let more_part = if extra > 0 {
                        format!(" (+{extra} more)")
                    } else {
                        String::new()
                    };
                    let npc_step = super::quests::guided_step_badge(&m.quest_key);
                    lines.push(Line::from(Span::styled(
                        truncate_str(
                            &format!("{prefix}{title_part}{more_part} (q: quest dialog){npc_step}"),
                            inner_w,
                        ),
                        Style::default().fg(col).add_modifier(Modifier::BOLD),
                    )));
                }
                let list_rows = main_budget.saturating_sub(lines.len()).max(1);
                let cur = self.cursor_row.min(NPC_TOPICS.len() - 1);
                for idx in visible_window(NPC_TOPICS.len(), cur, list_rows) {
                    if let Some(&(topic, q)) = NPC_TOPICS.get(idx) {
                        let sel = idx == cur;
                        let mark = if sel { "> " } else { "  " };
                        let st = if sel {
                            Style::default()
                                .fg(Color::Green)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::Gray)
                        };
                        let row = format!("{mark}{topic:<20} \"{q}\"");
                        lines.push(Line::from(Span::styled(truncate_str(&row, inner_w), st)));
                    }
                }
            }
        }

        // Clip main content to `main_budget` so log footer is NEVER pushed off-screen (`SSH-H-05`).
        lines.truncate(main_budget);

        if log_rows > 0 {
            let max_scroll = log_total.saturating_sub(log_rows);
            let scroll = self.drawer_log_scroll.min(max_scroll);
            let end = log_total.saturating_sub(scroll);
            let start = end.saturating_sub(log_rows);
            if log_hdr_rows > 0 {
                let hdr = format!(
                    "--- Output ({}-{} of {}, PgUp/PgDn) ---",
                    start + 1,
                    end,
                    log_total
                );
                lines.push(Line::from(Span::styled(
                    truncate_str(&hdr, inner_w),
                    Style::default().fg(Color::DarkGray),
                )));
            }
            for idx in start..end {
                if let Some(log_ln) = self.drawer_log.get(idx) {
                    lines.push(Line::from(Span::styled(
                        truncate_str(&format!("  {log_ln}"), inner_w),
                        Style::default().fg(Color::Yellow),
                    )));
                }
            }
        }

        frame.render_widget(Paragraph::new(lines), inner);
    }

    fn cursor_idx_clamped(&self, len: usize) -> usize {
        if len == 0 {
            0
        } else {
            self.corridor_idx.min(len - 1)
        }
    }
}

// ---- Geometry, Pack Parsers & Canonical Built-in Layouts --------------------

fn visible_window(total: usize, cursor: usize, max_rows: usize) -> Range<usize> {
    if total == 0 || max_rows == 0 {
        return 0..0;
    }
    let visible = max_rows.min(total);
    let cur = cursor.min(total - 1);
    let half = visible / 2;
    let start = cur.saturating_sub(half).min(total - visible);
    start..(start + visible)
}

/// Compute a landing position inside `room` strictly further than `STAIR_TRIGGER_RADIUS_M`
/// from `(ret_x, ret_y)` whenever the room geometry permits (`SSH-C-02`).
fn safe_stair_landing(room: &RoomGeom, ret_x: f64, ret_y: f64) -> (f64, f64) {
    let (cx, cy) = room.center();
    let dist_center = ((cx - ret_x).powi(2) + (cy - ret_y).powi(2)).sqrt();
    if dist_center >= STAIR_LANDING_CLEARANCE_M {
        return (cx, cy);
    }
    let pad_x = 0.8_f64.min((room.max_x - room.min_x) * 0.25);
    let pad_y = 0.8_f64.min((room.max_y - room.min_y) * 0.25);
    let x0 = room.min_x + pad_x;
    let x1 = room.max_x - pad_x;
    let y0 = room.min_y + pad_y;
    let y1 = room.max_y - pad_y;
    let candidates = [
        (cx, y1),
        (cx, y0),
        (x1, cy),
        (x0, cy),
        (x0, y0),
        (x1, y1),
        (x0, y1),
        (x1, y0),
    ];
    let mut best = (cx, cy);
    let mut best_d = dist_center;
    for (px, py) in candidates {
        let d = ((px - ret_x).powi(2) + (py - ret_y).powi(2)).sqrt();
        if d > best_d {
            best = (px, py);
            best_d = d;
        }
    }
    // Pull slightly toward center if the corner is very far, while keeping >= clearance
    let dx = best.0 - ret_x;
    let dy = best.1 - ret_y;
    let len = (dx * dx + dy * dy).sqrt();
    if len > STAIR_LANDING_CLEARANCE_M + 0.4 {
        let frac = (STAIR_LANDING_CLEARANCE_M + 0.35) / len;
        let nx = (ret_x + dx * frac).clamp(x0, x1);
        let ny = (ret_y + dy * frac).clamp(y0, y1);
        if ((nx - ret_x).powi(2) + (ny - ret_y).powi(2)).sqrt() >= STAIR_LANDING_CLEARANCE_M {
            return (nx, ny);
        }
    }
    best
}

fn normalize_unit_slug(raw: &str) -> String {
    let trimmed = raw.trim();
    let tail = trimmed
        .strip_prefix("tacsim.unit.")
        .or_else(|| trimmed.strip_prefix("unit."))
        .unwrap_or(trimmed);
    tail.to_ascii_lowercase()
}

fn classify_room_kind(entry_id: &str, is_dock: bool, atlas_kind: &str) -> &'static str {
    let low = entry_id.to_ascii_lowercase();
    if is_dock || low.contains("berth") {
        "berth"
    } else if low.contains("sim_red") || low.contains("sim_blue") || low.contains("pod") {
        "pod"
    } else if atlas_kind == "circulation" || low.contains("concourse") {
        "corridor"
    } else if low.contains("freight") || low.contains("hold") {
        "hold"
    } else if low.contains("bridge") {
        "bridge"
    } else if low.contains("eng") {
        "eng"
    } else if low.contains("registry")
        || low.contains("exchange")
        || low.contains("chandlery")
        || low.contains("commissary")
        || low.contains("dockmaster")
        || low.contains("harbourmaster")
        || low.contains("clearing")
        || low.contains("tycho")
        || low.contains("service")
        || low.contains("arrivals")
        || low.contains("customs")
        || low.contains("simulator_lobby")
    {
        "desk"
    } else {
        "room"
    }
}

/// Build a safe `goto <alias>` MUSH movement command from a semicolon-separated
/// exit name string, e.g. `"Pressure Lobby <N>;n;north;lobby"` -> `Some("goto n")`,
/// `"Inner Hatch <I>;inner;in"` -> `Some("goto inner")`. Rejects blank names or
/// names containing C0/DEL/C1 control or Unicode BiDi / line-separator controls
/// (`P2-CLIENT-2` / `EXT-P1-6`).
fn extract_exit_command(raw_name: &str) -> Option<String> {
    if raw_name.chars().any(crate::is_stripped_terminal_char) {
        return None;
    }
    let mut parts = raw_name.split(';').map(str::trim).filter(|s| !s.is_empty());
    let first = parts.next()?;
    let candidate = if let Some(alias) = parts.next() {
        alias.to_ascii_lowercase()
    } else if let (Some(lt), Some(gt)) = (first.find('<'), first.find('>'))
        && gt > lt + 1
    {
        first[lt + 1..gt].trim().to_ascii_lowercase()
    } else {
        first.to_ascii_lowercase()
    };
    super::app::exit_command(&candidate)
}

fn parse_footprint_rect(fp: &Value) -> Option<(f64, f64, f64, f64)> {
    const LIMIT: f64 = 10_000.0;
    let obj = fp.as_object()?;
    if let Some(rect) = obj.get("rect").and_then(Value::as_array)
        && rect.len() == 2
    {
        let p0 = rect.first()?.as_array()?;
        let p1 = rect.get(1)?.as_array()?;
        let x0 = p0.first()?.as_f64()?.clamp(-LIMIT, LIMIT);
        let y0 = p0.get(1)?.as_f64()?.clamp(-LIMIT, LIMIT);
        let x1 = p1.first()?.as_f64()?.clamp(-LIMIT, LIMIT);
        let y1 = p1.get(1)?.as_f64()?.clamp(-LIMIT, LIMIT);
        let (min_x, max_x) = (x0.min(x1), x0.max(x1));
        let (min_y, max_y) = (y0.min(y1), y0.max(y1));
        if max_x > min_x && max_y > min_y {
            return Some((min_x, min_y, max_x, max_y));
        }
    }
    if let Some(poly) = obj.get("polygon").and_then(Value::as_array)
        && !poly.is_empty()
    {
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for pt in poly {
            let Some(arr) = pt.as_array() else {
                continue;
            };
            if let (Some(x), Some(y)) = (
                arr.first().and_then(Value::as_f64),
                arr.get(1).and_then(Value::as_f64),
            ) {
                let x = x.clamp(-LIMIT, LIMIT);
                let y = y.clamp(-LIMIT, LIMIT);
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
        if min_x.is_finite() && max_x > min_x && max_y > min_y {
            return Some((min_x, min_y, max_x, max_y));
        }
    }
    if let Some(anchor) = obj.get("anchor").and_then(Value::as_object)
        && let Some(pt) = anchor.get("point").and_then(Value::as_array)
    {
        let x = pt.first()?.as_f64()?.clamp(-LIMIT, LIMIT);
        let y = pt.get(1)?.as_f64()?.clamp(-LIMIT, LIMIT);
        let r = anchor
            .get("radius_m")
            .and_then(Value::as_f64)
            .unwrap_or(5.0)
            .clamp(2.0, 200.0);
        return Some((x - r, y - r, x + r, y + r));
    }
    None
}

fn parse_surface_floor(surface: Option<&Value>) -> i64 {
    match surface {
        // `surface: "ground"` sits at floor 0 (matching `W3D-H6` / OOC Hall floor 0).
        Some(Value::String(_)) | None => 0,
        Some(Value::Object(map)) => map
            .get("floor")
            .and_then(Value::as_array)
            .and_then(|arr| arr.get(1))
            .and_then(Value::as_i64)
            .unwrap_or(0)
            .clamp(-100, 100),
        _ => 0,
    }
}

type PortalEndpoints = (Option<(f64, f64)>, Option<(f64, f64)>);

fn parse_portal_at_endpoints(portal: Option<&Value>) -> PortalEndpoints {
    let Some(at) = portal.and_then(|p| p.get("at")).and_then(Value::as_array) else {
        return (None, None);
    };
    // Form 1: `[[x0, y0, z0], [x1, y1, z1]]`
    if at.len() == 2
        && let (Some(a), Some(b)) = (
            at.first().and_then(Value::as_array),
            at.get(1).and_then(Value::as_array),
        )
        && let (Some(ax), Some(ay), Some(bx), Some(by)) = (
            a.first().and_then(Value::as_f64),
            a.get(1).and_then(Value::as_f64),
            b.first().and_then(Value::as_f64),
            b.get(1).and_then(Value::as_f64),
        )
    {
        return (Some((ax, ay)), Some((bx, by)));
    }
    // Form 2: `[x, y, z]`
    if at.len() >= 2
        && let (Some(x), Some(y)) = (
            at.first().and_then(Value::as_f64),
            at.get(1).and_then(Value::as_f64),
        )
    {
        return (Some((x, y)), Some((x, y)));
    }
    (None, None)
}

fn clamp_point_inside_room(room: &RoomGeom, x: f64, y: f64) -> (f64, f64) {
    let pad_x = 1.0_f64.min((room.max_x - room.min_x) * 0.25);
    let pad_y = 1.0_f64.min((room.max_y - room.min_y) * 0.25);
    (
        x.clamp(room.min_x + pad_x, room.max_x - pad_x),
        y.clamp(room.min_y + pad_y, room.max_y - pad_y),
    )
}

fn shared_edge_midpoint(a: &RoomGeom, b: &RoomGeom) -> Option<(f64, f64)> {
    const TOL: f64 = 1.5;
    // Touch along vertical edge (East/West)
    if (a.max_x - b.min_x).abs() <= TOL || (b.max_x - a.min_x).abs() <= TOL {
        let ov_min = a.min_y.max(b.min_y);
        let ov_max = a.max_y.min(b.max_y);
        if ov_max > ov_min + 0.5 {
            let x = if (a.max_x - b.min_x).abs() <= TOL {
                (a.max_x + b.min_x) * 0.5
            } else {
                (b.max_x + a.min_x) * 0.5
            };
            return Some((x, (ov_min + ov_max) * 0.5));
        }
    }
    // Touch along horizontal edge (North/South)
    if (a.max_y - b.min_y).abs() <= TOL || (b.max_y - a.min_y).abs() <= TOL {
        let ov_min = a.min_x.max(b.min_x);
        let ov_max = a.max_x.min(b.max_x);
        if ov_max > ov_min + 0.5 {
            let y = if (a.max_y - b.min_y).abs() <= TOL {
                (a.max_y + b.min_y) * 0.5
            } else {
                (b.max_y + a.min_y) * 0.5
            };
            return Some(((ov_min + ov_max) * 0.5, y));
        }
    }
    None
}

fn boundary_point_toward(from: &RoomGeom, to: &RoomGeom) -> (f64, f64) {
    let (fcx, fcy) = from.center();
    let (tcx, tcy) = to.center();
    let dx = tcx - fcx;
    let dy = tcy - fcy;
    if dx.abs() >= dy.abs() {
        let x = if dx >= 0.0 {
            from.max_x - 1.0
        } else {
            from.min_x + 1.0
        };
        (x.clamp(from.min_x + 0.8, from.max_x - 0.8), fcy)
    } else {
        let y = if dy >= 0.0 {
            from.max_y - 1.0
        } else {
            from.min_y + 1.0
        };
        (fcx, y.clamp(from.min_y + 0.8, from.max_y - 0.8))
    }
}

/// Parse a real projected `neighborhood.*` pack from `tacsim.content` into walkable rooms & portals (`SSH-C-02`).
fn parse_neighborhood_pack(pack_val: &Value) -> Option<(Vec<RoomGeom>, Vec<PortalGeom>)> {
    let entries = pack_val.get("entries")?.as_object()?;
    let mut rooms: Vec<RoomGeom> = Vec::new();
    let mut by_id: HashMap<String, usize> = HashMap::new();

    for (entry_id, row) in entries {
        if row.get("kind").and_then(Value::as_str) != Some("room") {
            continue;
        }
        let Some(def) = row.get("def").and_then(Value::as_object) else {
            continue;
        };
        let Some((min_x, min_y, max_x, max_y)) =
            def.get("footprint").and_then(parse_footprint_rect)
        else {
            continue;
        };
        let clean_id = sanitize(entry_id);
        let name = def
            .get("name")
            .and_then(Value::as_str)
            .map(sanitize)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| clean_id.clone());
        let floor = parse_surface_floor(def.get("surface"));
        let is_dock = def
            .get("markers")
            .and_then(Value::as_array)
            .is_some_and(|arr| arr.iter().any(|m| m.as_str() == Some("port.dock")));
        let atlas_kind = def.get("kind").and_then(Value::as_str).unwrap_or("indoor");
        let kind = classify_room_kind(&clean_id, is_dock, atlas_kind);

        let idx = rooms.len();
        by_id.insert(clean_id.clone(), idx);
        rooms.push(RoomGeom {
            entry_id: clean_id,
            name,
            floor,
            min_x,
            min_y,
            max_x,
            max_y,
            kind,
        });
    }

    if rooms.is_empty() {
        return None;
    }

    let mut portals: Vec<PortalGeom> = Vec::new();
    for row in entries.values() {
        if row.get("kind").and_then(Value::as_str) != Some("exit") {
            continue;
        }
        let Some(def) = row.get("def").and_then(Value::as_object) else {
            continue;
        };
        // Skip hidden / staff-locked exits in player walk geometry.
        if def.get("lock").and_then(Value::as_str) == Some("staff")
            || def.get("visibility").and_then(Value::as_str) == Some("hidden")
            || def
                .get("portal")
                .and_then(|p| p.get("visibility"))
                .and_then(Value::as_str)
                == Some("hidden")
        {
            continue;
        }
        let Some(from_id) = def.get("from").and_then(Value::as_str).map(sanitize) else {
            continue;
        };
        let Some(to_id) = def.get("to").and_then(Value::as_str).map(sanitize) else {
            continue;
        };
        let (Some(&idx_a), Some(&idx_b)) = (by_id.get(&from_id), by_id.get(&to_id)) else {
            continue;
        };
        let ra = &rooms[idx_a];
        let rb = &rooms[idx_b];

        let names = def.get("names").and_then(Value::as_array);
        let cmd_ab = match names.and_then(|arr| arr.first()).and_then(Value::as_str) {
            Some(raw) => extract_exit_command(raw),
            None => super::app::exit_command(&to_id),
        };
        let cmd_ba = names
            .and_then(|arr| arr.get(1))
            .and_then(Value::as_str)
            .and_then(extract_exit_command);

        let portal_def = def.get("portal");
        let (at_a, at_b) = parse_portal_at_endpoints(portal_def);

        if ra.floor != rb.floor {
            let (ax, ay) = match at_a {
                Some((x, y)) => clamp_point_inside_room(ra, x, y),
                None => ra.center(),
            };
            let (bx, by) = match at_b {
                Some((x, y)) => clamp_point_inside_room(rb, x, y),
                None => rb.center(),
            };
            if let Some(fwd) = cmd_ab {
                portals.push(PortalGeom {
                    from_room: from_id.clone(),
                    to_room: to_id.clone(),
                    command: fwd,
                    x_m: ax,
                    y_m: ay,
                    floor: ra.floor,
                    width_m: 2.8,
                    stairs_to: Some(rb.floor),
                    teleport: false,
                });
            }
            if let Some(rev) = cmd_ba {
                portals.push(PortalGeom {
                    from_room: to_id,
                    to_room: from_id,
                    command: rev,
                    x_m: bx,
                    y_m: by,
                    floor: rb.floor,
                    width_m: 2.8,
                    stairs_to: Some(ra.floor),
                    teleport: false,
                });
            }
        } else if let Some((mx, my)) = shared_edge_midpoint(ra, rb) {
            if let Some(fwd) = cmd_ab {
                portals.push(PortalGeom {
                    from_room: from_id.clone(),
                    to_room: to_id.clone(),
                    command: fwd,
                    x_m: mx,
                    y_m: my,
                    floor: ra.floor,
                    width_m: 2.8,
                    stairs_to: None,
                    teleport: false,
                });
            }
            if let Some(rev) = cmd_ba {
                portals.push(PortalGeom {
                    from_room: to_id,
                    to_room: from_id,
                    command: rev,
                    x_m: mx,
                    y_m: my,
                    floor: rb.floor,
                    width_m: 2.8,
                    stairs_to: None,
                    teleport: false,
                });
            }
        } else {
            // Non-adjacent horizontal portal (e.g. OOC `welcome_hall <-> simulator_lobby`)
            let (ax, ay) = boundary_point_toward(ra, rb);
            let (bx, by) = boundary_point_toward(rb, ra);
            if let Some(fwd) = cmd_ab {
                portals.push(PortalGeom {
                    from_room: from_id.clone(),
                    to_room: to_id.clone(),
                    command: fwd,
                    x_m: ax,
                    y_m: ay,
                    floor: ra.floor,
                    width_m: 2.8,
                    stairs_to: None,
                    teleport: true,
                });
            }
            if let Some(rev) = cmd_ba {
                portals.push(PortalGeom {
                    from_room: to_id,
                    to_room: from_id,
                    command: rev,
                    x_m: bx,
                    y_m: by,
                    floor: rb.floor,
                    width_m: 2.8,
                    stairs_to: None,
                    teleport: true,
                });
            }
        }
    }

    Some((rooms, portals))
}

fn has_unit_interior(pack_val: &Value) -> bool {
    pack_val
        .get("entries")
        .and_then(Value::as_object)
        .is_some_and(|entries| {
            entries.values().any(|row| {
                row.get("kind").and_then(Value::as_str) == Some("unit")
                    && row.get("def").and_then(|d| d.get("interior")).is_some()
            })
        })
}

/// Parse a real ship `unit` pack (`entries.*.def.interior`) into walkable rooms & portals (`SSH-C-02`).
fn parse_unit_interior_pack(pack_val: &Value) -> Option<(Vec<RoomGeom>, Vec<PortalGeom>)> {
    let entries = pack_val.get("entries")?.as_object()?;
    let interior = entries.values().find_map(|row| {
        (row.get("kind").and_then(Value::as_str) == Some("unit"))
            .then(|| row.get("def").and_then(|d| d.get("interior")))
            .flatten()
    })?;
    let raw_rooms = interior.get("rooms")?.as_array()?;
    let raw_exits = interior.get("exits")?.as_array()?;

    // Collect walkable compartments with authored names.
    let mut walkable_names: HashMap<String, String> = HashMap::new();
    let mut walkable_order: Vec<String> = Vec::new();
    for r in raw_rooms {
        let kind = r.get("kind").and_then(Value::as_str).unwrap_or("walkable");
        if kind != "walkable" {
            continue;
        }
        let Some(id) = r.get("id").and_then(Value::as_str).map(sanitize) else {
            continue;
        };
        let name = r
            .get("name")
            .and_then(Value::as_str)
            .map(sanitize)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| id.clone());
        walkable_order.push(id.clone());
        walkable_names.insert(id, name);
    }
    if walkable_order.is_empty() {
        return None;
    }

    // Assign canonical footprints for standard compartments (`airlock`, `passage`, `crew`, `bridge`,
    // `hold`, `engineering`) and lay out any additional walkable compartments cleanly alongside.
    let mut rooms: Vec<RoomGeom> = Vec::new();
    let mut by_id: HashMap<String, usize> = HashMap::new();
    let mut extra_idx = 0usize;

    for id in &walkable_order {
        let name = walkable_names
            .get(id)
            .cloned()
            .unwrap_or_else(|| id.clone());
        let (floor, min_x, min_y, max_x, max_y, kind) = match id.as_str() {
            "airlock" => (0, -3.5, 5.0, 3.5, 11.0, "room"),
            "passage" => (0, -4.0, -5.0, 4.0, 5.0, "corridor"),
            "crew" => (0, -4.5, -12.0, 4.5, -5.0, "room"),
            "bridge" => (1, -4.5, -1.0, 4.5, 9.0, "bridge"),
            "hold" => (-1, -5.5, -6.0, 5.5, 6.0, "hold"),
            "engineering" => (-2, -4.5, -8.0, 4.5, 2.0, "eng"),
            _ => {
                let col = (extra_idx as f64) + 1.0;
                extra_idx += 1;
                (
                    0,
                    4.0 + (col - 1.0) * 8.0,
                    -4.0,
                    4.0 + col * 8.0,
                    4.0,
                    "room",
                )
            }
        };
        let idx = rooms.len();
        by_id.insert(id.clone(), idx);
        rooms.push(RoomGeom {
            entry_id: id.clone(),
            name,
            floor,
            min_x,
            min_y,
            max_x,
            max_y,
            kind,
        });
    }

    let mut portals: Vec<PortalGeom> = Vec::new();
    for ex in raw_exits {
        let Some(from_id) = ex.get("from").and_then(Value::as_str).map(sanitize) else {
            continue;
        };
        let Some(to_id) = ex.get("to").and_then(Value::as_str).map(sanitize) else {
            continue;
        };
        let (Some(&idx_a), Some(&idx_b)) = (by_id.get(&from_id), by_id.get(&to_id)) else {
            continue;
        };
        let ra = &rooms[idx_a];
        let rb = &rooms[idx_b];
        let names = ex.get("names").and_then(Value::as_array);
        let cmd_ab = match names.and_then(|arr| arr.first()).and_then(Value::as_str) {
            Some(raw) => extract_exit_command(raw),
            None => super::app::exit_command(&to_id),
        };
        let cmd_ba = match names.and_then(|arr| arr.get(1)).and_then(Value::as_str) {
            Some(raw) => extract_exit_command(raw),
            None => super::app::exit_command(&from_id),
        };

        if ra.floor != rb.floor {
            // Place inter-deck ladders well clear (> 2.2 m) of each compartment's center (`SSH-C-02`).
            let (ax, ay, bx, by) = match (from_id.as_str(), to_id.as_str()) {
                ("passage", "bridge") => (0.0, 2.8, 0.0, 0.5),
                ("passage", "hold") => (0.0, -2.8, 0.0, 3.8),
                ("hold", "engineering") => (0.0, -3.8, 0.0, 0.5),
                _ => {
                    let (acx, acy) = ra.center();
                    let (bcx, bcy) = rb.center();
                    (
                        acx,
                        (acy + 2.5).clamp(ra.min_y + 1.0, ra.max_y - 1.0),
                        bcx,
                        (bcy - 2.5).clamp(rb.min_y + 1.0, rb.max_y - 1.0),
                    )
                }
            };
            if let Some(fwd) = cmd_ab {
                portals.push(PortalGeom {
                    from_room: from_id.clone(),
                    to_room: to_id.clone(),
                    command: fwd,
                    x_m: ax,
                    y_m: ay,
                    floor: ra.floor,
                    width_m: 2.0,
                    stairs_to: Some(rb.floor),
                    teleport: false,
                });
            }
            if let Some(rev) = cmd_ba {
                portals.push(PortalGeom {
                    from_room: to_id,
                    to_room: from_id,
                    command: rev,
                    x_m: bx,
                    y_m: by,
                    floor: rb.floor,
                    width_m: 2.0,
                    stairs_to: Some(ra.floor),
                    teleport: false,
                });
            }
        } else if let Some((mx, my)) = shared_edge_midpoint(ra, rb) {
            if let Some(fwd) = cmd_ab {
                portals.push(PortalGeom {
                    from_room: from_id.clone(),
                    to_room: to_id.clone(),
                    command: fwd,
                    x_m: mx,
                    y_m: my,
                    floor: ra.floor,
                    width_m: 2.0,
                    stairs_to: None,
                    teleport: false,
                });
            }
            if let Some(rev) = cmd_ba {
                portals.push(PortalGeom {
                    from_room: to_id,
                    to_room: from_id,
                    command: rev,
                    x_m: mx,
                    y_m: my,
                    floor: rb.floor,
                    width_m: 2.0,
                    stairs_to: None,
                    teleport: false,
                });
            }
        } else {
            let (ax, ay) = boundary_point_toward(ra, rb);
            let (bx, by) = boundary_point_toward(rb, ra);
            if let Some(fwd) = cmd_ab {
                portals.push(PortalGeom {
                    from_room: from_id.clone(),
                    to_room: to_id.clone(),
                    command: fwd,
                    x_m: ax,
                    y_m: ay,
                    floor: ra.floor,
                    width_m: 2.0,
                    stairs_to: None,
                    teleport: true,
                });
            }
            if let Some(rev) = cmd_ba {
                portals.push(PortalGeom {
                    from_room: to_id,
                    to_room: from_id,
                    command: rev,
                    x_m: bx,
                    y_m: by,
                    floor: rb.floor,
                    width_m: 2.0,
                    stairs_to: None,
                    teleport: true,
                });
            }
        }
    }

    Some((rooms, portals))
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

/// Canonical goods metadata matching `crates/tacsim/content/market/goods.json` (`SSH-H-02`).
fn default_goods_meta() -> HashMap<String, (String, i64, i64)> {
    let mut m = HashMap::new();
    for &(id, label, mass_kg, vol_l) in &[
        ("metals", "metal concentrate", 1000_i64, 400_i64),
        ("volatiles", "water ice", 1000, 1400),
        ("structural_alloys", "structural alloy stock", 1000, 300),
        ("industrial_parts", "yard fittings, crated", 1000, 2300),
        ("propellant", "reaction mass", 1000, 1000),
        ("provisions", "voyage provisions", 4, 10),
        ("instrument_boards", "instrument boards, crated", 1000, 2550),
        ("carbon_feedstock", "carbon billet, pressed", 1000, 600),
        ("metal_ore", "metal ore, run-of-mine", 1000, 500),
        ("ice_ore", "ice ore, run-of-mine", 1000, 1000),
        ("carbon_ore", "carbon ore, run-of-mine", 1000, 1200),
        ("strategic_ore", "strategic ore, run-of-mine", 1000, 600),
        ("ammo_ap_30", "30 mm AP rounds", 1, 1),
        ("ammo_ball_762", "7.62 mm ball, boxed", 3, 3),
        ("ammo_he_105", "105 mm HE rounds", 21, 30),
        ("ammo_missile", "anti-armor missiles, canistered", 50, 100),
    ] {
        m.insert(id.to_string(), (label.to_string(), mass_kg, vol_l));
    }
    m
}

/// Canonical Ceres Freeport fallback matching `crates/tacsim/content/atlas/neighborhood.ceres_freeport.json` (`SSH-C-02`).
fn builtin_ceres_layout() -> (Vec<RoomGeom>, Vec<PortalGeom>) {
    let rooms = vec![
        RoomGeom {
            entry_id: "room.c01_berth_gallery".into(),
            name: "Visiting Berth Gallery".into(),
            floor: 1,
            min_x: 110.0,
            min_y: 110.0,
            max_x: 210.0,
            max_y: 125.0,
            kind: "berth",
        },
        RoomGeom {
            entry_id: "room.c02_pressure".into(),
            name: "Pressure Lobby".into(),
            floor: 1,
            min_x: 110.0,
            min_y: 125.0,
            max_x: 140.0,
            max_y: 140.0,
            kind: "room",
        },
        RoomGeom {
            entry_id: "room.c03_arrivals".into(),
            name: "Port Watch Arrivals".into(),
            floor: 1,
            min_x: 110.0,
            min_y: 140.0,
            max_x: 140.0,
            max_y: 165.0,
            kind: "desk",
        },
        RoomGeom {
            entry_id: "room.c04_concourse".into(),
            name: "Freeport Concourse".into(),
            floor: 1,
            min_x: 140.0,
            min_y: 140.0,
            max_x: 175.0,
            max_y: 165.0,
            kind: "corridor",
        },
        RoomGeom {
            entry_id: "room.c05_registry".into(),
            name: "Registry & Co-op Desk".into(),
            floor: 1,
            min_x: 110.0,
            min_y: 165.0,
            max_x: 140.0,
            max_y: 190.0,
            kind: "desk",
        },
        RoomGeom {
            entry_id: "room.c06_clearing".into(),
            name: "Ceres Clearing Office".into(),
            floor: 1,
            min_x: 140.0,
            min_y: 165.0,
            max_x: 175.0,
            max_y: 190.0,
            kind: "desk",
        },
        RoomGeom {
            entry_id: "room.c07_exchange".into(),
            name: "Exchange Hall".into(),
            floor: 1,
            min_x: 175.0,
            min_y: 140.0,
            max_x: 210.0,
            max_y: 165.0,
            kind: "desk",
        },
        RoomGeom {
            entry_id: "room.c08_chandlery".into(),
            name: "Co-op Chandlery".into(),
            floor: 1,
            min_x: 175.0,
            min_y: 125.0,
            max_x: 210.0,
            max_y: 140.0,
            kind: "desk",
        },
        RoomGeom {
            entry_id: "room.c09_tycho".into(),
            name: "Tycho Mutual Agency".into(),
            floor: 1,
            min_x: 175.0,
            min_y: 165.0,
            max_x: 210.0,
            max_y: 190.0,
            kind: "desk",
        },
        RoomGeom {
            entry_id: "room.c10_slack_line".into(),
            name: "The Slack Line".into(),
            floor: 1,
            min_x: 140.0,
            min_y: 125.0,
            max_x: 175.0,
            max_y: 140.0,
            kind: "room",
        },
        RoomGeom {
            entry_id: "room.c14_dockmaster".into(),
            name: "Dockmaster's Office".into(),
            floor: 0,
            min_x: 110.0,
            min_y: 140.0,
            max_x: 140.0,
            max_y: 165.0,
            kind: "desk",
        },
    ];

    let mut portals = Vec::new();
    let mut link =
        |a: &str, b: &str, cmd_ab: &str, cmd_ba: &str, x: f64, y: f64, f_a: i64, f_b: i64| {
            let stairs_ab = (f_a != f_b).then_some(f_b);
            let stairs_ba = (f_a != f_b).then_some(f_a);
            if let Some(command) = super::app::exit_command(cmd_ab) {
                portals.push(PortalGeom {
                    from_room: a.into(),
                    to_room: b.into(),
                    command,
                    x_m: x,
                    y_m: y,
                    floor: f_a,
                    width_m: 2.8,
                    stairs_to: stairs_ab,
                    teleport: false,
                });
            }
            if let Some(command) = super::app::exit_command(cmd_ba) {
                portals.push(PortalGeom {
                    from_room: b.into(),
                    to_room: a.into(),
                    command,
                    x_m: x,
                    y_m: y,
                    floor: f_b,
                    width_m: 2.8,
                    stairs_to: stairs_ba,
                    teleport: false,
                });
            }
        };

    link(
        "room.c01_berth_gallery",
        "room.c02_pressure",
        "n",
        "s",
        125.0,
        125.0,
        1,
        1,
    );
    link(
        "room.c02_pressure",
        "room.c03_arrivals",
        "n",
        "s",
        125.0,
        140.0,
        1,
        1,
    );
    link(
        "room.c03_arrivals",
        "room.c04_concourse",
        "e",
        "w",
        140.0,
        152.5,
        1,
        1,
    );
    link(
        "room.c03_arrivals",
        "room.c05_registry",
        "n",
        "s",
        125.0,
        165.0,
        1,
        1,
    );
    link(
        "room.c03_arrivals",
        "room.c14_dockmaster",
        "d",
        "u",
        125.0,
        150.0,
        1,
        0,
    );
    link(
        "room.c04_concourse",
        "room.c06_clearing",
        "n",
        "s",
        157.5,
        165.0,
        1,
        1,
    );
    link(
        "room.c04_concourse",
        "room.c07_exchange",
        "x",
        "w",
        175.0,
        152.5,
        1,
        1,
    );
    link(
        "room.c04_concourse",
        "room.c10_slack_line",
        "s",
        "n",
        157.5,
        140.0,
        1,
        1,
    );
    link(
        "room.c07_exchange",
        "room.c08_chandlery",
        "s",
        "n",
        192.5,
        140.0,
        1,
        1,
    );
    link(
        "room.c07_exchange",
        "room.c09_tycho",
        "n",
        "s",
        192.5,
        165.0,
        1,
        1,
    );

    (rooms, portals)
}

/// Canonical OOC Area fallback matching `crates/mush-atlas/content/neighborhood.ooc_area.json` (`SSH-C-02`).
fn builtin_ooc_layout() -> (Vec<RoomGeom>, Vec<PortalGeom>) {
    let rooms = vec![
        RoomGeom {
            entry_id: "room.welcome_hall".into(),
            name: "Welcome Hall".into(),
            floor: 0,
            min_x: 30.0,
            min_y: 75.0,
            max_x: 60.0,
            max_y: 100.0,
            kind: "room",
        },
        RoomGeom {
            entry_id: "room.tutorial_moving".into(),
            name: "Tutorial: Moving".into(),
            floor: 0,
            min_x: 60.0,
            min_y: 75.0,
            max_x: 85.0,
            max_y: 100.0,
            kind: "room",
        },
        RoomGeom {
            entry_id: "room.tutorial_talking".into(),
            name: "Tutorial: Talking".into(),
            floor: 0,
            min_x: 85.0,
            min_y: 75.0,
            max_x: 110.0,
            max_y: 100.0,
            kind: "room",
        },
        RoomGeom {
            entry_id: "room.tutorial_building".into(),
            name: "Tutorial: Building".into(),
            floor: 0,
            min_x: 85.0,
            min_y: 50.0,
            max_x: 110.0,
            max_y: 75.0,
            kind: "room",
        },
        RoomGeom {
            entry_id: "room.lounge".into(),
            name: "Lounge".into(),
            floor: 0,
            min_x: 30.0,
            min_y: 50.0,
            max_x: 85.0,
            max_y: 75.0,
            kind: "room",
        },
        RoomGeom {
            entry_id: "room.bus_stop".into(),
            name: "Bus Stop (OOC)".into(),
            floor: 0,
            min_x: 40.0,
            min_y: 25.0,
            max_x: 100.0,
            max_y: 45.0,
            kind: "room",
        },
        RoomGeom {
            entry_id: "room.bus".into(),
            name: "Studio Tour Bus".into(),
            floor: 0,
            min_x: 41.0,
            min_y: 22.0,
            max_x: 53.0,
            max_y: 25.0,
            kind: "room",
        },
        RoomGeom {
            entry_id: "room.simulator_lobby".into(),
            name: "Simulator Lobby".into(),
            floor: 0,
            min_x: 118.0,
            min_y: 60.0,
            max_x: 138.0,
            max_y: 90.0,
            kind: "desk",
        },
        RoomGeom {
            entry_id: "room.sim_red".into(),
            name: "Red Room".into(),
            floor: 0,
            min_x: 118.0,
            min_y: 90.0,
            max_x: 138.0,
            max_y: 108.0,
            kind: "pod",
        },
        RoomGeom {
            entry_id: "room.sim_blue".into(),
            name: "Blue Room".into(),
            floor: 0,
            min_x: 118.0,
            min_y: 32.0,
            max_x: 138.0,
            max_y: 60.0,
            kind: "pod",
        },
    ];

    let mut portals = Vec::new();
    let mut link = |a: &str, b: &str, cmd_ab: &str, cmd_ba: &str, x: f64, y: f64| {
        if let Some(command) = super::app::exit_command(cmd_ab) {
            portals.push(PortalGeom {
                from_room: a.into(),
                to_room: b.into(),
                command,
                x_m: x,
                y_m: y,
                floor: 0,
                width_m: 2.8,
                stairs_to: None,
                teleport: false,
            });
        }
        if let Some(command) = super::app::exit_command(cmd_ba) {
            portals.push(PortalGeom {
                from_room: b.into(),
                to_room: a.into(),
                command,
                x_m: x,
                y_m: y,
                floor: 0,
                width_m: 2.8,
                stairs_to: None,
                teleport: false,
            });
        }
    };

    link(
        "room.welcome_hall",
        "room.tutorial_moving",
        "m",
        "w",
        60.0,
        87.5,
    );
    link(
        "room.tutorial_moving",
        "room.tutorial_talking",
        "t",
        "m",
        85.0,
        87.5,
    );
    link(
        "room.tutorial_talking",
        "room.tutorial_building",
        "b",
        "t",
        97.5,
        75.0,
    );
    link(
        "room.tutorial_building",
        "room.lounge",
        "lounge",
        "b",
        85.0,
        62.5,
    );
    link(
        "room.welcome_hall",
        "room.lounge",
        "lounge",
        "w",
        45.0,
        75.0,
    );
    link("room.bus_stop", "room.bus", "bus", "ooc", 47.0, 25.0);
    link(
        "room.simulator_lobby",
        "room.sim_red",
        "r",
        "lobby",
        128.0,
        90.0,
    );
    link(
        "room.simulator_lobby",
        "room.sim_blue",
        "b",
        "lobby",
        128.0,
        60.0,
    );

    // Non-adjacent corridor doorways (lounge <-> bus_stop, welcome_hall <-> simulator_lobby)
    for (from_room, to_room, raw_cmd, x_m, y_m) in [
        ("room.lounge", "room.bus_stop", "bs", 70.0, 51.0),
        ("room.bus_stop", "room.lounge", "lounge", 70.0, 44.0),
        (
            "room.welcome_hall",
            "room.simulator_lobby",
            "simulator",
            59.0,
            82.0,
        ),
        (
            "room.simulator_lobby",
            "room.welcome_hall",
            "w",
            119.0,
            75.0,
        ),
    ] {
        if let Some(command) = super::app::exit_command(raw_cmd) {
            portals.push(PortalGeom {
                from_room: from_room.into(),
                to_room: to_room.into(),
                command,
                x_m,
                y_m,
                floor: 0,
                width_m: 2.8,
                stairs_to: None,
                teleport: true,
            });
        }
    }

    (rooms, portals)
}

/// Canonical Dray interior fallback matching `crates/tacsim/content/units/dray.json` (`SSH-C-02`):
/// `airlock <-> passage <-> {bridge(u), crew(aft), hold(d) <-> engineering(d)}`, with all stairs
/// separated > 2.2 m from room centers so zero floor ping-pong can occur.
fn builtin_dray_layout() -> (Vec<RoomGeom>, Vec<PortalGeom>) {
    let rooms = vec![
        RoomGeom {
            entry_id: "engineering".into(),
            name: "Engineering Access".into(),
            floor: -2,
            min_x: -4.5,
            min_y: -8.0,
            max_x: 4.5,
            max_y: 2.0,
            kind: "eng",
        },
        RoomGeom {
            entry_id: "hold".into(),
            name: "Cargo Hold".into(),
            floor: -1,
            min_x: -5.5,
            min_y: -6.0,
            max_x: 5.5,
            max_y: 6.0,
            kind: "hold",
        },
        RoomGeom {
            entry_id: "airlock".into(),
            name: "Airlock".into(),
            floor: 0,
            min_x: -3.5,
            min_y: 5.0,
            max_x: 3.5,
            max_y: 11.0,
            kind: "room",
        },
        RoomGeom {
            entry_id: "passage".into(),
            name: "Central Passage".into(),
            floor: 0,
            min_x: -4.0,
            min_y: -5.0,
            max_x: 4.0,
            max_y: 5.0,
            kind: "corridor",
        },
        RoomGeom {
            entry_id: "crew".into(),
            name: "Crew Compartment".into(),
            floor: 0,
            min_x: -4.5,
            min_y: -12.0,
            max_x: 4.5,
            max_y: -5.0,
            kind: "room",
        },
        RoomGeom {
            entry_id: "bridge".into(),
            name: "Bridge".into(),
            floor: 1,
            min_x: -4.5,
            min_y: -1.0,
            max_x: 4.5,
            max_y: 9.0,
            kind: "bridge",
        },
    ];

    let mut portals = Vec::new();
    let link_horiz = |portals: &mut Vec<PortalGeom>,
                      a: &str,
                      b: &str,
                      cmd_ab: &str,
                      cmd_ba: &str,
                      x: f64,
                      y: f64,
                      f: i64| {
        if let Some(command) = super::app::exit_command(cmd_ab) {
            portals.push(PortalGeom {
                from_room: a.into(),
                to_room: b.into(),
                command,
                x_m: x,
                y_m: y,
                floor: f,
                width_m: 2.0,
                stairs_to: None,
                teleport: false,
            });
        }
        if let Some(command) = super::app::exit_command(cmd_ba) {
            portals.push(PortalGeom {
                from_room: b.into(),
                to_room: a.into(),
                command,
                x_m: x,
                y_m: y,
                floor: f,
                width_m: 2.0,
                stairs_to: None,
                teleport: false,
            });
        }
    };
    #[allow(clippy::too_many_arguments)]
    let link_stair = |portals: &mut Vec<PortalGeom>,
                      a: &str,
                      b: &str,
                      cmd_ab: &str,
                      cmd_ba: &str,
                      ax: f64,
                      ay: f64,
                      f_a: i64,
                      bx: f64,
                      by: f64,
                      f_b: i64| {
        if let Some(command) = super::app::exit_command(cmd_ab) {
            portals.push(PortalGeom {
                from_room: a.into(),
                to_room: b.into(),
                command,
                x_m: ax,
                y_m: ay,
                floor: f_a,
                width_m: 2.0,
                stairs_to: Some(f_b),
                teleport: false,
            });
        }
        if let Some(command) = super::app::exit_command(cmd_ba) {
            portals.push(PortalGeom {
                from_room: b.into(),
                to_room: a.into(),
                command,
                x_m: bx,
                y_m: by,
                floor: f_b,
                width_m: 2.0,
                stairs_to: Some(f_a),
                teleport: false,
            });
        }
    };

    link_horiz(
        &mut portals,
        "airlock",
        "passage",
        "inner",
        "airlock",
        0.0,
        5.0,
        0,
    );
    link_horiz(
        &mut portals,
        "passage",
        "crew",
        "crew",
        "passage",
        0.0,
        -5.0,
        0,
    );
    link_stair(
        &mut portals,
        "passage",
        "bridge",
        "bridge",
        "passage",
        0.0,
        2.8,
        0,
        0.0,
        0.5,
        1,
    );
    link_stair(
        &mut portals,
        "passage",
        "hold",
        "hold",
        "passage",
        0.0,
        -2.8,
        0,
        0.0,
        3.8,
        -1,
    );
    link_stair(
        &mut portals,
        "hold",
        "engineering",
        "engineering",
        "hold",
        0.0,
        -3.8,
        -1,
        0.0,
        0.5,
        -2,
    );

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

    /// Real `build_port_trade_frame` fixture matching `crates/tacsim/src/client_sync.rs:1280-1347`
    /// and `crates/tacsim/src/port/ship.rs:1593-1773`.
    fn live_port_trade_fixture() -> Value {
        json!({
            "schema": { "id": "port.trade", "version": 1 },
            "enabled": true,
            "venue": "ceres",
            "port_name": "Ceres Freeport",
            "aboard_docked_port": null,
            "balance_minor": 150_000,
            "balance_text": "1,500.00cr",
            "listings": [
                {
                    "id": 42,
                    "side": "ask",
                    "good": "volatiles",
                    "good_label": "water ice",
                    "vendor_name": "Occator W&P",
                    "is_personal": false,
                    "funded": true,
                    "unit_price_minor": 10_000,
                    "price_text": "100.00cr",
                    "available_qty": 50
                },
                {
                    "id": 43,
                    "side": "bid",
                    "good": "metals",
                    "good_label": "metal concentrate",
                    "vendor_name": "Freeport Exchange",
                    "is_personal": false,
                    "funded": true,
                    "unit_price_minor": 21_000,
                    "price_text": "210.00cr",
                    "available_qty": 30
                }
            ],
            "next_after": 99,
            "open_quotes": [],
            "ship": {
                "hull_id": "#401",
                "hull_name": "Salt Moth",
                "mark": "FPC-D-0401",
                "unit": "tacsim.unit.dray",
                "class": "hauler",
                "docked": true,
                "port": "ceres_freeport",
                "standing": "charter",
                "sales": 1,
                "buyout_minor": 12_000_000,
                "buyout_text": "120,000.00cr",
                "stowage": {
                    "cargo_kg": 4_000,
                    "cargo_limit_kg": 100_000,
                    "cargo_l": 1_600,
                    "cargo_limit_l": 120_000,
                    "tank_kg": 40_000,
                    "tank_limit_kg": 80_000,
                    "stores_days": 20,
                    "stores_limit_days": 120,
                    "items": [
                        { "good": "metals", "qty": 4 }
                    ]
                }
            },
            "berth": {
                "port_entry": "ceres_freeport",
                "docked_count": 2,
                "docked_hulls": [
                    { "id": "#401", "name": "Salt Moth", "mark": "FPC-D-0401", "unit": "tacsim.unit.dray" },
                    { "id": "#402", "name": "Copperfinch", "mark": "FPC-D-0402", "unit": "tacsim.unit.corvette" }
                ],
                "own_hull": "#401",
                "own_hull_name": "Salt Moth",
                "own_hull_docked_here": true
            },
            "simulator": null
        })
    }

    #[test]
    fn walking_across_threshold_emits_canonical_exit_command() {
        let mut st = PortWalkState::new();
        let now = Instant::now();
        st.observe_room(
            &sample_room("Port Watch Arrivals", "room.c03_arrivals", None),
            Some("Pilot"),
        );

        // Walk North (heading 0) from c03_arrivals center (125, 152.5) through the north doorway (y = 165) into c05_registry!
        let mut emitted = Vec::new();
        for i in 0..16 {
            let t = now + Duration::from_millis(250 * i);
            let out = st.handle_key(Key::Char('w'), false, false, &[], t);
            emitted.extend(out.commands);
        }
        assert_eq!(emitted, vec!["goto n".to_string()]);
    }

    /// A real `port.trade` frame (48e L8, clients:G3): tacsim's
    /// `port_trade_frame_matches_the_clients_shared_fixture` holds it to the
    /// live shape; the web decodes the same file.
    const PORT_TRADE_FIXTURE: &str =
        include_str!("../../../../web/src/lib/fixtures/port/port_trade.json");
    /// The real `goods` pack (48e L8, clients:G1).
    const GOODS_PACK: &str = include_str!("../../../tacsim/content/market/goods.json");

    fn trade_fixture() -> Value {
        serde_json::from_str(PORT_TRADE_FIXTURE).expect("the port.trade fixture parses")
    }

    fn first_listing<'a>(frame: &'a Value, side: &str) -> &'a Value {
        frame["listings"]
            .as_array()
            .and_then(|rows| rows.iter().find(|r| r["side"] == side))
            .expect("the fixture carries a listing on that side")
    }

    #[test]
    fn port_trade_fixture_fills_the_board_wallet_hold_and_quotes() {
        let frame = trade_fixture();
        let mut st = PortWalkState::new();
        let _ = st.observe_port_trade(&frame, Instant::now());
        let listings = frame["listings"].as_array().expect("listings");
        let asks = listings.iter().filter(|r| r["side"] == "ask").count();
        let bids = listings.iter().filter(|r| r["side"] == "bid").count();
        assert!(asks > 0 && bids > 0, "the fixture has both sides");
        assert_eq!(st.asks.len(), asks);
        assert_eq!(st.bids.len(), bids);
        let ask = first_listing(&frame, "ask");
        assert_eq!(Some(st.asks[0].listing_id), ask["id"].as_i64());
        assert_eq!(Some(st.asks[0].quantity), ask["available_qty"].as_i64());
        assert_eq!(
            st.asks[0].good_name,
            ask["good_label"].as_str().expect("good_label")
        );
        assert_eq!(Some(st.credits_minor()), frame["balance_minor"].as_i64());
        let stow = &frame["ship"]["stowage"];
        assert_eq!(
            Some(st.stowage().max_cargo_kg),
            stow["cargo_limit_kg"].as_i64()
        );
        assert_eq!(
            Some(st.stowage().max_reaction_kg),
            stow["tank_limit_kg"].as_i64()
        );
        assert_eq!(
            Some(st.stowage().max_stores_days),
            stow["stores_limit_days"].as_i64()
        );
        assert_eq!(
            st.cargo_lots.len(),
            stow["items"].as_array().map_or(0, Vec::len)
        );
        assert!(
            !st.cargo_lots.is_empty(),
            "the fixture's hold carries a line"
        );
        assert_eq!(
            st.own_quotes.len(),
            frame["open_quotes"].as_array().map_or(0, Vec::len)
        );
        assert!(
            !st.own_quotes.is_empty(),
            "the fixture carries an open quote"
        );
    }

    #[test]
    fn one_click_quote_and_sell_use_live_client_sync_schema_and_equals_syntax() {
        let mut st = PortWalkState::new();
        let now = Instant::now();
        let _ = st.observe_port_trade(&live_port_trade_fixture(), now);

        assert_eq!(st.credits_minor(), 150_000);
        assert_eq!(st.stowage().cargo_kg, 4_000);
        assert_eq!(st.stowage().max_cargo_kg, 100_000);
        assert!((st.stowage().cargo_m3 - 1.6).abs() < 1e-6);
        assert!((st.stowage().max_cargo_m3 - 120.0).abs() < 1e-6);
        assert_eq!(st.stowage().reaction_kg, 40_000);
        assert_eq!(st.stowage().max_reaction_kg, 80_000);
        assert_eq!(st.asks.len(), 1);
        assert_eq!(st.bids.len(), 1);
        assert_eq!(st.cargo_lots.len(), 1);
        assert_eq!(st.trade_next_after, Some(99));

        st.open_drawer(PortDrawerKind::Trade, &[], &mut Vec::new(), now);
        // Pressing `s` on Buy tab does NOT buy (`SSH-M-05`)
        assert!(
            st.handle_key(Key::Char('s'), false, false, &[], now)
                .commands
                .is_empty()
        );
        // Pressing `b` or `Enter` emits `+market/quote <id>=<qty>` (`C-20`)
        let buy_out = st.handle_key(Key::Char('b'), false, false, &[], now);
        assert_eq!(buy_out.commands, vec!["+market/quote 42=5"]);

        // Matching live `open_quotes` arrives -> auto-emits `+market/accept 77` ONCE in `pending_orders`!
        let _ = st.observe_port_trade(
            &json!({
                "open_quotes": [
                    {
                        "quote_id": 77,
                        "listing_id": 42,
                        "good": "volatiles",
                        "market": "ceres",
                        "quantity": 5,
                        "unit_price_minor": 10_000
                    }
                ]
            }),
            now + Duration::from_millis(200),
        );
        assert_eq!(st.take_orders(), vec!["+market/accept 77"]);
        assert!(st.take_orders().is_empty());

        // Switch to Sell Bids tab: pressing `b` does NOT sell (`SSH-M-05`), pressing `s` emits `+market/sell <id>=<qty>` (`C-20`)
        let _ = st.handle_key(Key::Right, false, false, &[], now);
        assert!(
            st.handle_key(Key::Char('b'), false, false, &[], now)
                .commands
                .is_empty()
        );
        // Set qty preset to Max Fit (index 4): held is 4 -> sells 4!
        st.qty_preset_idx = 4;
        let sell_out = st.handle_key(Key::Char('s'), false, false, &[], now);
        assert_eq!(sell_out.commands, vec!["+market/sell 43=4"]);

        // With 0 cargo held, Max Fit refuses locally instead of sending qty 1 (`SSH-M-05`)
        st.cargo_lots.clear();
        let zero_sell = st.handle_key(Key::Char('s'), false, false, &[], now);
        assert!(zero_sell.commands.is_empty());
    }

    #[test]
    fn tacsim_content_sends_op_pack_without_manifest_storm_and_loads_real_packs() {
        let mut st = PortWalkState::new();
        // 1. Manifest arrives with 20+ packs (including goods, ceres, dray, stickney)
        let manifest = json!({
            "op": "manifest",
            "packs": [
                { "id": "arena.proving_ground", "category": "arena" },
                { "id": "goods", "category": "market" },
                { "id": "neighborhood.ceres_freeport", "category": "neighborhood" },
                { "id": "neighborhood.stickney_yards", "category": "neighborhood" },
                { "id": "dray", "category": "unit" }
            ]
        });
        let q1 = st.observe_tacsim_content(&manifest);
        assert!(!q1.is_empty());
        for (feed, params) in &q1 {
            assert_eq!(feed, TACSIM_CONTENT_FEED);
            assert_eq!(params.get("op").and_then(Value::as_str), Some("pack"));
            assert!(params.get("id").and_then(Value::as_str).is_some());
        }
        // Receiving the manifest a second time MUST NOT storm duplicate requests (`C-18`)
        let q2 = st.observe_tacsim_content(&manifest);
        assert!(
            q2.is_empty(),
            "duplicate manifest must not re-request packs: {q2:?}"
        );

        // 2. Real `goods.json` pack decodes `goods[]` and `measures["si.mass_kg"]` (`C-18`)
        let _ = st.observe_tacsim_content(&json!({
            "op": "pack",
            "found": true,
            "id": "goods",
            "category": "market",
            "json": serde_json::to_string(&json!({
                "goods": [{
                    "id": "rare_alloy",
                    "label": "Rare Alloy Ingot",
                    "measures": { "si.mass_kg": 1500, "si.volume_l": 500 }
                }]
            })).unwrap()
        }));
        assert_eq!(
            st.lookup_goods_meta("rare_alloy"),
            ("Rare Alloy Ingot".to_string(), 1500, 500)
        );

        // 3. Entering non-Ceres port (`neighborhood.stickney_yards`) + receiving its pack switches geometry (`SSH-C-02`)
        st.observe_room(
            &RoomView {
                id: "#500".into(),
                name: "Republic Customs Hall".into(),
                zone: None,
                desc: "Stickney customs.".into(),
                contents: Vec::new(),
                exits: Vec::new(),
                ambient: std::collections::BTreeMap::from([(
                    "atlas".into(),
                    json!({ "pack": "neighborhood.stickney_yards", "entry": "room.p03_customs" }),
                )]),
            },
            Some("Pilot"),
        );
        let stickney_q = st.drain_pack_queries(&[TACSIM_CONTENT_FEED.to_string()]);
        assert_eq!(
            stickney_q,
            vec![(
                TACSIM_CONTENT_FEED.to_string(),
                json!({ "op": "pack", "id": "neighborhood.stickney_yards" })
            )]
        );
        let _ = st.observe_tacsim_content(&json!({
            "op": "pack",
            "found": true,
            "id": "neighborhood.stickney_yards",
            "category": "neighborhood",
            "json": serde_json::to_string(&json!({
                "entries": {
                    "room.p03_customs": {
                        "kind": "room",
                        "def": {
                            "name": "Republic Customs Hall",
                            "footprint": { "rect": [[100, 140], [130, 165]] },
                            "surface": { "floor": ["structure.yard_rim", 1] }
                        }
                    },
                    "room.p04_concourse": {
                        "kind": "room",
                        "def": {
                            "name": "Yard Concourse",
                            "footprint": { "rect": [[130, 140], [170, 165]] },
                            "surface": { "floor": ["structure.yard_rim", 1] }
                        }
                    },
                    "exit.p03_p04": {
                        "kind": "exit",
                        "def": {
                            "from": "room.p03_customs",
                            "to": "room.p04_concourse",
                            "names": ["Concourse <C>;c;concourse", "Customs <CH>;ch;customs"],
                            "portal": { "mode": "door" }
                        }
                    }
                }
            })).unwrap()
        }));
        assert_eq!(st.rooms.len(), 2);
        assert_eq!(st.active_location_key, "room.p03_customs");
        assert_eq!(
            st.first_bfs_step("room.p03_customs", "room.p04_concourse"),
            Some(("goto c".to_string(), "room.p04_concourse".to_string()))
        );
    }

    #[test]
    fn one_click_quote_verifies_price_and_emits_accept() {
        let mut frame = trade_fixture();
        frame["open_quotes"] = json!([]);
        let ask = first_listing(&frame, "ask").clone();
        let ask_id = ask["id"].as_i64().expect("ask id");
        let price = ask["unit_price_minor"].as_i64().expect("ask price");
        let want = 5.min(ask["available_qty"].as_i64().expect("qty").max(1));
        let mut st = PortWalkState::new();
        let now = Instant::now();
        let _ = st.observe_port_trade(&frame, now);
        st.open_drawer(PortDrawerKind::Trade, &[], &mut Vec::new(), now);
        let buy_out = st.handle_key(Key::Enter, false, false, &[], now);
        // The quantity rides right of `=`: `+market/quote 42 5` quotes one unit.
        assert_eq!(
            buy_out.commands,
            vec![format!("+market/quote {ask_id}={want}")]
        );

        // The live quote arrives in `open_quotes` at the shown price: accept it.
        frame["open_quotes"] = json!([{
            "quote_id": 77, "listing_id": ask_id, "good": ask["good"], "market": "m",
            "quantity": want, "unit_price_minor": price, "total_minor": price * want,
            "price_text": "", "total_text": "", "expires_at": 0
        }]);
        let _ = st.observe_port_trade(&frame, now + Duration::from_millis(200));
        assert_eq!(st.take_orders(), vec!["+market/accept 77"]);
    }

    #[test]
    fn selling_into_a_bid_puts_the_quantity_right_of_equals() {
        let mut frame = trade_fixture();
        let bid = first_listing(&frame, "bid").clone();
        let bid_id = bid["id"].as_i64().expect("bid id");
        let bid_good = bid["good"].as_str().expect("bid good").to_string();
        // Stock the ship's hold with the bid's good so the local zero-held guard (`SSH-M-05`) permits selling.
        frame["ship"]["stowage"]["items"] = json!([{ "good": bid_good, "qty": 5 }]);
        let mut st = PortWalkState::new();
        let now = Instant::now();
        let _ = st.observe_port_trade(&frame, now);
        st.open_drawer(PortDrawerKind::Trade, &[], &mut Vec::new(), now);
        let _ = st.handle_key(Key::Right, false, false, &[], now);
        let out = st.handle_key(Key::Enter, false, false, &[], now);
        assert_eq!(out.commands.len(), 1);
        let line = &out.commands[0];
        let qty = line
            .strip_prefix(&format!("+market/sell {bid_id}="))
            .unwrap_or_else(|| panic!("`+market/sell <id>=<qty>`, got {line}"));
        assert!(qty.parse::<u64>().is_ok(), "a bare quantity: {line}");
    }

    #[test]
    fn content_manifest_asks_each_pack_once_with_op_pack_and_reads_goods() {
        let mut st = PortWalkState::new();
        // The goods pack sits well past the first sixteen manifest rows.
        let mut packs: Vec<Value> = (0..40)
            .map(|i| json!({ "id": format!("cast.c{i}"), "category": "cast", "sha256": "x" }))
            .collect();
        packs.push(json!({ "id": "goods", "category": "market", "sha256": "x" }));
        packs.push(json!({ "id": "neighborhood.ceres_freeport", "category": "neighborhood", "sha256": "x" }));
        let manifest = json!({ "op": "manifest", "packs": packs });
        let asked = st.observe_tacsim_content(&manifest);
        assert_eq!(
            asked,
            vec![
                (
                    TACSIM_CONTENT_FEED.to_string(),
                    json!({ "op": "pack", "id": "goods" })
                ),
                (
                    TACSIM_CONTENT_FEED.to_string(),
                    json!({ "op": "pack", "id": "neighborhood.ceres_freeport" })
                ),
            ]
        );
        assert!(
            st.observe_tacsim_content(&manifest).is_empty(),
            "a second manifest asks for nothing already asked for"
        );
        assert!(
            st.observe_tacsim_content(&json!({ "op": "pack", "found": false, "id": "goods" }))
                .is_empty()
        );
        let reply = json!({ "op": "pack", "found": true, "id": "goods", "json": GOODS_PACK });
        assert!(st.observe_tacsim_content(&reply).is_empty());
        assert_eq!(
            st.goods_meta.get("metals"),
            Some(&("metal concentrate".to_string(), 1000, 400))
        );
    }

    #[test]
    fn ship_stairs_do_not_ping_pong_between_floors() {
        let mut st = PortWalkState::new();
        let now = Instant::now();
        st.observe_room(
            &sample_room(
                "Salt Moth - Central Passage",
                "",
                Some(json!({
                    "mode": "interior",
                    "unit": "dray",
                    "room": "passage",
                    "hull_name": "Salt Moth",
                    "docked": true
                })),
            ),
            Some("Pilot"),
        );
        assert_eq!(st.active_location_key, "passage");
        assert_eq!(st.you_floor, 0);

        // Walk North toward the bridge ladder at (0, 2.8)
        for i in 0..3 {
            let _ = st.handle_key(
                Key::Char('w'),
                false,
                false,
                &[],
                now + Duration::from_millis(250 * i),
            );
        }
        assert_eq!(st.active_location_key, "bridge");
        assert_eq!(st.you_floor, 1);

        // Stepping zero / idling or continuing forward MUST NOT ping-pong back to floor 0 (`SSH-C-02`)!
        let cmds = st.tick(Duration::from_millis(50), now + Duration::from_millis(800));
        assert!(cmds.is_empty());
        assert_eq!(st.active_location_key, "bridge");
        assert_eq!(st.you_floor, 1);
    }

    #[test]
    fn occupants_and_escape_sanitization_and_speech_bubble_bound() {
        let mut st = PortWalkState::new();
        let now = Instant::now();
        let mut amb = std::collections::BTreeMap::new();
        amb.insert(
            "atlas".into(),
            json!({ "pack": "neighborhood.ceres_freeport", "entry": "room.c01_berth_gallery" }),
        );
        amb.insert(
            "occupants".into(),
            json!({
                "npcs": ["#11"],
                "dormant": ["#12"],
                "players": ["#13"],
                "things": ["#14"]
            }),
        );
        st.observe_room(
            &RoomView {
                id: "#201".into(),
                name: "Visiting \x1b]52;c;ZGF0YQ==\x07Berth Gallery".into(),
                zone: None,
                desc: "Glass gallery.".into(),
                contents: vec![
                    ObjRef {
                        id: "#11".into(),
                        name: "Henrik\x1b[31m".into(),
                    },
                    ObjRef {
                        id: "#12".into(),
                        name: "Dormant Clerk".into(),
                    },
                    ObjRef {
                        id: "#13".into(),
                        name: "Kira\x1b_Ga=t;AAAA\x1b\\".into(),
                    },
                    ObjRef {
                        id: "#14".into(),
                        name: "Berth Board".into(),
                    },
                ],
                exits: Vec::new(),
                ambient: amb,
            },
            Some("Pilot"),
        );
        assert_eq!(st.room_title, "Visiting Berth Gallery");
        assert_eq!(
            st.actors.len(),
            2,
            "dormant NPC and inanimate thing excluded"
        );
        assert_eq!(st.actors[0].name, "Henrik");
        assert!(st.actors[0].npc);
        assert_eq!(st.actors[1].name, "Kira");
        assert!(!st.actors[1].npc);

        // Speech bubbles stay bounded <= 64 even when StationWalk tick() never runs (`SSH-H-03`).
        for i in 0..120 {
            let act = ObjRef {
                id: format!("#{i}"),
                name: format!("Speaker{i}"),
            };
            st.observe_narrative(
                NarrativeKind::Say,
                Some(&act),
                &format!("\x1b[31mMessage {i} {}", "x".repeat(200)),
                now + Duration::from_millis(i as u64),
            );
        }
        assert!(st.speech_bubbles.len() <= MAX_SPEECH_BUBBLES);
        for (txt, _) in st.speech_bubbles.values() {
            assert!(!txt.contains('\x1b'));
            assert!(txt.chars().count() <= MAX_SPEECH_CHARS);
        }
    }

    #[test]
    fn drawer_log_retains_24_week_history_and_sailing_uses_canonical_ports() {
        let mut st = PortWalkState::new();
        let now = Instant::now();
        st.open_drawer(PortDrawerKind::Industry, &[], &mut Vec::new(), now);
        let history_25 = (0..25)
            .map(|w| format!("Week {w:02}: volatiles 100.00cr"))
            .collect::<Vec<_>>()
            .join("\n");
        st.observe_sys(&history_25);
        assert_eq!(
            st.drawer_log.len(),
            25,
            "all 25 lines of +market/history kept"
        );

        // Canonical ports (`H-40`) plus lane extraction venues (`clients:G7`) — no invalid legacy slugs.
        let ids: Vec<&str> = CORRIDORS.iter().map(|(id, _, _)| *id).collect();
        for major in ["ceres", "stickney", "luna", "deimos", "titan"] {
            assert!(ids.contains(&major), "missing canonical port {major}");
        }
        for invalid in ["vesta", "pallas", "hygiea", "juno"] {
            assert!(!ids.contains(&invalid), "unexpected legacy slug {invalid}");
        }
    }

    #[test]
    fn corridor_destinations_are_exactly_the_lanes_venue_words() {
        let lane: Value = serde_json::from_str(include_str!(
            "../../../tacsim/content/lane/ceres_phobos.json"
        ))
        .expect("the lane pack parses");
        let mut venues: Vec<&str> = lane["entries"]
            .as_object()
            .expect("entries")
            .values()
            .filter(|e| e["kind"] == "port")
            .filter_map(|e| e["def"]["venue"].as_str())
            .collect();
        venues.sort_unstable();
        let mut ours: Vec<&str> = CORRIDORS.iter().map(|(id, _, _)| *id).collect();
        ours.sort_unstable();
        assert_eq!(ours, venues);
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
        // Walk forward toward helm couch at y = 7.8 and press `e`
        for i in 0..4 {
            let _ = st.handle_key(
                Key::Char('w'),
                false,
                false,
                &[],
                now + Duration::from_millis(50 * i),
            );
        }
        let out = st.handle_key(
            Key::Char('e'),
            false,
            false,
            &[],
            now + Duration::from_millis(300),
        );
        assert!(out.enter_helm_cockpit);
        assert_eq!(out.commands, vec!["+helm/station command"]);
        assert!(st.seated_at_helm());
    }

    #[test]
    fn doc48_jobs_and_industry_drawers_cycle_and_emit_registered_verbs() {
        let mut st = PortWalkState::new();
        let now = Instant::now();
        st.open_drawer(PortDrawerKind::Jobs, &[], &mut Vec::new(), now);
        let jobs_out = st.handle_key(Key::Enter, false, false, &[], now);
        assert_eq!(jobs_out.commands, vec!["+market/jobs"]);

        let locker_out = st.handle_key(Key::Char('o'), false, false, &[], now);
        assert_eq!(locker_out.commands, vec!["+locker/open"]);

        let _ = st.handle_key(Key::Tab, false, false, &[], now);
        assert_eq!(st.drawer, Some(PortDrawerKind::Industry));
        let _ = st.handle_key(Key::Down, false, false, &[], now);
        let _ = st.handle_key(Key::Down, false, false, &[], now);
        let hist_out = st.handle_key(Key::Enter, false, false, &[], now);
        assert_eq!(hist_out.commands, vec!["+market/history tacsim:volatiles"]);
    }

    #[test]
    fn kitty_arrow_repeat_and_strafe_and_server_location_sync_behave_cleanly() {
        let mut st = PortWalkState::new();
        let now = Instant::now();
        st.observe_room(
            &sample_room("Port Watch Arrivals", "room.c03_arrivals", None),
            Some("Pilot"),
        );
        let y0 = st.you_y;

        // `SSH-M-01`: Under `arrows_held = true`, pressing `Key::Up` starts continuous hold
        // without double discrete jumping, and `Key::Repeat(Arrow::Up)` is suppressed.
        let _ = st.handle_key(Key::Up, true, true, &[], now);
        assert_eq!(st.held_fwd, 1);
        assert!(
            (st.you_y - y0).abs() < 1e-6,
            "hold start must not double-step before tick"
        );
        let _ = st.handle_key(
            Key::Repeat(Arrow::Up),
            true,
            true,
            &[],
            now + Duration::from_millis(50),
        );
        assert!(
            (st.you_y - y0).abs() < 1e-6,
            "Key::Repeat(Arrow::Up) must not double-step under Kitty hold mode"
        );
        let _ = st.handle_key(
            Key::Release(Arrow::Up),
            true,
            true,
            &[],
            now + Duration::from_millis(100),
        );
        assert_eq!(st.held_fwd, 0);

        // `SSH-M-01`: `q`/`x` wire cleanly to `held_strafe` when `letters_held = true`
        let _ = st.handle_key(
            Key::Char('q'),
            true,
            true,
            &[],
            now + Duration::from_millis(120),
        );
        assert_eq!(st.held_strafe, -1);
        let _ = st.handle_key(
            Key::CharRelease('q'),
            true,
            true,
            &[],
            now + Duration::from_millis(160),
        );
        assert_eq!(st.held_strafe, 0);

        // `SSH-M-02`: crossing a portal emits the exit command while leaving
        // `server_location_key` at the unconfirmed room until `observe_room` confirms!
        let mut emitted = Vec::new();
        for i in 0..16 {
            let out = st.handle_key(
                Key::Char('w'),
                false,
                false,
                &[],
                now + Duration::from_millis(250 * (i + 1)),
            );
            emitted.extend(out.commands);
        }
        assert_eq!(emitted, vec!["goto n".to_string()]);
        assert_eq!(st.active_location_key, "room.c05_registry");
        assert_eq!(
            st.server_location_key, "room.c03_arrivals",
            "server_location_key must wait for authoritative OutputEvent::Room"
        );
    }

    #[test]
    fn sailing_dock_clears_stale_venue_manifest_uses_explicit_op_and_uncached_move_preserves_entry()
    {
        let mut st = PortWalkState::new();
        let now = Instant::now();

        // 1. `ensure_queries` sends explicit `{"op": "manifest"}` to `tacsim.content`
        let mut init_q = Vec::new();
        st.ensure_queries(&[TACSIM_CONTENT_FEED.to_string()], &mut init_q, now);
        assert_eq!(
            init_q,
            vec![(TACSIM_CONTENT_FEED.to_string(), json!({ "op": "manifest" }))]
        );

        // 2. Docked at Ceres (`venue: "ceres"`), then undock into space (`venue: null`),
        // select `stickney` in Sailing drawer, and press `d` -> emits `+helm/dock stickney` (`H-40`)
        let _ = st.observe_port_trade(&live_port_trade_fixture(), now);
        assert_eq!(st.current_venue.as_deref(), Some("ceres"));
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
        let _ = st.observe_port_trade(
            &json!({ "venue": null, "port_name": null }),
            now + Duration::from_secs(1),
        );
        assert_eq!(st.current_venue, None);
        st.open_drawer(PortDrawerKind::Sailing, &[], &mut Vec::new(), now);
        // Move from index 0 (`ceres`) -> index 1 (`stickney`)
        let _ = st.handle_key(Key::Down, false, false, &[], now);
        let dock_out = st.handle_key(Key::Char('d'), false, false, &[], now);
        assert_eq!(dock_out.commands, vec!["+helm/dock stickney"]);

        // 3. Moving between two rooms of an uncached non-Ceres port BEFORE its pack arrives
        // preserves the latest `atlas_entry` and snaps to it once the pack loads.
        st.observe_room(
            &RoomView {
                id: "#500".into(),
                name: "Republic Customs Hall".into(),
                zone: None,
                desc: "Customs.".into(),
                contents: Vec::new(),
                exits: Vec::new(),
                ambient: std::collections::BTreeMap::from([(
                    "atlas".into(),
                    json!({ "pack": "neighborhood.stickney_yards", "entry": "room.p03_customs" }),
                )]),
            },
            Some("Pilot"),
        );
        assert_eq!(st.server_location_key, "room.p03_customs");
        st.observe_room(
            &RoomView {
                id: "#501".into(),
                name: "Yard Concourse".into(),
                zone: None,
                desc: "Concourse.".into(),
                contents: Vec::new(),
                exits: Vec::new(),
                ambient: std::collections::BTreeMap::from([(
                    "atlas".into(),
                    json!({ "pack": "neighborhood.stickney_yards", "entry": "room.p04_concourse" }),
                )]),
            },
            Some("Pilot"),
        );
        assert_eq!(st.server_location_key, "room.p04_concourse");
        let _ = st.observe_tacsim_content(&json!({
            "op": "pack",
            "found": true,
            "id": "neighborhood.stickney_yards",
            "category": "neighborhood",
            "json": serde_json::to_string(&json!({
                "entries": {
                    "room.p03_customs": {
                        "kind": "room",
                        "def": {
                            "name": "Republic Customs Hall",
                            "footprint": { "rect": [[100, 140], [130, 165]] },
                            "surface": { "floor": ["structure.yard_rim", 1] }
                        }
                    },
                    "room.p04_concourse": {
                        "kind": "room",
                        "def": {
                            "name": "Yard Concourse",
                            "footprint": { "rect": [[130, 140], [170, 165]] },
                            "surface": { "floor": ["structure.yard_rim", 1] }
                        }
                    }
                }
            })).unwrap()
        }));
        assert_eq!(st.server_location_key, "room.p04_concourse");
        assert_eq!(st.active_location_key, "room.p04_concourse");
    }

    #[test]
    fn pending_quote_requires_fresh_exact_listing_quantity_and_authorized_total() {
        let mut st = PortWalkState::new();
        let now = Instant::now();
        let mut initial = live_port_trade_fixture();
        // Seed a pre-existing quote on the exact same listing & qty BEFORE `+market/quote` is sent.
        initial["open_quotes"] = json!([
            {
                "quote_id": 70,
                "listing_id": 42,
                "good": "volatiles",
                "market": "ceres",
                "quantity": 5,
                "unit_price_minor": 10_000,
                "total_minor": 50_000
            }
        ]);
        let _ = st.observe_port_trade(&initial, now);
        st.open_drawer(PortDrawerKind::Trade, &[], &mut Vec::new(), now);
        let buy = st.handle_key(Key::Char('b'), false, false, &[], now);
        assert_eq!(buy.commands, vec!["+market/quote 42=5"]);
        assert!(st.pending_quote.is_some());

        // Pre-existing quote (#70), missing listing_id (#71), and wrong quantity (#72) must ALL be ignored.
        let _ = st.observe_port_trade(
            &json!({
                "open_quotes": [
                    {
                        "quote_id": 70,
                        "listing_id": 42,
                        "good": "volatiles",
                        "quantity": 5,
                        "unit_price_minor": 10_000,
                        "total_minor": 50_000
                    },
                    {
                        "quote_id": 71,
                        "good": "volatiles",
                        "quantity": 5,
                        "unit_price_minor": 10_000,
                        "total_minor": 50_000
                    },
                    {
                        "quote_id": 72,
                        "listing_id": 42,
                        "good": "volatiles",
                        "quantity": 2,
                        "unit_price_minor": 10_000,
                        "total_minor": 20_000
                    }
                ]
            }),
            now + Duration::from_millis(100),
        );
        assert!(
            st.take_orders().is_empty(),
            "stale, wildcard-listing, and mismatched-qty quotes must never be accepted"
        );
        assert!(st.pending_quote.is_some());

        // A fresh quote matching exact listing_id, exact quantity, and total <= authorized IS accepted.
        let _ = st.observe_port_trade(
            &json!({
                "open_quotes": [
                    {
                        "quote_id": 70,
                        "listing_id": 42,
                        "good": "volatiles",
                        "quantity": 5,
                        "unit_price_minor": 10_000,
                        "total_minor": 50_000
                    },
                    {
                        "quote_id": 74,
                        "listing_id": 42,
                        "good": "volatiles",
                        "quantity": 5,
                        "unit_price_minor": 10_000,
                        "total_minor": 50_000
                    }
                ]
            }),
            now + Duration::from_millis(200),
        );
        assert_eq!(st.take_orders(), vec!["+market/accept 74"]);
        assert!(st.pending_quote.is_none());

        // A fresh exact-listing/qty quote whose total_minor exceeds authorized_total MUST halt without accepting.
        let buy2 = st.handle_key(
            Key::Char('b'),
            false,
            false,
            &[],
            now + Duration::from_millis(300),
        );
        assert_eq!(buy2.commands, vec!["+market/quote 42=5"]);
        let _ = st.observe_port_trade(
            &json!({
                "open_quotes": [
                    {
                        "quote_id": 75,
                        "listing_id": 42,
                        "good": "volatiles",
                        "quantity": 5,
                        "unit_price_minor": 10_000,
                        "total_minor": 55_000
                    }
                ]
            }),
            now + Duration::from_millis(400),
        );
        assert!(st.take_orders().is_empty());
        assert!(
            st.pending_quote.is_none(),
            "total exceeding authorized_total must clear pending_quote instead of accepting"
        );
    }

    #[test]
    fn installing_neighborhood_pack_rebuilds_desks_and_occupants_on_real_floor() {
        let mut st = PortWalkState::new();
        let mut amb = std::collections::BTreeMap::new();
        amb.insert(
            "atlas".into(),
            json!({ "pack": "neighborhood.stickney_yards", "entry": "room.p03_customs" }),
        );
        amb.insert(
            "occupants".into(),
            json!({ "npcs": ["#91"], "dormant": [], "players": [], "things": [] }),
        );
        st.observe_room(
            &RoomView {
                id: "#500".into(),
                name: "Republic Customs Hall".into(),
                zone: None,
                desc: "Stickney customs.".into(),
                contents: vec![ObjRef {
                    id: "#91".into(),
                    name: "Inspector Vance".into(),
                }],
                exits: Vec::new(),
                ambient: amb,
            },
            Some("Pilot"),
        );

        // Before real pack lands, placeholder geometry puts everything on floor 0 near (0, 0).
        assert_eq!(st.you_floor, 0);
        assert!(!st.interactables.is_empty());
        assert!(st.interactables.iter().all(|it| it.floor == 0));
        assert_eq!(st.actors.len(), 1);
        assert_eq!(st.actors[0].floor, 0);

        // Installing the real neighborhood pack (floor 1, rect [100..130, 140..165]) must
        // IMMEDIATELY rebuild desks AND occupants onto floor 1 inside the real bounds.
        let _ = st.observe_tacsim_content(&json!({
            "op": "pack",
            "found": true,
            "id": "neighborhood.stickney_yards",
            "category": "neighborhood",
            "json": serde_json::to_string(&json!({
                "entries": {
                    "room.p03_customs": {
                        "kind": "room",
                        "def": {
                            "name": "Republic Customs Hall",
                            "footprint": { "rect": [[100, 140], [130, 165]] },
                            "surface": { "floor": ["structure.yard_rim", 1] }
                        }
                    }
                }
            })).unwrap()
        }));

        assert_eq!(st.you_floor, 1);
        assert!((100.0..=130.0).contains(&st.you_x));
        assert!((140.0..=165.0).contains(&st.you_y));
        assert!(
            st.interactables.len() >= 2,
            "expected customs Jobs desk + Inspector Vance NPC interactable"
        );
        for it in &st.interactables {
            assert_eq!(it.floor, 1, "stale placeholder floor on {:?}", it.label);
            assert!(
                (100.0..=130.0).contains(&it.x_m) && (140.0..=165.0).contains(&it.y_m),
                "stale placeholder coords ({}, {}) on {:?}",
                it.x_m,
                it.y_m,
                it.label
            );
        }
        assert_eq!(st.actors.len(), 1);
        assert_eq!(st.actors[0].name, "Inspector Vance");
        assert_eq!(st.actors[0].floor, 1);
        assert!((100.0..=130.0).contains(&st.actors[0].x_m));
        assert!((140.0..=165.0).contains(&st.actors[0].y_m));
    }

    #[test]
    fn interacting_during_unacknowledged_move_keeps_single_flight_and_resumes_on_room_ack() {
        let mut st = PortWalkState::new();
        let now = Instant::now();
        st.observe_room(
            &sample_room("Port Watch Arrivals", "room.c03_arrivals", None),
            Some("Pilot"),
        );

        // Walk North across doorway into `room.c05_registry` -> sends `"goto n"` once.
        let mut emitted = Vec::new();
        for i in 0..16 {
            let out = st.handle_key(
                Key::Char('w'),
                false,
                false,
                &[],
                now + Duration::from_millis(250 * i),
            );
            emitted.extend(out.commands);
        }
        assert_eq!(emitted, vec!["goto n"]);
        assert_eq!(st.active_location_key, "room.c05_registry");
        assert_eq!(st.server_location_key, "room.c03_arrivals");
        assert!(st.pending_move_target.is_some());

        // Pressing `e` while `room.c05_registry` ack is still pending MUST NOT resend `"n"`.
        let panels = vec![PORT_TRADE_FEED.to_string()];
        let press_e = st.handle_key(
            Key::Char('e'),
            false,
            false,
            &panels,
            now + Duration::from_secs(5),
        );
        assert!(
            press_e.commands.is_empty(),
            "must never duplicate an in-flight exit command: {:?}",
            press_e.commands
        );
        assert!(
            st.drawer.is_none(),
            "interaction must wait for authoritative Room acknowledgement"
        );
        assert!(st.pending_interaction.is_some());

        // Once authoritative Room ack for `room.c05_registry` arrives, queued interaction resumes.
        st.observe_room(
            &sample_room("Charter Registry", "room.c05_registry", None),
            Some("Pilot"),
        );
        assert!(st.pending_move_target.is_none());
        assert!(st.pending_interaction.is_none());
        assert!(st.drawer.is_some());
        st.close_drawer();

        // Multi-hop single-flight test: server at `c03_arrivals`, predicted 2 hops away at `c07_exchange`.
        st.observe_room(
            &sample_room("Port Watch Arrivals", "room.c03_arrivals", None),
            Some("Pilot"),
        );
        st.active_location_key = "room.c07_exchange".to_string();
        assert!(st.pending_move_target.is_none());

        let first_hop = st.handle_key(
            Key::Char('e'),
            false,
            false,
            &panels,
            now + Duration::from_secs(6),
        );
        assert_eq!(
            first_hop.commands.len(),
            1,
            "multi-hop interact must send at most ONE movement command in flight"
        );
        let (hop1_cmd, hop1_room) = st
            .first_bfs_step("room.c03_arrivals", "room.c07_exchange")
            .unwrap();
        assert_eq!(first_hop.commands, vec![hop1_cmd]);
        assert!(st.drawer.is_none());

        // Intermediate hop ack dispatches the second hop (still 1 in flight, drawer not yet open).
        st.observe_room(
            &sample_room("Intermediate Concourse", &hop1_room, None),
            Some("Pilot"),
        );
        let (hop2_cmd, hop2_room) = st.first_bfs_step(&hop1_room, "room.c07_exchange").unwrap();
        assert_eq!(hop2_room, "room.c07_exchange");
        assert_eq!(st.take_orders(), vec![hop2_cmd]);
        assert!(st.drawer.is_none());

        // Final Room ack completes the walk and resumes the queued interaction.
        st.observe_room(
            &sample_room("Port Commodity Exchange", "room.c07_exchange", None),
            Some("Pilot"),
        );
        assert!(st.pending_move_target.is_none());
        assert!(st.pending_interaction.is_none());
        assert!(st.drawer.is_some());
    }

    #[test]
    fn missing_pack_found_false_does_not_permanently_mark_loaded_and_retries_on_new_manifest() {
        let mut st = PortWalkState::new();
        let manifest_v1 = json!({
            "op": "manifest",
            "packs": [
                { "id": "goods", "category": "market", "sha256": "aaa111" },
                { "id": "neighborhood.stickney_yards", "category": "neighborhood", "sha256": "bbb111" }
            ]
        });
        let q1 = st.observe_tacsim_content(&manifest_v1);
        assert_eq!(
            q1,
            vec![(
                TACSIM_CONTENT_FEED.to_string(),
                json!({ "op": "pack", "id": "goods" })
            )]
        );

        // Server answers `found: false` (e.g. pack temporarily absent or raced).
        let _ = st.observe_tacsim_content(&json!({
            "op": "pack",
            "found": false,
            "id": "goods"
        }));
        assert!(
            !st.loaded_packs.contains_key("goods"),
            "found:false must NEVER permanently mark a pack as loaded"
        );
        assert!(!st.in_flight_packs.contains("goods"));

        // Repeating the exact same manifest must NOT storm duplicate retries.
        let q_dup = st.observe_tacsim_content(&manifest_v1);
        assert!(q_dup.is_empty(), "same manifest must not storm: {q_dup:?}");

        // A new manifest revision (updated sha256) MUST retry `goods` and load it cleanly!
        let manifest_v2 = json!({
            "op": "manifest",
            "packs": [
                { "id": "goods", "category": "market", "sha256": "aaa222" },
                { "id": "neighborhood.stickney_yards", "category": "neighborhood", "sha256": "bbb111" }
            ]
        });
        let q2 = st.observe_tacsim_content(&manifest_v2);
        assert_eq!(
            q2,
            vec![(
                TACSIM_CONTENT_FEED.to_string(),
                json!({ "op": "pack", "id": "goods" })
            )]
        );
        let _ = st.observe_tacsim_content(&json!({
            "op": "pack",
            "found": true,
            "id": "goods",
            "sha256": "aaa222",
            "category": "market",
            "json": serde_json::to_string(&json!({
                "goods": [{
                    "id": "volatiles",
                    "label": "Water Ice",
                    "measures": { "si.mass_kg": 1000, "si.volume_l": 1000 }
                }]
            })).unwrap()
        }));
        assert_eq!(
            st.loaded_packs.get("goods").map(String::as_str),
            Some("aaa222")
        );
    }

    #[test]
    fn ship_change_or_null_stowage_resets_hold_and_blocks_trade_until_fresh_snapshot() {
        let mut st = PortWalkState::new();
        let now = Instant::now();

        // Fresh state starts unloaded (no fake 100,000 kg defaults).
        assert!(!st.stowage_loaded);

        // Load Ship A (#401, Salt Moth) with 4t metals & 100t hold.
        let _ = st.observe_port_trade(&live_port_trade_fixture(), now);
        assert!(st.stowage_loaded);
        assert_eq!(st.stowage.max_cargo_kg, 100_000);
        assert_eq!(st.cargo_lots.len(), 1);
        assert!(st.max_fit_for_ask(&st.asks[0]) > 0);

        // Switch to Ship B (#402, Copperfinch) with `stowage: null` -> MUST wipe Ship A's hold!
        let _ = st.observe_port_trade(
            &json!({
                "ship": {
                    "hull_id": "#402",
                    "hull_name": "Copperfinch",
                    "unit": "tacsim.unit.corvette",
                    "docked": true,
                    "stowage": null
                }
            }),
            now + Duration::from_secs(1),
        );
        assert!(!st.stowage_loaded);
        assert_eq!(st.stowage.cargo_kg, 0);
        assert_eq!(st.stowage.max_cargo_kg, 0);
        assert!(st.cargo_lots.is_empty());
        assert_eq!(st.max_fit_for_ask(&st.asks[0]), 0);

        // Max Fit buy and Sell MUST both refuse while stowage is unloaded.
        st.open_drawer(PortDrawerKind::Trade, &[], &mut Vec::new(), now);
        st.qty_preset_idx = 4; // Max Fit
        let blocked_buy = st.handle_key(
            Key::Char('b'),
            false,
            false,
            &[],
            now + Duration::from_secs(2),
        );
        assert!(blocked_buy.commands.is_empty());

        let _ = st.handle_key(Key::Right, false, false, &[], now + Duration::from_secs(2));
        let blocked_sell = st.handle_key(
            Key::Char('s'),
            false,
            false,
            &[],
            now + Duration::from_secs(2),
        );
        assert!(blocked_sell.commands.is_empty());

        // Fresh stowage for Ship B (10t hold, 2t metals) re-enables Max Fit and Sell accurately.
        let _ = st.observe_port_trade(
            &json!({
                "ship": {
                    "hull_id": "#402",
                    "hull_name": "Copperfinch",
                    "stowage": {
                        "cargo_kg": 2_000,
                        "cargo_limit_kg": 10_000,
                        "cargo_l": 2_000,
                        "cargo_limit_l": 10_000,
                        "items": [{ "good": "metals", "qty": 2 }]
                    }
                }
            }),
            now + Duration::from_secs(3),
        );
        assert!(st.stowage_loaded);
        let sell_b = st.handle_key(
            Key::Char('s'),
            false,
            false,
            &[],
            now + Duration::from_secs(3),
        );
        assert_eq!(sell_b.commands, vec!["+market/sell 43=2"]);

        // `ship: null` clears active hull and resets stowage.
        let _ = st.observe_port_trade(&json!({ "ship": null }), now + Duration::from_secs(4));
        assert!(!st.stowage_loaded);
        assert!(st.cargo_lots.is_empty());
    }

    #[test]
    fn max_fit_uses_live_goods_catalogue_weights_after_goods_pack_arrives() {
        let mut st = PortWalkState::new();
        let now = Instant::now();

        // Trade frame arrives BEFORE `goods` pack: heavy 30t hauler power plant initially falls back to 1,000 kg.
        let _ = st.observe_port_trade(
            &json!({
                "enabled": true,
                "venue": "ceres",
                "balance_minor": 100_000_000,
                "listings": [
                    {
                        "id": 99,
                        "side": "ask",
                        "good": "tacsim.part.plant.hauler_mk1",
                        "good_label": "hauler power plant",
                        "vendor_name": "Freeport Yard",
                        "unit_price_minor": 100_000,
                        "available_qty": 100
                    }
                ],
                "ship": {
                    "hull_id": "#401",
                    "hull_name": "Salt Moth",
                    "stowage": {
                        "cargo_kg": 4_000,
                        "cargo_limit_kg": 100_000,
                        "cargo_l": 4_000,
                        "cargo_limit_l": 120_000,
                        "items": []
                    }
                }
            }),
            now,
        );
        assert_eq!(st.asks.len(), 1);
        assert_eq!(st.max_fit_for_ask(&st.asks[0]), 96);

        // Real `goods` pack lands with 30,000 kg / 30,000 L for `tacsim.part.plant.hauler_mk1`.
        let _ = st.observe_tacsim_content(&json!({
            "op": "pack",
            "found": true,
            "id": "goods",
            "category": "market",
            "json": serde_json::to_string(&json!({
                "goods": [{
                    "id": "tacsim.part.plant.hauler_mk1",
                    "label": "hauler power plant",
                    "measures": { "si.mass_kg": 30_000, "si.volume_l": 30_000 }
                }]
            })).unwrap()
        }));

        // Free hold is 96,000 kg / 116,000 L -> 96,000 / 30,000 = 3 units (NOT 96!).
        assert_eq!(st.asks[0].unit_mass_kg, 30_000);
        assert_eq!(st.asks[0].unit_vol_dm3, 30_000);
        assert_eq!(st.max_fit_for_ask(&st.asks[0]), 3);

        // Executing Max Fit buy immediately quotes 3 units, not 96.
        st.open_drawer(PortDrawerKind::Trade, &[], &mut Vec::new(), now);
        st.qty_preset_idx = 4; // Max Fit
        let buy = st.handle_key(
            Key::Char('b'),
            false,
            false,
            &[],
            now + Duration::from_secs(1),
        );
        assert_eq!(buy.commands, vec!["+market/quote 99=3"]);
    }

    #[test]
    fn station_walk_prefixes_goto_and_drops_control_or_bidi_exits() {
        // Direct extract_exit_command checks (`P2-CLIENT-2`).
        assert_eq!(
            extract_exit_command("Pressure Lobby <N>;n;north").as_deref(),
            Some("goto n")
        );
        assert_eq!(
            extract_exit_command("Concourse <C>").as_deref(),
            Some("goto c")
        );
        assert_eq!(
            extract_exit_command("@destroy me").as_deref(),
            Some("goto @destroy me")
        );
        assert_eq!(
            extract_exit_command("Innocent;QUIT").as_deref(),
            Some("goto quit")
        );
        assert_eq!(
            extract_exit_command("Innocent;\"spoofed").as_deref(),
            Some("goto \"spoofed")
        );
        assert_eq!(extract_exit_command("North\nQUIT"), None);
        assert_eq!(extract_exit_command("North\r\n@boot all"), None);
        assert_eq!(extract_exit_command("North;bad\x1b[31m"), None);
        assert_eq!(extract_exit_command("North\u{202E}evil"), None);
        assert_eq!(extract_exit_command("North\u{2028}QUIT"), None);
        assert_eq!(extract_exit_command("   "), None);

        // Built-in layouts must prefix every single portal with `goto `.
        for (_, portals) in [
            builtin_ceres_layout(),
            builtin_ooc_layout(),
            builtin_dray_layout(),
        ] {
            assert!(!portals.is_empty());
            for p in &portals {
                assert!(
                    p.command.starts_with("goto "),
                    "unprefixed built-in portal command: {:?}",
                    p.command
                );
            }
        }

        // Reconciled neighborhood pack with hostile exit names:
        // - raw commands (`@destroy me`, `QUIT`) are prefixed with `goto `
        // - control/BiDi exit names are dropped entirely (never falling back to raw `to_id`)
        let pack = json!({
            "entries": {
                "room.a": {
                    "kind": "room",
                    "def": {
                        "name": "Room A",
                        "footprint": { "rect": [[0, 0], [20, 20]] },
                        "surface": { "floor": ["deck", 0] }
                    }
                },
                "room.b": {
                    "kind": "room",
                    "def": {
                        "name": "Room B",
                        "footprint": { "rect": [[20, 0], [40, 20]] },
                        "surface": { "floor": ["deck", 0] }
                    }
                },
                "room.c": {
                    "kind": "room",
                    "def": {
                        "name": "Room C",
                        "footprint": { "rect": [[0, 20], [20, 40]] },
                        "surface": { "floor": ["deck", 0] }
                    }
                },
                "exit.ab": {
                    "kind": "exit",
                    "def": {
                        "from": "room.a",
                        "to": "room.b",
                        "names": ["Trap;@destroy me", "Back;QUIT"]
                    }
                },
                "exit.ac": {
                    "kind": "exit",
                    "def": {
                        "from": "room.a",
                        "to": "room.c",
                        "names": ["Bad\nQUIT", "Bad\u{202E}Back"]
                    }
                }
            }
        });
        let (_, portals) = parse_neighborhood_pack(&pack).expect("neighborhood parses");
        assert_eq!(portals.len(), 2, "control/BiDi exits must be dropped");
        assert_eq!(portals[0].command, "goto @destroy me");
        assert_eq!(portals[1].command, "goto quit");

        // Unit interior pack with hostile names & control chars behaves identically.
        let unit_pack = json!({
            "entries": {
                "unit.test": {
                    "kind": "unit",
                    "def": {
                        "interior": {
                            "rooms": [
                                { "id": "airlock", "name": "Airlock", "kind": "walkable" },
                                { "id": "passage", "name": "Passage", "kind": "walkable" },
                                { "id": "crew", "name": "Crew", "kind": "walkable" }
                            ],
                            "exits": [
                                {
                                    "from": "airlock",
                                    "to": "passage",
                                    "names": ["Hatch;\"say owned", "Out;\\emit boom"]
                                },
                                {
                                    "from": "passage",
                                    "to": "crew",
                                    "names": ["Broken\r\nQUIT", "Broken\u{2029}back"]
                                }
                            ]
                        }
                    }
                }
            }
        });
        let (_, ship_portals) = parse_unit_interior_pack(&unit_pack).expect("unit pack parses");
        assert_eq!(
            ship_portals.len(),
            2,
            "unit control/separator exits must be dropped"
        );
        assert_eq!(ship_portals[0].command, "goto \"say owned");
        assert_eq!(ship_portals[1].command, "goto \\emit boom");
    }

    #[test]
    fn quest_markers_decorate_walk_actors_and_hand_off_interact_to_quest_dialog() {
        let mut st = PortWalkState::new();
        let mut ambient = std::collections::BTreeMap::new();
        ambient.insert(
            "atlas".into(),
            json!({ "pack": "neighborhood.ceres_freeport", "entry": "room.c03_arrivals" }),
        );
        ambient.insert(
            "npcs".into(),
            json!([
                { "id": "#42", "name": "Maren Halvorsen" }
            ]),
        );
        ambient.insert(
            "quest_markers".into(),
            json!([
                {
                    "npc_id": "#42",
                    "npc_name": "Maren Halvorsen",
                    "marker": "available",
                    "symbol": "!",
                    "quest_key": "tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll",
                    "quest_title": "First Papers at the Registry",
                    "category": "guided"
                }
            ]),
        );
        let rv = RoomView {
            id: "#10".into(),
            name: "Port Watch Arrivals".into(),
            desc: mush_proto::RichText::plain("Arrivals desk."),
            exits: Vec::new(),
            contents: Vec::new(),
            zone: None,
            ambient,
        };
        st.observe_room(&rv, Some("Pilot"));

        // 1. Kitty scene JSON carries gold '!' quest marker on Maren
        let scene = st.to_feed_value();
        let actors = scene["actors"].as_array().expect("actors array");
        let maren = actors
            .iter()
            .find(|a| a["name"] == "Maren Halvorsen")
            .expect("Maren spawned in arrivals");
        assert_eq!(maren["quest_marker"], "!");
        assert_eq!(maren["quest_gold"], true);

        // 2. Focus Maren and verify [E] focus prompt & direct quest dialog handoff
        let maren_idx = st
            .interactables
            .iter()
            .position(|i| i.label.contains("Maren Halvorsen"))
            .expect("Maren interactable");
        st.focus_idx = maren_idx;
        assert_eq!(
            st.focused_label(),
            Some("[!] Maren Halvorsen — Quest Available")
        );

        let now = Instant::now();
        let out = st.handle_key(Key::Char('e'), false, false, &[], now);
        assert_eq!(
            out.open_quest_for_npc,
            Some(("#42".to_string(), "Maren Halvorsen".to_string())),
            "pressing [E] on available-quest NPC must hand off directly to Quest Dialog"
        );
        assert!(
            !st.drawer_open(),
            "generic Talk drawer must not open when handing off to Quest Dialog"
        );

        // 3. Updating marker to in_progress switches badge to silver '?' and [E] opens Talk drawer
        st.observe_quest_markers(&[QuestMarker {
            npc_id: "#42".into(),
            npc_name: "Maren Halvorsen".into(),
            marker: "in_progress".into(),
            symbol: "?".into(),
            quest_key: "tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll".into(),
            quest_title: "First Papers at the Registry".into(),
            category: "guided".into(),
        }]);
        let scene2 = st.to_feed_value();
        let maren2 = scene2["actors"]
            .as_array()
            .expect("actors")
            .iter()
            .find(|a| a["name"] == "Maren Halvorsen")
            .expect("Maren");
        assert_eq!(maren2["quest_marker"], "?");
        assert_eq!(maren2["quest_gold"], false);
        assert_eq!(st.focused_label(), Some("[?] Talk to Maren Halvorsen"));

        let out2 = st.handle_key(Key::Char('e'), false, false, &[], now);
        assert!(out2.open_quest_for_npc.is_none());
        assert!(st.drawer_open());
    }

    #[test]
    fn quest_guidance_highlights_trade_and_sailing_drawers_and_jumps_to_exact_qty() {
        let mut st = PortWalkState::new();
        let now = Instant::now();
        let mut trade_fixture = live_port_trade_fixture();
        trade_fixture["listings"]
            .as_array_mut()
            .expect("listings array")
            .push(json!({
                "id": 55,
                "side": "ask",
                "good": "industrial_parts",
                "good_label": "industrial parts",
                "vendor_name": "Freeport Yard",
                "is_personal": false,
                "funded": true,
                "unit_price_minor": 12_000,
                "price_text": "120.00cr",
                "available_qty": 10
            }));
        st.observe_port_trade(&trade_fixture, now);
        st.observe_quest_guidance(
            Some(QuestTradeNeed {
                quest_title: "Refit the Dray".into(),
                checkpoint_title: "Buy 2 industrial parts".into(),
                commodity_slug: "industrial_parts".into(),
                target_quantity: 2,
                wants_buy: true,
                wants_sell: false,
            }),
            Some(QuestDestinationHint {
                quest_title: "Deliver Metals to Stickney".into(),
                port_id: "stickney".into(),
                port_label: "Stickney Yards".into(),
            }),
        );

        let mut queries = Vec::new();
        st.open_drawer(PortDrawerKind::Trade, &[], &mut queries, now);

        // Render Trade drawer and confirm [QUEST] banner & [Q] row highlight
        let backend = ratatui::backend::TestBackend::new(84, 24);
        let mut term = ratatui::Terminal::new(backend).expect("test terminal");
        term.draw(|f| st.draw_drawer_box(f, f.area(), true))
            .expect("draw trade drawer");
        let rendered_trade = term
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(
            rendered_trade.contains("[QUEST] BUY 2 x industrial parts (g:jump+qty)"),
            "expected quest trade banner in Trade drawer: {rendered_trade}"
        );
        assert!(
            rendered_trade.contains("[Q] #55"),
            "expected [Q] tag on matching industrial_parts listing: {rendered_trade}"
        );

        // Press 'g' to jump to listing #55 and arm exact non-preset qty = 2
        let jump_out = st.handle_key(Key::Char('g'), false, false, &[], now);
        assert!(jump_out.consumed);
        assert_eq!(st.trade_subtab, 0);
        assert_eq!(st.asks[st.cursor_row].listing_id, 55);
        assert_eq!(st.active_trade_qty_setting(), 2);

        // Press Enter to submit +market/quote 55=2
        let buy_out = st.handle_key(Key::Enter, false, false, &[], now);
        assert!(
            buy_out.commands.iter().any(|c| c == "+market/quote 55=2"),
            "expected exact quest qty quote '+market/quote 55=2', got {:?}",
            buy_out.commands
        );

        // Switch to Sailing drawer and verify [QUEST] destination banner + 'g' corridor select
        st.open_drawer(PortDrawerKind::Sailing, &[], &mut queries, now);
        term.draw(|f| st.draw_drawer_box(f, f.area(), true))
            .expect("draw sailing drawer");
        let rendered_sail = term
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(
            rendered_sail.contains("[QUEST] Dest: Stickney Yards (g:select)"),
            "expected quest destination banner in Sailing drawer: {rendered_sail}"
        );
        assert!(
            rendered_sail.contains("[Q] Stickney Yards"),
            "expected [Q] tag on Stickney Yards corridor row: {rendered_sail}"
        );

        let dest_jump = st.handle_key(Key::Char('g'), false, false, &[], now);
        assert!(dest_jump.consumed);
        assert_eq!(CORRIDORS[st.corridor_idx].0, "stickney");
    }

    #[test]
    fn quest_objective_room_propagates_to_kitty_feed() {
        let mut st = PortWalkState::new();
        let mut ambient = std::collections::BTreeMap::new();
        ambient.insert(
            "atlas".into(),
            json!({ "pack": "neighborhood.ceres_freeport", "entry": "room.c03_arrivals" }),
        );
        let rv = RoomView {
            id: "#10".into(),
            name: "Port Watch Arrivals".into(),
            desc: mush_proto::RichText::plain("Arrivals desk."),
            exits: Vec::new(),
            contents: Vec::new(),
            zone: None,
            ambient,
        };
        st.observe_room(&rv, Some("Pilot"));
        st.observe_quest_objective_room(Some(("room.c03_arrivals".into(), true)));

        let feed = st.to_feed_value();
        let rooms = feed["rooms"].as_array().expect("rooms array");
        let target = rooms
            .iter()
            .find(|r| r["id"] == "room.c03_arrivals")
            .expect("arrivals room present");
        assert_eq!(target["quest_target"], true);
        assert_eq!(target["quest_turn_in"], true);

        st.observe_quest_objective_room(None);
        let feed_cleared = st.to_feed_value();
        let cleared = feed_cleared["rooms"]
            .as_array()
            .expect("rooms array")
            .iter()
            .find(|r| r["id"] == "room.c03_arrivals")
            .expect("arrivals room present");
        assert_eq!(cleared["quest_target"], false);
        assert_eq!(cleared["quest_turn_in"], false);
    }

    #[test]
    fn coop_drawer_shows_quest_refit_banner_and_dispatches_refit_on_p() {
        let mut st = PortWalkState::new();
        let now = Instant::now();
        st.observe_quest_coop_action(Some(QuestSuggestedAction {
            label: "Refit Dray-P".into(),
            command: "+coop/refit prospector".into(),
        }));
        let mut queries = Vec::new();
        st.open_drawer(PortDrawerKind::Coop, &[], &mut queries, now);

        let backend = ratatui::backend::TestBackend::new(84, 16);
        let mut term = ratatui::Terminal::new(backend).expect("test terminal");
        term.draw(|f| st.draw_drawer_box(f, f.area(), true))
            .expect("draw coop drawer");
        let rendered = term
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(
            rendered.contains("[QUEST] Refit Dray-P Prospector"),
            "expected quest refit banner in Co-op drawer: {rendered}"
        );

        let out = st.handle_key(Key::Char('p'), false, false, &[], now);
        assert!(out.consumed);
        assert!(
            out.commands.iter().any(|c| c == "+coop/refit prospector"),
            "pressing p in Co-op drawer must submit '+coop/refit prospector', got {:?}",
            out.commands
        );
    }

    #[test]
    fn coop_drawer_shows_refit_empty_hold_readiness_with_propellant_allowed() {
        let mut st = PortWalkState::new();
        let now = Instant::now();
        st.stowage_loaded = true;
        st.stowage.cargo_kg = 2_000;
        st.stowage.stores_days = 4;
        st.stowage.reaction_kg = 12_000;
        st.observe_quest_coop_action(Some(QuestSuggestedAction {
            label: "Refit Dray-P".into(),
            command: "+coop/refit prospector".into(),
        }));
        let mut queries = Vec::new();
        st.open_drawer(PortDrawerKind::Coop, &[], &mut queries, now);

        let blocked = st.quest_refit_hold_readiness().expect("refit readiness");
        assert!(!blocked.ready);
        assert!(!blocked.cargo_empty);
        assert!(!blocked.provisions_empty);
        assert_eq!(blocked.propellant_tonnes, 12);

        let draw_text = |s: &PortWalkState| -> String {
            let backend = ratatui::backend::TestBackend::new(88, 16);
            let mut term = ratatui::Terminal::new(backend).expect("test terminal");
            term.draw(|f| s.draw_drawer_box(f, f.area(), true))
                .expect("draw coop drawer");
            term.backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect()
        };

        let blocked_txt = draw_text(&st);
        assert!(
            blocked_txt.contains("[CLEAR HOLD] Cargo: 2t | Stores: 4d (Tank 12t OK)"),
            "expected blocked refit hold line: {blocked_txt}"
        );

        // Clearing cargo + provisions makes refit READY even with 12t propellant in tank
        st.stowage.cargo_kg = 0;
        st.stowage.stores_days = 0;
        let ready = st.quest_refit_hold_readiness().expect("refit ready");
        assert!(ready.ready);
        let ready_txt = draw_text(&st);
        assert!(
            ready_txt.contains("[READY] Hold empty | Stores empty | Tank 12t OK"),
            "expected ready refit hold line: {ready_txt}"
        );
    }

    #[test]
    fn cross_port_quest_routes_to_berth_boarding_and_docked_airlock_disembark() {
        let mut st = PortWalkState::new();
        let make_station_room = |entry: &str, name: &str| {
            let mut ambient = std::collections::BTreeMap::new();
            ambient.insert(
                "atlas".into(),
                json!({ "pack": "neighborhood.ceres_freeport", "entry": entry }),
            );
            RoomView {
                id: "#10".into(),
                name: name.into(),
                desc: mush_proto::RichText::plain("Station room."),
                exits: Vec::new(),
                contents: Vec::new(),
                zone: None,
                ambient,
            }
        };

        // 1. Standing at Ceres Registry with a Stickney objective -> walks toward Berth Gallery
        st.observe_room(
            &make_station_room("room.c05_registry", "Registry & Co-op Desk"),
            Some("Pilot"),
        );
        st.observe_quest_objective_room(Some(("room.p07_receiving".into(), false)));
        let hop_to_berth = st
            .next_quest_hop()
            .expect("expected cross-port hop toward berth gallery");
        assert!(hop_to_berth.next_exit_command.starts_with("goto "));
        assert!(
            hop_to_berth.target_room_name.contains("Board Ship"),
            "{:?}",
            hop_to_berth
        );
        assert!(hop_to_berth.hops >= 2);

        // 2. Arriving at Ceres Berth Gallery -> 1-hop +ship/board
        st.observe_room(
            &make_station_room("room.c01_berth_gallery", "Visiting Berth Gallery"),
            Some("Pilot"),
        );
        let board_hop = st
            .next_quest_hop()
            .expect("expected Board Ship hop at berth gallery");
        assert_eq!(board_hop.next_room_name, "Board Ship");
        assert_eq!(board_hop.next_exit_command, "+ship/board");
        assert_eq!(board_hop.hops, 1);

        // 3. Aboard ship docked at Stickney on Bridge -> routes toward Airlock
        let make_ship_room = |comp: &str| {
            let mut ambient = std::collections::BTreeMap::new();
            ambient.insert(
                "tacsim".into(),
                json!({
                    "mode": "interior",
                    "room": comp,
                    "hull_name": "Salt Moth",
                    "docked": true,
                    "port": "stickney"
                }),
            );
            RoomView {
                id: "#50".into(),
                name: format!("Salt Moth - {comp}"),
                desc: mush_proto::RichText::plain("Ship deck."),
                exits: Vec::new(),
                contents: Vec::new(),
                zone: None,
                ambient,
            }
        };
        st.observe_room(&make_ship_room("bridge"), Some("Pilot"));
        assert_eq!(st.docked_port_slug(), Some("stickney"));
        let to_airlock = st
            .next_quest_hop()
            .expect("expected bridge -> airlock hop when docked at Stickney");
        assert_eq!(to_airlock.next_room_entry, "passage");
        assert!(to_airlock.target_room_name.contains("Cycle Ashore"));
        assert!(to_airlock.hops >= 2);

        // 4. In Airlock while docked at Stickney -> 1-hop +ship/leave
        st.observe_room(&make_ship_room("airlock"), Some("Pilot"));
        let leave_hop = st
            .next_quest_hop()
            .expect("expected Cycle Ashore hop in airlock");
        assert_eq!(leave_hop.next_room_name, "Cycle Ashore");
        assert_eq!(leave_hop.next_exit_command, "+ship/leave");
        assert_eq!(leave_hop.hops, 1);
    }

    #[test]
    fn vehicle_bay_hud_shows_quest_mining_checklist_and_works_drawer_handles_refit_hotkey() {
        let mut st = PortWalkState::new();
        let now = Instant::now();
        st.aboard_ship = true;
        st.active_location_key = "bay".into();
        st.observe_quest_mining_status(Some(QuestSurfaceMiningStatus {
            quest_title: "Scan, Sound, and Mine a Landing Site".into(),
            target_ore_slug: "carbon_ore".into(),
            recommended_family: "carbon".into(),
            scan_done: true,
            sound_done: false,
            mine_done: false,
        }));
        let (bay_top, _) = st.hud_lines("kitty");
        assert!(
            bay_top.contains(
                "[QUEST] Rig Bay: [x] Scan [ ] Sound [ ] Mine carbon_ore (+tv/1 launch go)"
            ),
            "expected vehicle bay rig quest strip in HUD: {bay_top}"
        );

        // Once mine_done is true, HUD flips command cue to +tv/1 recover go
        st.observe_quest_mining_status(Some(QuestSurfaceMiningStatus {
            quest_title: "Scan, Sound, and Mine a Landing Site".into(),
            target_ore_slug: "carbon_ore".into(),
            recommended_family: "carbon".into(),
            scan_done: true,
            sound_done: true,
            mine_done: true,
        }));
        let (bay_done_top, _) = st.hud_lines("kitty");
        assert!(
            bay_done_top.contains("(+tv/1 recover go)"),
            "expected recover cue once mine_done: {bay_done_top}"
        );

        // Works & Yard drawer renders Step 5 refit banner + empty-hold readiness and dispatches on 'p'
        st.stowage_loaded = true;
        st.stowage.cargo_kg = 0;
        st.stowage.stores_days = 0;
        st.stowage.reaction_kg = 10_000;
        st.observe_quest_coop_action(Some(QuestSuggestedAction {
            label: "Refit Dray-P".into(),
            command: "+coop/refit prospector".into(),
        }));
        let mut queries = Vec::new();
        st.open_drawer(PortDrawerKind::Industry, &[], &mut queries, now);
        let backend = ratatui::backend::TestBackend::new(88, 16);
        let mut term = ratatui::Terminal::new(backend).expect("test terminal");
        term.draw(|f| st.draw_drawer_box(f, f.area(), true))
            .expect("draw works drawer");
        let rendered: String = term
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            rendered.contains("[QUEST] Refit Dray-P Prospector"),
            "expected refit banner in Works drawer: {rendered}"
        );
        assert!(
            rendered.contains("[READY] Hold empty | Stores empty | Tank 10t OK"),
            "expected ready refit hold line in Works drawer: {rendered}"
        );

        let out = st.handle_key(Key::Char('p'), false, false, &[], now);
        assert!(out.consumed);
        assert!(
            out.commands.iter().any(|c| c == "+coop/refit prospector"),
            "pressing p in Works drawer must submit '+coop/refit prospector', got {:?}",
            out.commands
        );
    }

    #[test]
    fn quest_exit_breadcrumb_computes_shortest_hop_and_updates_hud_lines() {
        let mut st = PortWalkState::new();
        let mut ambient = std::collections::BTreeMap::new();
        ambient.insert(
            "atlas".into(),
            json!({ "pack": "neighborhood.ceres_freeport", "entry": "room.c01_berth_gallery" }),
        );
        let rv = RoomView {
            id: "#10".into(),
            name: "Visiting Berth Gallery".into(),
            desc: mush_proto::RichText::plain("Berth gallery."),
            exits: Vec::new(),
            contents: Vec::new(),
            zone: None,
            ambient,
        };
        st.observe_room(&rv, Some("Pilot"));
        st.observe_quest_objective_room(Some(("room.c05_registry".into(), false)));

        let hop = st
            .next_quest_hop()
            .expect("expected BFS breadcrumb from berth gallery to registry");
        assert_eq!(hop.next_room_entry, "room.c02_pressure");
        assert_eq!(hop.next_room_name, "Pressure Lobby");
        assert_eq!(hop.hops, 3);
        assert!(hop.matches_exit("Pressure Lobby", &[]));
        assert!(!hop.matches_exit("Cargo Airlock", &[]));

        let (top, _) = st.hud_lines("kitty");
        assert!(
            top.contains("[QUEST -> Pressure Lobby (3 hops)]"),
            "expected station quest hop breadcrumb in HUD top line: {top}"
        );

        // Arriving at target clears breadcrumb cleanly
        st.observe_quest_objective_room(Some(("room.c01_berth_gallery".into(), false)));
        assert!(st.next_quest_hop().is_none());

        // Aboard ship HUD surfaces active destination port
        st.aboard_ship = true;
        st.observe_quest_guidance(
            None,
            Some(QuestDestinationHint {
                quest_title: "Run Metals to Stickney Yards".into(),
                port_id: "stickney".into(),
                port_label: "Stickney Yards".into(),
            }),
        );
        let (ship_top, _) = st.hud_lines("kitty");
        assert!(
            ship_top.contains("[QUEST] Dest: Stickney Yards"),
            "expected ship HUD quest destination: {ship_top}"
        );
    }

    #[test]
    fn quest_stowage_readiness_tracks_propellant_provisions_and_hold_cargo_in_gauges() {
        let mut st = PortWalkState::new();
        st.stowage_loaded = true;
        st.stowage.reaction_kg = 5_200;
        st.stowage.max_reaction_kg = 20_000;
        st.stowage.stores_days = 8;
        st.stowage.max_stores_days = 30;
        st.cargo_lots = vec![CargoLotRow {
            good_id: "tacsim:metals".into(),
            good_name: "Metals".into(),
            quantity: 5,
        }];

        let lines_text = |lines: Vec<Line<'static>>| -> String {
            lines
                .iter()
                .map(|l| {
                    l.spans
                        .iter()
                        .map(|s| s.content.as_ref())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        // 1. Propellant readiness (5t >= 5t -> READY + *Reaction)
        st.observe_quest_guidance(
            Some(QuestTradeNeed {
                quest_title: "Fuel".into(),
                checkpoint_title: "Buy 5 propellant".into(),
                commodity_slug: "propellant".into(),
                target_quantity: 5,
                wants_buy: true,
                wants_sell: false,
            }),
            None,
        );
        let prop_r = st.quest_stowage_readiness().expect("propellant readiness");
        assert!(prop_r.ready);
        assert_eq!(prop_r.current_qty, 5);
        assert_eq!(prop_r.gauge_kind, "propellant");
        let prop_txt = lines_text(st.stowage_gauge_lines(true));
        assert!(prop_txt.contains("*Reaction"), "{prop_txt}");
        assert!(prop_txt.contains("[READY] PROPELLANT: 5/5 t"), "{prop_txt}");

        // 2. Provisions loading (8d < 10d -> QUEST + *Stores)
        st.observe_quest_guidance(
            Some(QuestTradeNeed {
                quest_title: "Stores".into(),
                checkpoint_title: "Buy 10 provisions".into(),
                commodity_slug: "provisions".into(),
                target_quantity: 10,
                wants_buy: true,
                wants_sell: false,
            }),
            None,
        );
        let prov_r = st.quest_stowage_readiness().expect("provisions readiness");
        assert!(!prov_r.ready);
        assert_eq!(prov_r.current_qty, 8);
        let prov_txt = lines_text(st.stowage_gauge_lines(true));
        assert!(prov_txt.contains("*Stores"), "{prov_txt}");
        assert!(
            prov_txt.contains("[QUEST] PROVISIONS: 8/10 d"),
            "{prov_txt}"
        );

        // 3. Cargo hold metals (5/5 -> READY + *Hold Mass)
        st.observe_quest_guidance(
            Some(QuestTradeNeed {
                quest_title: "Metals".into(),
                checkpoint_title: "Deliver 5 metals".into(),
                commodity_slug: "metals".into(),
                target_quantity: 5,
                wants_buy: true,
                wants_sell: true,
            }),
            None,
        );
        let cargo_r = st.quest_stowage_readiness().expect("cargo readiness");
        assert!(cargo_r.ready);
        assert_eq!(cargo_r.gauge_kind, "cargo");
        let cargo_txt = lines_text(st.stowage_gauge_lines(true));
        assert!(cargo_txt.contains("*Hold Mass"), "{cargo_txt}");
        assert!(cargo_txt.contains("[READY] METALS: 5/5 u"), "{cargo_txt}");
    }

    #[test]
    fn aboard_ship_quest_breadcrumb_guides_to_bridge_and_clears_at_helm() {
        let mut st = PortWalkState::new();
        let mut ambient = std::collections::BTreeMap::new();
        ambient.insert(
            "tacsim".into(),
            json!({
                "mode": "interior",
                "unit": "tacsim.unit.dray",
                "room": "airlock",
                "hull_name": "M/V Starling",
                "docked": true
            }),
        );
        let rv = RoomView {
            id: "#50".into(),
            name: "M/V Starling - Airlock".into(),
            desc: mush_proto::RichText::plain("Airlock."),
            exits: Vec::new(),
            contents: Vec::new(),
            zone: None,
            ambient,
        };
        st.observe_room(&rv, Some("Pilot"));
        assert!(st.aboard_ship);
        st.observe_quest_ship_target(Some("bridge"));
        st.observe_quest_guidance(
            None,
            Some(QuestDestinationHint {
                quest_title: "Run Metals to Stickney Yards".into(),
                port_id: "stickney".into(),
                port_label: "Stickney Yards".into(),
            }),
        );

        let hop = st
            .next_quest_hop()
            .expect("expected aboard-ship BFS hop from airlock to bridge");
        assert_eq!(hop.next_room_entry, "passage");
        assert_eq!(hop.next_room_name, "Central Passage");
        assert_eq!(hop.hops, 2);

        let (top, _) = st.hud_lines("kitty");
        assert!(top.contains("[QUEST -> Central Passage (2 hops)]"), "{top}");
        assert!(top.contains("[QUEST] Dest: Stickney Yards"), "{top}");

        // Seating at helm clears interior walk breadcrumb
        st.seated_at_helm = true;
        assert!(st.next_quest_hop().is_none());
    }

    #[test]
    fn sailing_preflight_readiness_checks_fuel_stores_and_quest_cargo_and_jumps_to_exchange() {
        let now = Instant::now();
        let mut st = PortWalkState::new();
        st.stowage_loaded = true;
        st.stowage.reaction_kg = 0;
        st.stowage.stores_days = 0;
        st.cargo_lots = vec![CargoLotRow {
            good_id: "tacsim:metals".into(),
            good_name: "Metals".into(),
            quantity: 2,
        }];
        st.observe_quest_guidance(
            Some(QuestTradeNeed {
                quest_title: "Metals Down the Corridor".into(),
                checkpoint_title: "Deliver 5t metals to Stickney Yards".into(),
                commodity_slug: "metals".into(),
                target_quantity: 5,
                wants_buy: true,
                wants_sell: true,
            }),
            Some(QuestDestinationHint {
                quest_title: "Metals Down the Corridor".into(),
                port_id: "stickney".into(),
                port_label: "Stickney Yards".into(),
            }),
        );

        let blocked = st
            .quest_sailing_preflight_readiness()
            .expect("expected preflight readiness");
        assert!(!blocked.ready);
        assert!(!blocked.propellant_ok);
        assert!(!blocked.provisions_ok);
        assert!(blocked.cargo_required);
        assert_eq!(blocked.cargo_qty, 2);
        assert_eq!(blocked.cargo_target, 5);
        assert!(!blocked.cargo_ok);

        // Sailing drawer renders [CHECK] pre-flight line and 'b' jumps to Port Exchange (Trade)
        let mut queries = Vec::new();
        st.open_drawer(PortDrawerKind::Sailing, &[], &mut queries, now);
        let backend = ratatui::backend::TestBackend::new(96, 16);
        let mut term = ratatui::Terminal::new(backend).expect("test terminal");
        term.draw(|f| st.draw_drawer_box(f, f.area(), true))
            .expect("draw sailing drawer");
        let rendered_blocked: String = term
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            rendered_blocked.contains("[CHECK] Fuel 0t! | Stores 0d! | metals 2/5 ! (b:Exchange)"),
            "expected blocked pre-flight strip in Sailing drawer: {rendered_blocked}"
        );

        let out = st.handle_key(Key::Char('b'), false, false, &[], now);
        assert!(out.consumed);
        assert_eq!(st.drawer, Some(PortDrawerKind::Trade));

        // Top up fuel, stores, and full 5t metals -> pre-flight OK
        st.stowage.reaction_kg = 14_000;
        st.stowage.stores_days = 12;
        st.cargo_lots[0].quantity = 5;
        let ready = st
            .quest_sailing_preflight_readiness()
            .expect("expected ready preflight");
        assert!(ready.ready);
        assert!(ready.propellant_ok);
        assert_eq!(ready.propellant_tonnes, 14);
        assert!(ready.provisions_ok);
        assert_eq!(ready.provisions_days, 12);
        assert!(ready.cargo_ok);

        st.open_drawer(PortDrawerKind::Sailing, &[], &mut queries, now);
        term.draw(|f| st.draw_drawer_box(f, f.area(), true))
            .expect("draw ready sailing drawer");
        let rendered_ready: String = term
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            rendered_ready.contains("[PRE-FLIGHT OK] Fuel 14t | Stores 12d | metals 5/5 OK"),
            "expected ready pre-flight strip in Sailing drawer: {rendered_ready}"
        );
    }

    #[test]
    fn cross_port_trade_quest_routes_to_local_exchange_before_berth_until_cargo_loaded() {
        let mut st = PortWalkState::new();
        let make_room = |entry: &str, name: &str| {
            let mut ambient = std::collections::BTreeMap::new();
            ambient.insert(
                "atlas".into(),
                json!({ "pack": "neighborhood.ceres_freeport", "entry": entry }),
            );
            RoomView {
                id: "#10".into(),
                name: name.into(),
                desc: mush_proto::RichText::plain("Station deck."),
                exits: Vec::new(),
                contents: Vec::new(),
                zone: None,
                ambient,
            }
        };
        st.observe_room(
            &make_room("room.c05_registry", "Registry & Co-op Desk"),
            Some("Pilot"),
        );
        st.observe_quest_objective_room(Some(("room.p07_receiving".into(), false)));
        st.observe_quest_guidance(
            Some(QuestTradeNeed {
                quest_title: "Metals Down the Corridor".into(),
                checkpoint_title: "Buy 5t metals and deliver to Stickney Yards".into(),
                commodity_slug: "metals".into(),
                target_quantity: 5,
                wants_buy: true,
                wants_sell: true,
            }),
            Some(QuestDestinationHint {
                quest_title: "Metals Down the Corridor".into(),
                port_id: "stickney".into(),
                port_label: "Stickney Yards".into(),
            }),
        );

        // Live stowage loaded with only 2/5 metals -> routes to Exchange Hall first
        st.stowage_loaded = true;
        st.cargo_lots = vec![CargoLotRow {
            good_id: "tacsim:metals".into(),
            good_name: "Metals".into(),
            quantity: 2,
        }];
        assert!(st.needs_origin_port_procurement());
        let to_exchange = st
            .next_quest_hop()
            .expect("expected hop toward Exchange Hall while cargo incomplete");
        assert_eq!(to_exchange.target_room_name, "Exchange Hall");

        // Standing in Exchange Hall while still procuring -> None (buy here!)
        st.observe_room(
            &make_room("room.c07_exchange", "Exchange Hall"),
            Some("Pilot"),
        );
        assert!(st.next_quest_hop().is_none());

        // Once 5/5 metals are loaded -> automatically flips to Berth Gallery (Board Ship)!
        st.cargo_lots[0].quantity = 5;
        assert!(!st.needs_origin_port_procurement());
        let to_berth = st
            .next_quest_hop()
            .expect("expected hop toward Berth Gallery once metals loaded");
        assert!(
            to_berth.target_room_name.contains("Board Ship"),
            "expected Berth Gallery (Board Ship) route: {:?}",
            to_berth
        );
    }

    #[test]
    fn step6_surface_expedition_routes_berth_bridge_bay_bridge_airlock() {
        let mut st = PortWalkState::new();
        let make_station_room = |entry: &str, name: &str| {
            let mut ambient = std::collections::BTreeMap::new();
            ambient.insert(
                "atlas".into(),
                json!({ "pack": "neighborhood.ceres_freeport", "entry": entry }),
            );
            RoomView {
                id: "#17".into(),
                name: name.into(),
                desc: mush_proto::RichText::plain("Station deck."),
                exits: Vec::new(),
                contents: Vec::new(),
                zone: None,
                ambient,
            }
        };
        let make_ship_room = |comp: &str, docked: bool, settled_pad: bool, port: &str| {
            let mut ambient = std::collections::BTreeMap::new();
            ambient.insert(
                "tacsim".into(),
                json!({
                    "mode": "interior",
                    "unit": "tacsim.unit.dray_p",
                    "room": comp,
                    "hull_name": "M/V Starling",
                    "docked": docked,
                    "settled_pad": settled_pad,
                    "port": port
                }),
            );
            RoomView {
                id: "#50".into(),
                name: format!("M/V Starling - {comp}"),
                desc: mush_proto::RichText::plain("Ship deck."),
                exits: Vec::new(),
                contents: Vec::new(),
                zone: None,
                ambient,
            }
        };

        st.observe_quest_objective_room(Some(("room.c07_exchange".into(), false)));
        st.observe_quest_mining_status(Some(QuestSurfaceMiningStatus {
            quest_title: "Scan, Sound, and Mine a Landing Site".into(),
            target_ore_slug: "carbon_ore".into(),
            recommended_family: "carbon".into(),
            scan_done: false,
            sound_done: false,
            mine_done: false,
        }));

        // 1. Standing right in Rosalind's Exchange Hall ashore -> routes to Berth Gallery (Board Ship)
        st.observe_room(
            &make_station_room("room.c07_exchange", "Exchange Hall"),
            Some("Pilot"),
        );
        assert!(st.needs_surface_expedition_boarding());
        let to_berth = st.next_quest_hop().expect("expected Berth Gallery route");
        assert!(to_berth.target_room_name.contains("Board Ship"));

        // 2. Aboard Dray-P still docked at Ceres station -> routes to Bridge (fly down to surface site)
        st.observe_room(
            &make_ship_room("passage", true, false, "ceres"),
            Some("Pilot"),
        );
        let to_bridge = st
            .next_quest_hop()
            .expect("expected Bridge route while docked");
        assert_eq!(to_bridge.next_room_entry, "bridge");

        // 3. Settled on a surface landing pad -> routes to Vehicle Bay / Hold
        st.observe_room(
            &make_ship_room("passage", false, true, "commons"),
            Some("Pilot"),
        );
        assert!(st.settled_pad());
        let to_bay = st
            .next_quest_hop()
            .expect("expected Bay route on settled pad");
        assert_eq!(to_bay.next_room_entry, "hold");

        // 4. Mining complete while still on surface pad -> routes to Bridge (lift off & return to Ceres)
        st.observe_quest_mining_status(Some(QuestSurfaceMiningStatus {
            quest_title: "Scan, Sound, and Mine a Landing Site".into(),
            target_ore_slug: "carbon_ore".into(),
            recommended_family: "carbon".into(),
            scan_done: true,
            sound_done: true,
            mine_done: true,
        }));
        let lift_off = st.next_quest_hop().expect("expected Bridge lift-off route");
        assert_eq!(lift_off.next_room_entry, "bridge");

        // 5. Docked back at Ceres Freeport -> routes to Airlock (+ship/leave to turn in to Rosalind)
        st.observe_room(
            &make_ship_room("airlock", true, false, "ceres"),
            Some("Pilot"),
        );
        let ashore = st
            .next_quest_hop()
            .expect("expected Cycle Ashore at Ceres airlock");
        assert_eq!(ashore.next_exit_command, "+ship/leave");
    }

    #[test]
    fn trade_drawer_advances_step2_past_loaded_provisions_and_auto_opens_sell_tab_for_loaded_delivery()
     {
        let mut st = PortWalkState::new();
        let now = Instant::now();
        st.observe_port_trade(
            &json!({
                "enabled": true,
                "venue": "ceres",
                "port_name": "Ceres Freeport",
                "stowage": {
                    "cargo_kg": 0,
                    "max_cargo_kg": 100000,
                    "reaction_kg": 0,
                    "max_reaction_kg": 80000,
                    "stores_days": 10,
                    "stores_limit_days": 120
                },
                "asks": [
                    { "listing_id": 11, "good_id": "tacsim:provisions", "good_name": "Provisions", "vendor_name": "Chandlery", "quantity": 25, "unit_price_minor": 1000 },
                    { "listing_id": 12, "good_id": "tacsim:propellant", "good_name": "Propellant", "vendor_name": "Exchange", "quantity": 40, "unit_price_minor": 1800 }
                ],
                "bids": []
            }),
            now,
        );
        st.observe_quest_trade_needs(vec![
            QuestTradeNeed {
                quest_title: "Stock the Locker and Tanks".into(),
                checkpoint_title: "Buy 10 provisions".into(),
                commodity_slug: "provisions".into(),
                target_quantity: 10,
                wants_buy: true,
                wants_sell: false,
            },
            QuestTradeNeed {
                quest_title: "Stock the Locker and Tanks".into(),
                checkpoint_title: "Buy 5 propellant".into(),
                commodity_slug: "propellant".into(),
                target_quantity: 5,
                wants_buy: true,
                wants_sell: false,
            },
        ]);

        let eff = st
            .effective_quest_trade_need()
            .expect("should advance to propellant");
        assert_eq!(eff.commodity_slug, "propellant");
        assert_eq!(eff.target_quantity, 5);

        let mut queries = Vec::new();
        st.open_drawer(PortDrawerKind::Trade, &[], &mut queries, now);
        let jump = st.handle_key(Key::Char('g'), false, false, &[], now);
        assert!(jump.consumed);
        assert_eq!(st.trade_subtab, 0);
        assert_eq!(st.asks[st.cursor_row].listing_id, 12);
        assert_eq!(st.active_trade_qty_setting(), 5);

        // Step 3 metals loaded in hold -> narrows to SELL and auto-opens Sell Bids tab (#22)
        st.observe_port_trade(
            &json!({
                "enabled": true,
                "venue": "stickney",
                "port_name": "Stickney Yards",
                "stowage": {
                    "cargo_kg": 5000,
                    "max_cargo_kg": 100000,
                    "reaction_kg": 10000,
                    "max_reaction_kg": 80000,
                    "stores_days": 10,
                    "stores_limit_days": 120
                },
                "cargo": [
                    { "good": "tacsim:metals", "label": "Metals", "quantity": 5 }
                ],
                "asks": [
                    { "listing_id": 21, "good_id": "tacsim:metals", "good_name": "Metals", "vendor_name": "Vendor", "quantity": 10, "unit_price_minor": 10000 }
                ],
                "bids": [
                    { "listing_id": 22, "good_id": "tacsim:metals", "good_name": "Metals", "vendor_name": "Kasei", "quantity_wanted": 20, "unit_price_minor": 14500, "funded": true }
                ]
            }),
            now,
        );
        st.observe_quest_trade_needs(vec![QuestTradeNeed {
            quest_title: "Metals Down the Corridor".into(),
            checkpoint_title: "Deliver 5t metals".into(),
            commodity_slug: "metals".into(),
            target_quantity: 5,
            wants_buy: true,
            wants_sell: true,
        }]);
        let eff3 = st
            .effective_quest_trade_need()
            .expect("should narrow to SELL");
        assert!(!eff3.wants_buy);
        assert!(eff3.wants_sell);

        st.open_drawer(PortDrawerKind::Trade, &[], &mut queries, now);
        assert_eq!(st.trade_subtab, 1, "expected Sell Bids tab auto-selected");
        assert_eq!(st.bids[st.cursor_row].listing_id, 22);

        let backend = ratatui::backend::TestBackend::new(96, 24);
        let mut term = ratatui::Terminal::new(backend).expect("test terminal");
        term.draw(|f| st.draw_drawer_box(f, f.area(), true))
            .expect("draw sell trade drawer");
        let rendered: String = term
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            rendered.contains("[QUEST] SELL 5 x metals (g:jump+qty)"),
            "{rendered}"
        );
        assert!(rendered.contains("[Q] #22"), "{rendered}");
    }

    #[test]
    fn to_feed_value_highlights_ship_compartment_and_cross_port_berth_as_quest_target() {
        let mut st = PortWalkState::new();
        let make_room = |id: &str, name: &str, mode: Option<&str>, port: Option<&str>| {
            let mut ambient = std::collections::BTreeMap::new();
            if mode == Some("interior") {
                ambient.insert(
                    "tacsim".into(),
                    json!({
                        "mode": "interior",
                        "compartment": id,
                        "docked": port.is_some(),
                        "settled_pad": false,
                        "port": port.unwrap_or("")
                    }),
                );
            } else {
                ambient.insert(
                    "atlas".into(),
                    json!({
                        "pack": "neighborhood.ceres_freeport",
                        "entry": id,
                        "surface": "station"
                    }),
                );
            }
            RoomView {
                id: id.into(),
                name: name.into(),
                desc: mush_proto::RichText::plain("Deck."),
                exits: Vec::new(),
                contents: Vec::new(),
                zone: None,
                ambient,
            }
        };

        let find_quest_target_id = |feed: &Value| -> Option<String> {
            feed.get("rooms")?
                .as_array()?
                .iter()
                .find(|r| r.get("quest_target").and_then(Value::as_bool) == Some(true))
                .and_then(|r| r.get("id").and_then(Value::as_str))
                .map(str::to_string)
        };

        // 1. Ashore at Ceres with off-station Stickney target -> highlights Ceres Berth Gallery in 2.5D feed
        st.observe_room(
            &make_room("room.c03_arrivals", "Port Watch Arrivals", None, None),
            Some("Pilot"),
        );
        st.observe_quest_objective_room(Some(("room.p07_receiving".into(), false)));
        let ashore_feed = st.to_feed_value();
        assert_eq!(
            find_quest_target_id(&ashore_feed).as_deref(),
            Some("room.c01_berth_gallery")
        );

        // 2. Aboard ship docked at Ceres with destination Stickney -> highlights Bridge compartment in 2.5D feed
        st.observe_room(
            &make_room("airlock", "Airlock", Some("interior"), Some("ceres")),
            Some("Pilot"),
        );
        st.observe_quest_ship_target(Some("bridge"));
        let bridge_feed = st.to_feed_value();
        assert_eq!(
            find_quest_target_id(&bridge_feed).as_deref(),
            Some("bridge")
        );

        // 3. Docked at Stickney (objective room's port) -> highlights Airlock compartment to cycle ashore
        st.observe_room(
            &make_room("passage", "Passage", Some("interior"), Some("stickney")),
            Some("Pilot"),
        );
        st.observe_quest_ship_target(Some("airlock"));
        let airlock_feed = st.to_feed_value();
        assert_eq!(
            find_quest_target_id(&airlock_feed).as_deref(),
            Some("airlock")
        );
    }

    #[test]
    fn industry_drawer_shows_quest_history_banner_highlights_row_and_h_hotkey_runs_trend() {
        let mut st = PortWalkState::new();
        let now = Instant::now();
        st.observe_quest_trade_needs(vec![QuestTradeNeed {
            quest_title: "Metals Down the Corridor".into(),
            checkpoint_title: "Deliver 5t metals".into(),
            commodity_slug: "metals".into(),
            target_quantity: 5,
            wants_buy: true,
            wants_sell: true,
        }]);
        let mut queries = Vec::new();
        st.open_drawer(PortDrawerKind::Industry, &[], &mut queries, now);

        let backend = ratatui::backend::TestBackend::new(96, 18);
        let mut term = ratatui::Terminal::new(backend).expect("test terminal");
        term.draw(|f| st.draw_drawer_box(f, f.area(), true))
            .expect("draw industry drawer");
        let rendered: String = term
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            rendered.contains("[QUEST] 24w Trend: METALS (h: +market/history tacsim:metals)"),
            "{rendered}"
        );
        assert!(rendered.contains("[Q] 24-Week Metals Trend"), "{rendered}");

        let out = st.handle_key(Key::Char('h'), false, false, &[], now);
        assert_eq!(out.commands, vec!["+market/history tacsim:metals"]);

        // Step 7 mined carbon_ore uses exact tacsim:carbon_ore history command
        st.observe_quest_trade_needs(vec![QuestTradeNeed {
            quest_title: "Deliver Your Mined Ore to Market".into(),
            checkpoint_title: "Sell 1t carbon ore".into(),
            commodity_slug: "carbon_ore".into(),
            target_quantity: 1,
            wants_buy: false,
            wants_sell: true,
        }]);
        let out7 = st.handle_key(Key::Char('h'), false, false, &[], now);
        assert_eq!(out7.commands, vec!["+market/history tacsim:carbon_ore"]);
    }

    #[test]
    fn jobs_drawer_shows_quest_haul_and_survey_cross_links_with_f_hotkey() {
        let mut st = PortWalkState::new();
        let now = Instant::now();
        st.observe_quest_guidance(
            None,
            Some(QuestDestinationHint {
                quest_title: "Metals Down the Corridor".into(),
                port_id: "stickney".into(),
                port_label: "Stickney Yards".into(),
            }),
        );
        let mut queries = Vec::new();
        st.open_drawer(PortDrawerKind::Jobs, &[], &mut queries, now);

        let backend = ratatui::backend::TestBackend::new(96, 18);
        let mut term = ratatui::Terminal::new(backend).expect("test terminal");
        term.draw(|f| st.draw_drawer_box(f, f.area(), true))
            .expect("draw jobs drawer");
        let rendered: String = term
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            rendered.contains("[QUEST] Corridor Haul -> Stickney Yards (f: +market/jobs haul)"),
            "{rendered}"
        );
        assert!(rendered.contains("[Q] Haulage Contracts"), "{rendered}");

        let out_haul = st.handle_key(Key::Char('f'), false, false, &[], now);
        assert_eq!(out_haul.commands, vec!["+market/jobs haul"]);

        // Step 6 surface mining switches cross-link to Survey Commissions
        st.observe_quest_mining_status(Some(QuestSurfaceMiningStatus {
            quest_title: "Scan, Sound, and Mine a Landing Site".into(),
            target_ore_slug: "carbon_ore".into(),
            recommended_family: "carbon".into(),
            scan_done: false,
            sound_done: false,
            mine_done: false,
        }));
        term.draw(|f| st.draw_drawer_box(f, f.area(), true))
            .expect("draw jobs drawer mining");
        let rendered2: String = term
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            rendered2.contains("[QUEST] Surface Surveys (f: +market/jobs survey)"),
            "{rendered2}"
        );
        assert!(rendered2.contains("[Q] Survey Commissions"), "{rendered2}");

        let out_survey = st.handle_key(Key::Char('f'), false, false, &[], now);
        assert_eq!(out_survey.commands, vec!["+market/jobs survey"]);
    }

    #[test]
    fn port_walk_prioritizes_turn_in_over_available_marker_for_multi_quest_npc() {
        let mut st = PortWalkState::new();
        st.observe_quest_markers(&[
            QuestMarker {
                npc_id: "#18".into(),
                npc_name: "Ilse Oduya-Brandt".into(),
                marker: "in_progress".into(),
                symbol: "?".into(),
                quest_key: "side.in_progress".into(),
                quest_title: "In-Progress Favor".into(),
                category: "optional".into(),
            },
            QuestMarker {
                npc_id: "#18".into(),
                npc_name: "Ilse Oduya-Brandt".into(),
                marker: "available".into(),
                symbol: "!".into(),
                quest_key: "side.offer".into(),
                quest_title: "One-Shot Audit".into(),
                category: "optional".into(),
            },
            QuestMarker {
                npc_id: "#18".into(),
                npc_name: "Ilse Oduya-Brandt".into(),
                marker: "turn_in".into(),
                symbol: "?".into(),
                quest_key: "tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll".into(),
                quest_title: "First Papers at the Registry".into(),
                category: "guided".into(),
            },
        ]);

        // Both find_quest_marker and selected_npc_quest_marker select turn_in '?' over available '!' and in_progress '?'
        let best = st
            .find_quest_marker("#18", "Ilse Oduya-Brandt")
            .expect("best marker");
        assert_eq!(best.marker, "turn_in");
        assert_eq!(
            st.format_npc_interact_label("#18", "Ilse Oduya-Brandt", true),
            "[?] Ilse Oduya-Brandt — Turn In Quest (+2 more)"
        );

        st.selected_npc_name = "Ilse Oduya-Brandt".into();
        assert_eq!(
            st.selected_npc_quest_marker().map(|m| m.marker.as_str()),
            Some("turn_in")
        );
        assert_eq!(st.selected_npc_quest_count(), 3);

        st.drawer = Some(PortDrawerKind::Npc);
        let backend = ratatui::backend::TestBackend::new(100, 20);
        let mut terminal = ratatui::Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|f| st.draw_drawer_box(f, f.area(), true))
            .expect("render npc drawer");
        let rendered: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            rendered.contains("[?] Turn In Quest: First Papers at the Registry (+2 more)"),
            "{rendered}"
        );
    }

    #[test]
    fn port_trade_drawer_sorts_quest_matching_asks_and_bids_first() {
        let mut st = PortWalkState::new();
        let now = Instant::now();
        let trade_frame = json!({
            "enabled": true,
            "venue": "ceres",
            "listings": [
                { "id": 10, "side": "ask", "good": "tacsim:silicates", "good_label": "Silicates", "available_qty": 20, "unit_price_minor": 500, "funded": true },
                { "id": 11, "side": "ask", "good": "tacsim:metals", "good_label": "Metals", "available_qty": 15, "unit_price_minor": 1500, "funded": true },
                { "id": 20, "side": "bid", "good": "tacsim:water_ice", "good_label": "Water Ice", "available_qty": 30, "unit_price_minor": 800, "funded": true },
                { "id": 21, "side": "bid", "good": "tacsim:metals", "good_label": "Metals", "available_qty": 12, "unit_price_minor": 2200, "funded": true }
            ]
        });

        // 1. No active quest -> raw server order preserved
        let _ = st.observe_port_trade(&trade_frame, now);
        assert_eq!(st.asks[0].listing_id, 10);
        assert_eq!(st.bids[0].listing_id, 20);

        // 2. Sync Step 3 metals buy/sell need -> matching metals ask (#11) and bid (#21) sort first
        st.observe_quest_trade_needs(vec![QuestTradeNeed {
            quest_title: "Metals Down the Corridor".into(),
            checkpoint_title: "Deliver 5t metals".into(),
            commodity_slug: "metals".into(),
            target_quantity: 5,
            wants_buy: true,
            wants_sell: true,
        }]);
        assert_eq!(st.asks[0].listing_id, 11);
        assert_eq!(st.bids[0].listing_id, 21);

        // 3. Re-observing port.trade keeps metals sorted first automatically
        let _ = st.observe_port_trade(&trade_frame, now);
        assert_eq!(st.asks[0].listing_id, 11);
        assert_eq!(st.bids[0].listing_id, 21);
    }

    #[test]
    fn sailing_drawer_sorts_quest_dest_first_marks_here_and_supports_step6_mining() {
        let mut st = PortWalkState::new();
        let now = Instant::now();
        let _ = st.observe_port_trade(&json!({ "enabled": true, "venue": "ceres" }), now);

        // 1. Active Stickney quest destination floats Stickney Yards to visual row 0 and marks Ceres [HERE]
        st.observe_quest_guidance(
            None,
            Some(QuestDestinationHint {
                quest_title: "Metals Down the Corridor".into(),
                port_id: "stickney".into(),
                port_label: "Stickney Yards".into(),
            }),
        );
        let mut queries = Vec::new();
        st.open_drawer(PortDrawerKind::Sailing, &[], &mut queries, now);
        assert_eq!(CORRIDORS[st.corridor_idx].0, "stickney");
        let ordered = st.ordered_sailing_corridor_indices();
        assert_eq!(CORRIDORS[ordered[0]].0, "stickney");

        let render_sailing = |state: &PortWalkState| -> String {
            let backend = ratatui::backend::TestBackend::new(100, 22);
            let mut terminal = ratatui::Terminal::new(backend).expect("test terminal");
            terminal
                .draw(|f| state.draw_drawer_box(f, f.area(), true))
                .expect("render sailing drawer");
            let buf = terminal.backend().buffer();
            let mut out = String::new();
            for y in 0..buf.area.height {
                for x in 0..buf.area.width {
                    out.push_str(buf[(x, y)].symbol());
                }
                out.push('\n');
            }
            out
        };

        let rendered_step3 = render_sailing(&st);
        let pos_stickney = rendered_step3
            .find("[Q] Stickney Yards")
            .expect("expected [Q] Stickney Yards");
        let pos_ceres_here = rendered_step3
            .find("Ceres Freeport")
            .expect("expected Ceres Freeport");
        assert!(
            pos_stickney < pos_ceres_here,
            "expected [Q] Stickney Yards ahead of Ceres Freeport:\n{rendered_step3}"
        );
        assert!(
            rendered_step3.contains("[HERE]"),
            "expected [HERE] tag on current port:\n{rendered_step3}"
        );

        // 2. Step 6 surface extraction (no port dest) floats Freeport Commons first, shows Step 6 banner, and binds 'c' -> +chart/ground
        st.observe_quest_guidance(None, None);
        st.observe_quest_mining_status(Some(QuestSurfaceMiningStatus {
            quest_title: "Scan, Sound, and Mine a Landing Site".into(),
            target_ore_slug: "carbon_ore".into(),
            recommended_family: "carbon".into(),
            scan_done: false,
            sound_done: false,
            mine_done: false,
        }));
        st.open_drawer(PortDrawerKind::Sailing, &[], &mut queries, now);
        assert_eq!(CORRIDORS[st.corridor_idx].0, "commons");
        let ordered_step6 = st.ordered_sailing_corridor_indices();
        assert_eq!(CORRIDORS[ordered_step6[0]].0, "commons");
        let rendered_step6 = render_sailing(&st);
        assert!(
            rendered_step6.contains("[QUEST] Surface Extraction: carbon ore"),
            "expected Step 6 surface extraction banner:\n{rendered_step6}"
        );
        assert!(
            rendered_step6.contains("[Q] Freeport Commons"),
            "expected [Q] Freeport Commons row:\n{rendered_step6}"
        );

        let key_out = st.handle_drawer_key(Key::Char('c'), &[], now);
        assert!(
            key_out.commands.iter().any(|c| c == "+chart/ground"),
            "expected 'c' in Sailing drawer to emit +chart/ground, got {:?}",
            key_out.commands
        );
    }

    #[test]
    fn ship_top_bar_shows_quest_target_deck_when_in_different_compartment() {
        let mut st = PortWalkState::new();
        st.aboard_ship = true;
        st.active_location_key = "airlock".into();
        st.room_title = "Airlock".into();

        // Target Bridge while in Airlock -> shows [QUEST -> Deck: Bridge]
        st.observe_quest_ship_target(Some("bridge"));
        let (top_bridge, _) = st.hud_lines("kitty");
        assert!(
            top_bridge.contains("[QUEST -> Deck: Bridge]"),
            "expected Bridge deck cue in ship top bar:\n{top_bridge}"
        );

        // Target Vehicle Bay while in Airlock -> shows [QUEST -> Deck: Vehicle Bay (+tv/bay in)]
        st.observe_quest_ship_target(Some("bay"));
        let (top_bay, _) = st.hud_lines("kitty");
        assert!(
            top_bay.contains("[QUEST -> Deck: Vehicle Bay (+tv/bay in)]"),
            "expected Vehicle Bay deck cue in ship top bar:\n{top_bay}"
        );

        // Once inside target compartment -> suppresses redundant Go to Deck cue
        st.active_location_key = "bay".into();
        let (top_in_bay, _) = st.hud_lines("kitty");
        assert!(
            !top_in_bay.contains("[QUEST -> Deck:"),
            "expected no redundant Deck cue once inside bay:\n{top_in_bay}"
        );
    }

    #[test]
    fn nearby_interactables_sort_quest_markers_first_and_surface_vehicle_bay_actions() {
        let mut st = PortWalkState::new();
        let mut ambient = std::collections::BTreeMap::new();
        ambient.insert(
            "atlas".into(),
            json!({ "pack": "neighborhood.ceres_freeport", "entry": "room.c03_arrivals" }),
        );
        ambient.insert("occupants".into(), json!({ "npcs": ["#99", "#11", "#18"] }));
        let arrivals = RoomView {
            id: "#10".into(),
            name: "Port Watch Arrivals".into(),
            desc: mush_proto::RichText::plain("Arrivals."),
            exits: Vec::new(),
            contents: vec![
                mush_proto::ObjRef {
                    id: "#99".into(),
                    name: "Unmarked Clerk".into(),
                },
                mush_proto::ObjRef {
                    id: "#11".into(),
                    name: "Maren Halvorsen".into(),
                },
                mush_proto::ObjRef {
                    id: "#18".into(),
                    name: "Ilse Oduya-Brandt".into(),
                },
            ],
            zone: None,
            ambient,
        };
        st.observe_room(&arrivals, Some("Pilot"));
        st.observe_quest_markers(&[
            QuestMarker {
                npc_id: "#11".into(),
                npc_name: "Maren Halvorsen".into(),
                marker: "available".into(),
                symbol: "!".into(),
                quest_key: "q.avail".into(),
                quest_title: "First Papers".into(),
                category: "guided".into(),
            },
            QuestMarker {
                npc_id: "#18".into(),
                npc_name: "Ilse Oduya-Brandt".into(),
                marker: "turn_in".into(),
                symbol: "?".into(),
                quest_key: "q.turnin".into(),
                quest_title: "Ready Turn-In".into(),
                category: "guided".into(),
            },
        ]);
        assert!(
            st.interactables[0]
                .label
                .starts_with("[?] Ilse Oduya-Brandt"),
            "expected turn_in NPC at row 0, got {:?}",
            st.interactables
                .iter()
                .map(|i| &i.label)
                .collect::<Vec<_>>()
        );
        assert!(
            st.interactables[1].label.starts_with("[!] Maren Halvorsen"),
            "expected available NPC at row 1, got {:?}",
            st.interactables
                .iter()
                .map(|i| &i.label)
                .collect::<Vec<_>>()
        );

        // Aboard Dray-P in Cargo Hold with Step 6 mining active -> [Q] Enter Vehicle Bay floats to row 0
        let mut hold_amb = std::collections::BTreeMap::new();
        hold_amb.insert(
            "tacsim".into(),
            json!({
                "mode": "interior",
                "room": "hold",
                "hull_name": "Starling",
                "unit_type": "tacsim.unit.dray_p"
            }),
        );
        let hold_view = RoomView {
            id: "#54".into(),
            name: "M/V Starling - Cargo Hold".into(),
            desc: mush_proto::RichText::plain("Hold."),
            exits: Vec::new(),
            contents: Vec::new(),
            zone: None,
            ambient: hold_amb,
        };
        st.observe_room(&hold_view, Some("Pilot"));
        if !st.ship_rooms.iter().any(|r| r.entry_id == "bay") {
            st.ship_rooms.push(RoomGeom {
                entry_id: "bay".into(),
                name: "Vehicle Bay".into(),
                kind: "bay",
                floor: 0,
                min_x: -4.0,
                max_x: 4.0,
                min_y: -4.0,
                max_y: 4.0,
            });
            st.observe_room(&hold_view, Some("Pilot"));
        }
        st.observe_quest_mining_status(Some(QuestSurfaceMiningStatus {
            quest_title: "Scan, Sound, and Mine a Landing Site".into(),
            target_ore_slug: "carbon_ore".into(),
            recommended_family: "carbon".into(),
            scan_done: false,
            sound_done: false,
            mine_done: false,
        }));
        assert_eq!(
            st.interactables[0].label,
            "[Q] Enter Vehicle Bay (+tv/bay in)"
        );
        assert_eq!(
            st.interactables[0].action,
            InteractAction::Command("+tv/bay in")
        );

        // Inside Vehicle Bay -> surfaces +tv and +tv/bay out interactables
        let mut bay_amb = std::collections::BTreeMap::new();
        bay_amb.insert(
            "tacsim".into(),
            json!({
                "mode": "interior",
                "room": "bay",
                "hull_name": "Starling",
                "unit_type": "tacsim.unit.dray_p"
            }),
        );
        let bay_view = RoomView {
            id: "#55".into(),
            name: "M/V Starling - Vehicle Bay".into(),
            desc: mush_proto::RichText::plain("Bay."),
            exits: Vec::new(),
            contents: Vec::new(),
            zone: None,
            ambient: bay_amb,
        };
        if !st.ship_rooms.iter().any(|r| r.entry_id == "bay") {
            st.ship_rooms.push(RoomGeom {
                entry_id: "bay".into(),
                name: "Vehicle Bay".into(),
                kind: "bay",
                floor: 0,
                min_x: -4.0,
                max_x: 4.0,
                min_y: -4.0,
                max_y: 4.0,
            });
        }
        st.observe_room(&bay_view, Some("Pilot"));
        assert_eq!(st.active_location_key, "bay");
        assert!(
            st.interactables
                .iter()
                .any(|t| t.action == InteractAction::Command("+tv")),
            "expected +tv interactable in bay"
        );
        assert!(
            st.interactables
                .iter()
                .any(|t| t.action == InteractAction::Command("+tv/bay out")),
            "expected +tv/bay out interactable in bay"
        );
    }

    #[test]
    fn vehicle_bay_hud_and_interactables_surface_live_ore_hold_and_post_mine_return_cue() {
        let mut st = PortWalkState::new();
        st.stowage_loaded = true;
        st.cargo_lots.push(CargoLotRow {
            good_id: "tacsim:carbon_ore".into(),
            good_name: "Carbon Ore".into(),
            quantity: 2,
        });
        st.observe_quest_objective_room(Some(("room.c07_exchange".into(), true)));
        st.observe_quest_mining_status(Some(QuestSurfaceMiningStatus {
            quest_title: "Scan, Sound, and Mine a Landing Site".into(),
            target_ore_slug: "carbon_ore".into(),
            recommended_family: "carbon".into(),
            scan_done: true,
            sound_done: true,
            mine_done: true,
        }));

        let mut bay_amb = std::collections::BTreeMap::new();
        bay_amb.insert(
            "tacsim".into(),
            json!({
                "mode": "interior",
                "room": "bay",
                "hull_name": "Starling",
                "unit_type": "tacsim.unit.dray_p"
            }),
        );
        let bay_view = RoomView {
            id: "#55".into(),
            name: "M/V Starling - Vehicle Bay".into(),
            desc: mush_proto::RichText::plain("Bay."),
            exits: Vec::new(),
            contents: Vec::new(),
            zone: None,
            ambient: bay_amb,
        };
        if !st.ship_rooms.iter().any(|r| r.entry_id == "bay") {
            st.ship_rooms.push(RoomGeom {
                entry_id: "bay".into(),
                name: "Vehicle Bay".into(),
                kind: "bay",
                floor: 0,
                min_x: -4.0,
                max_x: 4.0,
                min_y: -4.0,
                max_y: 4.0,
            });
        }
        st.observe_room(&bay_view, Some("Pilot"));
        assert_eq!(st.active_location_key, "bay");
        // Re-apply stowage after observe_room since empty tacsim stowage keeps cargo_lots
        st.stowage_loaded = true;
        if st.cargo_lots.is_empty() {
            st.cargo_lots.push(CargoLotRow {
                good_id: "tacsim:carbon_ore".into(),
                good_name: "Carbon Ore".into(),
                quantity: 2,
            });
        }

        let (top, _) = st.hud_lines("kitty");
        assert!(
            top.contains("Hold: 2t carbon ore"),
            "expected live ore hold in Vehicle Bay HUD:\n{top}"
        );
        assert!(
            top.contains("Return: Ceres Freeport (+ship/plan ceres)"),
            "expected post-mine return cue in Vehicle Bay HUD:\n{top}"
        );
        assert_eq!(
            st.interactables[0].label,
            "[Q] Recover Rig (+tv/1 recover go)"
        );
        assert_eq!(
            st.interactables[0].action,
            InteractAction::Command("+tv/1 recover go")
        );
    }

    #[test]
    fn port_service_and_npc_drawers_display_guided_step_badges() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut st = PortWalkState::new();
        st.observe_quest_step_badge(" [Step 3/7]".to_string());
        st.observe_quest_guidance(
            Some(QuestTradeNeed {
                quest_title: "Metals Down the Corridor".into(),
                checkpoint_title: "Buy 5t metals".into(),
                commodity_slug: "metals".into(),
                target_quantity: 5,
                wants_buy: true,
                wants_sell: false,
            }),
            Some(QuestDestinationHint {
                quest_title: "Metals Down the Corridor".into(),
                port_id: "stickney".into(),
                port_label: "Stickney Yards".into(),
            }),
        );
        st.observe_quest_coop_action(Some(QuestSuggestedAction {
            label: "Enroll in Co-op".into(),
            command: "+coop/enroll Starling".into(),
        }));
        st.observe_quest_markers(&[QuestMarker {
            npc_id: "#11".into(),
            npc_name: "Maren Halvorsen".into(),
            marker: "available".into(),
            symbol: "!".into(),
            category: "guided".into(),
            quest_key: "tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll".into(),
            quest_title: "First Papers at the Registry".into(),
        }]);
        st.selected_npc_name = "Maren Halvorsen".into();

        let render_drawer_text = |st: &mut PortWalkState, kind: PortDrawerKind| -> String {
            st.drawer = Some(kind);
            let backend = TestBackend::new(120, 24);
            let mut term = Terminal::new(backend).expect("terminal");
            term.draw(|f| st.draw_drawer_box(f, f.area(), true))
                .expect("draw");
            let buf = term.backend().buffer();
            let mut out = String::new();
            for y in 0..buf.area.height {
                for x in 0..buf.area.width {
                    out.push_str(buf[(x, y)].symbol());
                }
                out.push('\n');
            }
            out
        };

        let trade_txt = render_drawer_text(&mut st, PortDrawerKind::Trade);
        assert!(
            trade_txt.contains("[Step 3/7]"),
            "trade drawer missing step badge:\n{trade_txt}"
        );

        let sail_txt = render_drawer_text(&mut st, PortDrawerKind::Sailing);
        assert!(
            sail_txt.contains("[Step 3/7]"),
            "sailing drawer missing step badge:\n{sail_txt}"
        );

        let jobs_txt = render_drawer_text(&mut st, PortDrawerKind::Jobs);
        assert!(
            jobs_txt.contains("[Step 3/7]"),
            "jobs drawer missing step badge:\n{jobs_txt}"
        );

        let ind_txt = render_drawer_text(&mut st, PortDrawerKind::Industry);
        assert!(
            ind_txt.contains("[Step 3/7]"),
            "industry drawer missing step badge:\n{ind_txt}"
        );

        let coop_txt = render_drawer_text(&mut st, PortDrawerKind::Coop);
        assert!(
            coop_txt.contains("[Step 3/7]"),
            "coop drawer missing step badge:\n{coop_txt}"
        );

        let npc_txt = render_drawer_text(&mut st, PortDrawerKind::Npc);
        assert!(
            npc_txt.contains("[Step 1/7]"),
            "npc drawer missing step badge:\n{npc_txt}"
        );
    }

    #[test]
    fn trade_drawer_shows_cross_port_haul_guard_and_d_jumps_to_sailing() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut st = PortWalkState::new();
        st.current_venue = Some("ceres".into());
        st.stowage_loaded = true;
        st.stowage.reaction_kg = 12_000;
        st.stowage.stores_days = 0;
        st.cargo_lots.push(CargoLotRow {
            good_id: "tacsim:carbon_ore".into(),
            good_name: "Carbon Ore".into(),
            quantity: 2,
        });
        st.observe_quest_guidance(
            Some(QuestTradeNeed {
                quest_title: "Deliver Your Mined Ore to Market".into(),
                checkpoint_title: "Sell 1t carbon_ore at Stickney Yards".into(),
                commodity_slug: "carbon_ore".into(),
                target_quantity: 1,
                wants_buy: false,
                wants_sell: true,
            }),
            Some(QuestDestinationHint {
                quest_title: "Deliver Your Mined Ore to Market".into(),
                port_id: "stickney".into(),
                port_label: "Stickney Yards".into(),
            }),
        );

        let render_trade = |st: &mut PortWalkState| -> String {
            st.drawer = Some(PortDrawerKind::Trade);
            let backend = TestBackend::new(120, 24);
            let mut term = Terminal::new(backend).expect("terminal");
            term.draw(|f| st.draw_drawer_box(f, f.area(), true))
                .expect("draw");
            let buf = term.backend().buffer();
            (0..buf.area.height)
                .map(|y| {
                    (0..buf.area.width)
                        .map(|x| buf[(x, y)].symbol())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let ceres_txt = render_trade(&mut st);
        assert!(
            ceres_txt.contains(
                "[HAUL] -> Stickney Yards (d:Sail | Top up Stores first | do not sell here)"
            ),
            "expected cross-port haul guard with missing stores reminder:\n{ceres_txt}"
        );

        // Pressing `d` switches to Sailing drawer with Stickney Yards selected
        let _ = st.handle_drawer_key(Key::Char('d'), &[], Instant::now());
        assert_eq!(st.drawer, Some(PortDrawerKind::Sailing));
        assert_eq!(CORRIDORS[st.corridor_idx].0, "stickney");

        // Once docked at Stickney Yards, haul guard clears
        st.current_venue = Some("stickney".into());
        let stickney_txt = render_trade(&mut st);
        assert!(
            !stickney_txt.contains("do not sell here"),
            "haul guard should clear at destination port:\n{stickney_txt}"
        );
    }

    #[test]
    fn trade_drawer_guides_step5_refit_hold_clearing_and_coop_handoff() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut st = PortWalkState::new();
        st.current_venue = Some("ceres".into());
        st.stowage_loaded = true;
        st.stowage.cargo_kg = 2000;
        st.stowage.reaction_kg = 15_000;
        st.stowage.stores_days = 8;
        st.cargo_lots.push(CargoLotRow {
            good_id: "tacsim:metals".into(),
            good_name: "Metals".into(),
            quantity: 2,
        });
        st.bids.push(BidRow {
            listing_id: 901,
            good_id: "tacsim:silicates".into(),
            good_name: "Silicates".into(),
            vendor_name: "Ceres Works".into(),
            quantity_wanted: 20,
            unit_price_minor: 1200,
            funded: true,
        });
        st.bids.push(BidRow {
            listing_id: 902,
            good_id: "tacsim:provisions".into(),
            good_name: "Provisions".into(),
            vendor_name: "Co-op Chandlery".into(),
            quantity_wanted: 30,
            unit_price_minor: 400,
            funded: true,
        });
        st.bids.push(BidRow {
            listing_id: 903,
            good_id: "tacsim:metals".into(),
            good_name: "Metals".into(),
            vendor_name: "Ceres Exchange".into(),
            quantity_wanted: 25,
            unit_price_minor: 900,
            funded: true,
        });
        st.observe_quest_coop_action(Some(QuestSuggestedAction {
            label: "Refit Dray-P".into(),
            command: "+coop/refit prospector".into(),
        }));
        st.observe_quest_step_badge(" [Step 5/7]".into());

        let render_trade = |st: &mut PortWalkState| -> String {
            st.drawer = Some(PortDrawerKind::Trade);
            let backend = TestBackend::new(120, 24);
            let mut term = Terminal::new(backend).expect("terminal");
            term.draw(|f| st.draw_drawer_box(f, f.area(), true))
                .expect("draw");
            let buf = term.backend().buffer();
            (0..buf.area.height)
                .map(|y| {
                    (0..buf.area.width)
                        .map(|x| buf[(x, y)].symbol())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let blocked_txt = render_trade(&mut st);
        assert!(
            blocked_txt.contains("[REFIT PREP] Empty hold: Cargo 2.0t & Stores 8d (g:Bids | propellant OK) [Step 5/7]"),
            "expected Step 5 refit prep banner:\n{blocked_txt}"
        );

        // `g` switches to Bids subtab and jumps cursor onto the first refit-clearing bid (#902 Provisions)
        let _ = st.handle_drawer_key(Key::Char('g'), &[], Instant::now());
        assert_eq!(st.trade_subtab, 1);
        assert_eq!(st.cursor_row, 1);
        let bids_txt = render_trade(&mut st);
        assert!(
            bids_txt.contains("[CLEAR] #902"),
            "expected [CLEAR] tag on Provisions bid:\n{bids_txt}"
        );
        assert!(
            bids_txt.contains("[CLEAR] #903"),
            "expected [CLEAR] tag on stowed Metals bid:\n{bids_txt}"
        );
        assert!(
            !bids_txt.contains("[CLEAR] #901"),
            "unrelated Silicates bid must not get [CLEAR] tag:\n{bids_txt}"
        );

        // Clear cargo & provisions (propellant remains) -> banner flips to [REFIT HOLD CLEAR] and `c` jumps to Co-op Desk
        st.stowage.cargo_kg = 0;
        st.stowage.stores_days = 0;
        st.cargo_lots.clear();
        let clear_txt = render_trade(&mut st);
        assert!(
            clear_txt.contains(
                "[REFIT HOLD CLEAR] Ready -> c:Co-op Desk (+coop/refit prospector) [Step 5/7]"
            ),
            "expected clear hold banner:\n{clear_txt}"
        );
        let _ = st.handle_drawer_key(Key::Char('c'), &[], Instant::now());
        assert_eq!(st.drawer, Some(PortDrawerKind::Coop));
    }

    #[test]
    fn resolved_station_quest_target_matches_same_station_role_suffix_without_cross_port_leak() {
        let mut st = PortWalkState::new();
        // Rename the loaded chandlery room to a different slot number on Ceres (`room.c06_chandlery`)
        for r in &mut st.rooms {
            if r.entry_id == "room.c08_chandlery" {
                r.entry_id = "room.c06_chandlery".into();
            }
        }
        st.observe_quest_objective_room(Some(("room.c08_chandlery".into(), false)));
        assert_eq!(
            st.resolved_station_quest_target(),
            Some(("room.c06_chandlery".into(), false))
        );

        // Cross-port target (`room.p08_chandlery`) must NOT match Ceres chandlery; routes to Berth Gallery.
        st.observe_quest_objective_room(Some(("room.p08_chandlery".into(), false)));
        assert_eq!(
            st.resolved_station_quest_target(),
            Some(("room.c01_berth_gallery".into(), true))
        );
    }
}
