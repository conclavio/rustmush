//! WoW-style Quest Markers, Tracker, Quest Dialog & Completion Log for the SSH TUI.
//!
//! Consumes the `"tacsim.quests"` panel feed and `RoomView.ambient["quest_markers"]`.
//! Every mutation submits a server-signed `+quest/...` command after an explicit key
//! inside the modal overlay (never from a bare nav key on the main screen).

use mush_proto::RoomView;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use serde_json::Value;

use super::keys::Key;
use super::render::clean;

/// Panel feed id carrying structured quest state and room NPC markers.
pub const QUESTS_FEED: &str = "tacsim.quests";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestMarker {
    pub npc_id: String,
    pub npc_name: String,
    pub marker: String, // "turn_in" | "available" | "in_progress"
    pub symbol: String, // "!" | "?"
    pub quest_key: String,
    pub quest_title: String,
    pub category: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestCheckpoint {
    pub id: String,
    pub title: String,
    pub instructions: String,
    pub room_name: String,
    pub checked: bool,
    pub live_ready: bool,
    pub detail_text: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveQuest {
    pub key: String,
    pub title: String,
    pub category: String,
    pub introduction: String,
    pub reward_summary: String,
    pub giver_name: String,
    pub turn_in_name: String,
    pub allow_restart: bool,
    pub ready_to_finish: bool,
    pub next_step: String,
    pub next_room_name: String,
    pub can_check_now: bool,
    pub can_finish_now: bool,
    pub check_cmd: String,
    pub finish_cmd: String,
    pub abandon_cmd: String,
    pub checkpoints: Vec<QuestCheckpoint>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AvailableQuest {
    pub key: String,
    pub title: String,
    pub category: String,
    pub introduction: String,
    pub reward_summary: String,
    pub giver_name: String,
    pub turn_in_name: String,
    pub allow_restart: bool,
    pub accept_cmd: String,
    pub decline_cmd: String,
    pub checkpoints: Vec<QuestCheckpoint>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletedQuest {
    pub key: String,
    pub title: String,
    pub category: String,
    pub giver_name: String,
    pub turn_in_name: String,
    pub completion: String,
    pub reward_summary: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QuestsSnapshot {
    pub room_name: String,
    pub room_markers: Vec<QuestMarker>,
    pub active: Vec<ActiveQuest>,
    pub available: Vec<AvailableQuest>,
    pub completed: Vec<CompletedQuest>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestTradeNeed {
    pub quest_title: String,
    pub checkpoint_title: String,
    pub commodity_slug: String,
    pub target_quantity: i64,
    pub wants_buy: bool,
    pub wants_sell: bool,
}

impl QuestTradeNeed {
    #[must_use]
    pub fn matches_listing(&self, good_id: &str, good_name: &str, is_bid: bool) -> bool {
        if is_bid {
            if !self.wants_sell {
                return false;
            }
        } else if !self.wants_buy {
            return false;
        }
        let code = normalize_good_slug(good_id);
        if !code.is_empty() && code == self.commodity_slug {
            return true;
        }
        let name = normalize_good_slug(good_name);
        !name.is_empty() && name == self.commodity_slug
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestDestinationHint {
    pub quest_title: String,
    pub port_id: String,
    pub port_label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestSurfaceMiningStatus {
    pub quest_title: String,
    pub target_ore_slug: String,
    pub recommended_family: String,
    pub scan_done: bool,
    pub sound_done: bool,
    pub mine_done: bool,
}

const KNOWN_TRADE_COMMODITIES: [(&str, &[&str]); 10] = [
    (
        "industrial_parts",
        &["industrial_parts", "industrial parts"],
    ),
    ("carbon_ore", &["carbon_ore", "carbon ore"]),
    (
        "structural_alloys",
        &["structural_alloys", "structural alloys"],
    ),
    ("rare_earths", &["rare_earths", "rare earths"]),
    ("water_ice", &["water_ice", "water ice"]),
    ("provisions", &["provisions"]),
    ("propellant", &["propellant"]),
    ("volatiles", &["volatiles"]),
    ("silicates", &["silicates"]),
    ("metals", &["metals"]),
];

const KNOWN_DESTINATIONS: [(&str, &str, &[&str]); 13] = [
    (
        "stickney",
        "Stickney Yards",
        &[
            "+ship/plan stickney",
            "+ship/order depart stickney",
            "stickney yards",
            "stickney",
        ],
    ),
    (
        "ceres",
        "Ceres Freeport",
        &[
            "+ship/plan ceres",
            "+ship/order depart ceres",
            "ceres freeport",
            "back to ceres",
            "return to ceres",
        ],
    ),
    (
        "luna",
        "Aitken Highport",
        &[
            "+ship/plan luna",
            "+ship/order depart luna",
            "aitken highport",
        ],
    ),
    (
        "deimos",
        "Voltaire Anchorage",
        &[
            "+ship/plan deimos",
            "+ship/order depart deimos",
            "voltaire anchorage",
        ],
    ),
    (
        "titan",
        "Ligeia Terminal",
        &[
            "+ship/plan titan",
            "+ship/order depart titan",
            "ligeia terminal",
        ],
    ),
    (
        "isidis",
        "Isidis Concession",
        &[
            "+ship/plan isidis",
            "+ship/order depart isidis",
            "isidis concession",
        ],
    ),
    (
        "commons",
        "Freeport Commons",
        &["+ship/plan commons", "freeport commons"],
    ),
    ("ahuna", "Ahuna Flank", &["+ship/plan ahuna", "ahuna flank"]),
    (
        "occator",
        "Occator Rim",
        &["+ship/plan occator", "occator rim"],
    ),
    (
        "kerwan",
        "Kerwan Floor",
        &["+ship/plan kerwan", "kerwan floor"],
    ),
    (
        "juling",
        "Juling Shadow",
        &["+ship/plan juling", "juling shadow"],
    ),
    (
        "urvara",
        "Urvara Reach",
        &["+ship/plan urvara", "urvara reach"],
    ),
    (
        "rheasilvia",
        "Rheasilvia Commons",
        &["+ship/plan rheasilvia", "rheasilvia commons"],
    ),
];

#[must_use]
pub fn normalize_good_slug(raw: &str) -> String {
    let trimmed = raw.trim().to_ascii_lowercase();
    let tail = trimmed
        .rsplit_once(':')
        .map_or(trimmed.as_str(), |(_, rhs)| rhs);
    let mut out = String::with_capacity(tail.len());
    let mut prev_us = false;
    for ch in tail.chars() {
        let mapped = if ch == '-' || ch.is_ascii_whitespace() {
            '_'
        } else {
            ch
        };
        if mapped == '_' {
            if !prev_us {
                out.push('_');
            }
            prev_us = true;
        } else {
            out.push(mapped);
            prev_us = false;
        }
    }
    out
}

fn extract_leading_positive_i64(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let mut val: i64 = 0;
            while i < bytes.len() && bytes[i].is_ascii_digit() && val < 10_000 {
                val = val * 10 + i64::from(bytes[i] - b'0');
                i += 1;
            }
            if val > 0 {
                return Some(val);
            }
        } else {
            i += 1;
        }
    }
    None
}

impl QuestsSnapshot {
    fn ordered_active<'a>(&'a self, tracked_key: Option<&str>) -> Vec<&'a ActiveQuest> {
        let mut out = Vec::with_capacity(self.active.len());
        if let Some(tk) = tracked_key
            && let Some(found) = self.active.iter().find(|q| q.key == tk)
        {
            out.push(found);
        }
        for q in &self.active {
            if !out.iter().any(|existing| existing.key == q.key) {
                out.push(q);
            }
        }
        out
    }

    #[must_use]
    pub fn active_trade_need(&self, tracked_key: Option<&str>) -> Option<QuestTradeNeed> {
        for q in self.ordered_active(tracked_key) {
            for cp in &q.checkpoints {
                if cp.checked || cp.live_ready {
                    continue;
                }
                let id_low = cp.id.to_ascii_lowercase();
                let title_low = cp.title.to_ascii_lowercase();
                let instr_low = cp.instructions.to_ascii_lowercase();
                if id_low.starts_with("mine_")
                    || id_low.starts_with("scan_")
                    || id_low.starts_with("sound_")
                    || title_low.starts_with("mine ")
                {
                    continue;
                }
                let combined = format!("{id_low} {title_low} {instr_low}");
                let slug = KNOWN_TRADE_COMMODITIES
                    .iter()
                    .find(|(_, pats)| pats.iter().any(|p| combined.contains(p)))
                    .map(|(slug, _)| *slug)?;
                let wants_buy = id_low.starts_with("buy_")
                    || id_low.starts_with("deliver_")
                    || id_low.starts_with("return_")
                    || title_low.contains("buy ")
                    || title_low.contains("haul ")
                    || instr_low.contains("buy ");
                let wants_sell = id_low.starts_with("sell_")
                    || id_low.starts_with("deliver_")
                    || id_low.starts_with("return_")
                    || title_low.contains("sell ")
                    || title_low.contains("deliver ")
                    || title_low.contains("haul ")
                    || instr_low.contains("+market/sell");
                if !wants_buy && !wants_sell {
                    continue;
                }
                let target_quantity = extract_leading_positive_i64(&cp.title)
                    .or_else(|| extract_leading_positive_i64(&cp.instructions))
                    .unwrap_or(1);
                return Some(QuestTradeNeed {
                    quest_title: q.title.clone(),
                    checkpoint_title: cp.title.clone(),
                    commodity_slug: slug.to_string(),
                    target_quantity,
                    wants_buy,
                    wants_sell,
                });
            }
        }
        None
    }

    #[must_use]
    pub fn active_destination_port(
        &self,
        tracked_key: Option<&str>,
    ) -> Option<QuestDestinationHint> {
        for q in self.ordered_active(tracked_key) {
            for cp in &q.checkpoints {
                if cp.checked || cp.live_ready {
                    continue;
                }
                let combined = format!("{} {} {}", cp.instructions, cp.title, q.introduction)
                    .to_ascii_lowercase();
                for prefix in ["+ship/plan ", "+ship/order depart ", "+ship/undock "] {
                    if let Some(pos) = combined.find(prefix) {
                        let rest = &combined[pos + prefix.len()..];
                        let word: String = rest
                            .chars()
                            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                            .collect();
                        if let Some((id, label, _)) =
                            KNOWN_DESTINATIONS.iter().find(|(id, _, _)| *id == word)
                        {
                            return Some(QuestDestinationHint {
                                quest_title: q.title.clone(),
                                port_id: (*id).to_string(),
                                port_label: (*label).to_string(),
                            });
                        }
                    }
                }
                if let Some((id, label, _)) = KNOWN_DESTINATIONS
                    .iter()
                    .find(|(_, _, pats)| pats.iter().any(|p| combined.contains(p)))
                {
                    return Some(QuestDestinationHint {
                        quest_title: q.title.clone(),
                        port_id: (*id).to_string(),
                        port_label: (*label).to_string(),
                    });
                }
            }
        }
        None
    }

    #[must_use]
    pub fn surface_mining_status(
        &self,
        tracked_key: Option<&str>,
    ) -> Option<QuestSurfaceMiningStatus> {
        for q in self.ordered_active(tracked_key) {
            let mut has_any = false;
            let mut scan_done = false;
            let mut sound_done = false;
            let mut mine_done = false;
            let mut ore_slug = String::new();
            for cp in &q.checkpoints {
                let id_low = cp.id.to_ascii_lowercase();
                let text_low = format!(
                    "{id_low} {} {}",
                    cp.title.to_ascii_lowercase(),
                    cp.instructions.to_ascii_lowercase()
                );
                let done = cp.checked || cp.live_ready;
                if id_low.contains("scan")
                    || text_low.contains("surface scan")
                    || text_low.contains("scan go")
                {
                    has_any = true;
                    scan_done = done;
                } else if id_low.contains("sound")
                    || text_low.contains("sound a detected")
                    || text_low.contains("sound go")
                {
                    has_any = true;
                    sound_done = done;
                } else if id_low.starts_with("mine_")
                    || text_low.contains("mine at least")
                    || text_low.contains("mine go")
                {
                    has_any = true;
                    mine_done = done;
                    if text_low.contains("carbon_ore") || text_low.contains("carbon ore") {
                        ore_slug = "carbon_ore".to_string();
                    } else if text_low.contains("rare_earths") || text_low.contains("rare earths") {
                        ore_slug = "rare_earths".to_string();
                    } else if text_low.contains("volatiles") || text_low.contains("water_ice") {
                        ore_slug = "volatiles".to_string();
                    } else if text_low.contains("metals") {
                        ore_slug = "metals".to_string();
                    }
                }
            }
            if has_any {
                if ore_slug.is_empty() {
                    ore_slug = "carbon_ore".to_string();
                }
                let recommended_family = match ore_slug.as_str() {
                    "carbon_ore" => "carbon",
                    "rare_earths" => "strategic",
                    "metals" => "metals",
                    _ => "volatiles",
                }
                .to_string();
                return Some(QuestSurfaceMiningStatus {
                    quest_title: q.title.clone(),
                    target_ore_slug: ore_slug,
                    recommended_family,
                    scan_done,
                    sound_done,
                    mine_done,
                });
            }
        }
        None
    }
}

fn str_field(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .map(clean)
        .unwrap_or_default()
}

fn bool_field(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn parse_markers_value(raw: Option<&Value>) -> Vec<QuestMarker> {
    let Some(arr) = raw.and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let npc_id = str_field(item, "npc_id");
        let npc_name = str_field(item, "npc_name");
        let marker = str_field(item, "marker");
        if npc_id.is_empty() || npc_name.is_empty() {
            continue;
        }
        if !matches!(marker.as_str(), "turn_in" | "available" | "in_progress") {
            continue;
        }
        let symbol = if marker == "available" {
            "!".to_owned()
        } else {
            "?".to_owned()
        };
        out.push(QuestMarker {
            npc_id,
            npc_name,
            marker,
            symbol,
            quest_key: str_field(item, "quest_key"),
            quest_title: str_field(item, "quest_title"),
            category: str_field(item, "category"),
        });
    }
    out
}

fn parse_checkpoints(raw: Option<&Value>) -> Vec<QuestCheckpoint> {
    let Some(arr) = raw.and_then(Value::as_array) else {
        return Vec::new();
    };
    arr.iter()
        .map(|cp| QuestCheckpoint {
            id: str_field(cp, "id"),
            title: str_field(cp, "title"),
            instructions: str_field(cp, "instructions"),
            room_name: str_field(cp, "room_name"),
            checked: bool_field(cp, "checked"),
            live_ready: bool_field(cp, "live_ready"),
            detail_text: cp.get("detail_text").and_then(Value::as_str).map(clean),
        })
        .collect()
}

/// Extract quest markers from a `RoomView`'s `ambient["quest_markers"]`.
#[must_use]
pub fn parse_room_markers(view: &RoomView) -> Vec<QuestMarker> {
    parse_markers_value(view.ambient.get("quest_markers"))
}

/// Return a compact colored badge (`" [!]"`, `" [?]"`) for an NPC listed in `RoomView`.
#[must_use]
pub fn marker_badge_for_npc(view: &RoomView, npc_id: &str) -> Option<(&'static str, Style)> {
    let markers = parse_room_markers(view);
    let found = markers.iter().find(|m| m.npc_id == npc_id)?;
    match found.marker.as_str() {
        "turn_in" => Some((
            " [?]",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        "available" => Some((
            " [!]",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        "in_progress" => Some((
            " [?]",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )),
        _ => None,
    }
}

/// Parse a `"tacsim.quests"` panel keyframe defensively.
#[must_use]
pub fn parse_frame(data: &Value) -> QuestsSnapshot {
    let active = data
        .get("active")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let key = str_field(item, "key");
                    if key.is_empty() {
                        return None;
                    }
                    Some(ActiveQuest {
                        key,
                        title: str_field(item, "title"),
                        category: str_field(item, "category"),
                        introduction: str_field(item, "introduction"),
                        reward_summary: str_field(item, "reward_summary"),
                        giver_name: str_field(item, "giver_name"),
                        turn_in_name: str_field(item, "turn_in_name"),
                        allow_restart: bool_field(item, "allow_restart"),
                        ready_to_finish: bool_field(item, "ready_to_finish"),
                        next_step: str_field(item, "next_step"),
                        next_room_name: str_field(item, "next_room_name"),
                        can_check_now: bool_field(item, "can_check_now"),
                        can_finish_now: bool_field(item, "can_finish_now"),
                        check_cmd: str_field(item, "check_cmd"),
                        finish_cmd: str_field(item, "finish_cmd"),
                        abandon_cmd: str_field(item, "abandon_cmd"),
                        checkpoints: parse_checkpoints(item.get("checkpoints")),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let available = data
        .get("available")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let key = str_field(item, "key");
                    if key.is_empty() {
                        return None;
                    }
                    Some(AvailableQuest {
                        key,
                        title: str_field(item, "title"),
                        category: str_field(item, "category"),
                        introduction: str_field(item, "introduction"),
                        reward_summary: str_field(item, "reward_summary"),
                        giver_name: str_field(item, "giver_name"),
                        turn_in_name: str_field(item, "turn_in_name"),
                        allow_restart: bool_field(item, "allow_restart"),
                        accept_cmd: str_field(item, "accept_cmd"),
                        decline_cmd: str_field(item, "decline_cmd"),
                        checkpoints: parse_checkpoints(item.get("checkpoints")),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let completed = data
        .get("completed")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let key = str_field(item, "key");
                    if key.is_empty() {
                        return None;
                    }
                    Some(CompletedQuest {
                        key,
                        title: str_field(item, "title"),
                        category: str_field(item, "category"),
                        giver_name: str_field(item, "giver_name"),
                        turn_in_name: str_field(item, "turn_in_name"),
                        completion: str_field(item, "completion"),
                        reward_summary: str_field(item, "reward_summary"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    QuestsSnapshot {
        room_name: str_field(data, "room_name"),
        room_markers: parse_markers_value(data.get("room_markers")),
        active,
        available,
        completed,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum QuestTab {
    #[default]
    Active,
    Available,
    Completed,
}

impl QuestTab {
    fn next(self) -> Self {
        match self {
            Self::Active => Self::Available,
            Self::Available => Self::Completed,
            Self::Completed => Self::Active,
        }
    }

    fn prev(self) -> Self {
        match self {
            Self::Active => Self::Completed,
            Self::Available => Self::Active,
            Self::Completed => Self::Available,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuestOverlayAction {
    None,
    Close,
    Submit(String),
}

#[derive(Clone, Debug, Default)]
pub struct QuestOverlayState {
    pub open: bool,
    pub tab: QuestTab,
    pub selected: usize,
    pub tracked_key: Option<String>,
    pub confirm_abandon: bool,
}

impl QuestOverlayState {
    pub fn open_for(&mut self, snap: &QuestsSnapshot) {
        self.open = true;
        self.confirm_abandon = false;
        self.selected = 0;
        self.tab = if !snap.available.is_empty() && snap.active.is_empty() {
            QuestTab::Available
        } else if !snap.active.is_empty() {
            QuestTab::Active
        } else if !snap.available.is_empty() {
            QuestTab::Available
        } else {
            QuestTab::Completed
        };
    }

    /// Open the quest dialog focused directly on the quest offered by or turn-in ready at `npc_id` / `npc_name`.
    pub fn open_for_npc(&mut self, snap: &QuestsSnapshot, npc_id: &str, npc_name: &str) {
        self.open = true;
        self.confirm_abandon = false;

        let marker = snap.room_markers.iter().find(|m| {
            (!npc_id.is_empty() && m.npc_id == npc_id)
                || (!npc_name.is_empty() && m.npc_name.eq_ignore_ascii_case(npc_name))
        });

        if let Some(m) = marker {
            if m.marker == "available" {
                if let Some(idx) = snap
                    .available
                    .iter()
                    .position(|q| !m.quest_key.is_empty() && q.key == m.quest_key)
                    .or_else(|| {
                        snap.available
                            .iter()
                            .position(|q| q.giver_name.eq_ignore_ascii_case(&m.npc_name))
                    })
                {
                    self.tab = QuestTab::Available;
                    self.selected = idx;
                    return;
                }
            } else if let Some(idx) = snap
                .active
                .iter()
                .position(|q| !m.quest_key.is_empty() && q.key == m.quest_key)
                .or_else(|| {
                    snap.active.iter().position(|q| {
                        q.turn_in_name.eq_ignore_ascii_case(&m.npc_name)
                            || q.giver_name.eq_ignore_ascii_case(&m.npc_name)
                    })
                })
            {
                self.tab = QuestTab::Active;
                self.selected = idx;
                return;
            }
        }

        if !npc_name.is_empty() {
            if let Some(idx) = snap
                .available
                .iter()
                .position(|q| q.giver_name.eq_ignore_ascii_case(npc_name))
            {
                self.tab = QuestTab::Available;
                self.selected = idx;
                return;
            }
            if let Some(idx) = snap.active.iter().position(|q| {
                q.turn_in_name.eq_ignore_ascii_case(npc_name)
                    || q.giver_name.eq_ignore_ascii_case(npc_name)
            }) {
                self.tab = QuestTab::Active;
                self.selected = idx;
                return;
            }
        }

        self.open_for(snap);
    }

    pub fn close(&mut self) {
        self.open = false;
        self.confirm_abandon = false;
    }

    pub fn tab_len(&self, snap: &QuestsSnapshot) -> usize {
        match self.tab {
            QuestTab::Active => snap.active.len(),
            QuestTab::Available => snap.available.len(),
            QuestTab::Completed => snap.completed.len(),
        }
    }

    pub fn clamp_selection(&mut self, snap: &QuestsSnapshot) {
        let len = self.tab_len(snap);
        if len == 0 {
            self.selected = 0;
        } else if self.selected >= len {
            self.selected = len - 1;
        }
    }

    pub fn handle_key(&mut self, key: Key, snap: &QuestsSnapshot) -> QuestOverlayAction {
        self.clamp_selection(snap);
        match key {
            Key::Esc | Key::Char('q') => {
                self.close();
                QuestOverlayAction::Close
            }
            Key::Tab | Key::Right => {
                self.tab = self.tab.next();
                self.selected = 0;
                self.confirm_abandon = false;
                QuestOverlayAction::None
            }
            Key::BackTab | Key::Left => {
                self.tab = self.tab.prev();
                self.selected = 0;
                self.confirm_abandon = false;
                QuestOverlayAction::None
            }
            Key::Up | Key::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                self.confirm_abandon = false;
                QuestOverlayAction::None
            }
            Key::Down | Key::Char('j') => {
                let len = self.tab_len(snap);
                if len > 0 && self.selected + 1 < len {
                    self.selected += 1;
                }
                self.confirm_abandon = false;
                QuestOverlayAction::None
            }
            Key::Char('a') | Key::Enter if self.tab == QuestTab::Available => {
                self.confirm_abandon = false;
                if let Some(offer) = snap.available.get(self.selected)
                    && !offer.accept_cmd.is_empty()
                {
                    let cmd = offer.accept_cmd.clone();
                    self.tracked_key = Some(offer.key.clone());
                    self.tab = QuestTab::Active;
                    self.selected = 0;
                    return QuestOverlayAction::Submit(cmd);
                }
                QuestOverlayAction::None
            }
            Key::Enter | Key::Char('c') | Key::Char('f') if self.tab == QuestTab::Active => {
                self.confirm_abandon = false;
                if let Some(item) = snap.active.get(self.selected) {
                    if item.can_finish_now && !item.finish_cmd.is_empty() {
                        return QuestOverlayAction::Submit(item.finish_cmd.clone());
                    }
                    if item.can_check_now && !item.check_cmd.is_empty() {
                        return QuestOverlayAction::Submit(item.check_cmd.clone());
                    }
                }
                QuestOverlayAction::None
            }
            Key::Char('t') if self.tab == QuestTab::Active => {
                self.confirm_abandon = false;
                if let Some(item) = snap.active.get(self.selected) {
                    self.tracked_key = Some(item.key.clone());
                }
                QuestOverlayAction::None
            }
            Key::Char('x') if self.tab == QuestTab::Active => {
                if let Some(item) = snap.active.get(self.selected)
                    && !item.abandon_cmd.is_empty()
                {
                    if self.confirm_abandon {
                        self.confirm_abandon = false;
                        return QuestOverlayAction::Submit(item.abandon_cmd.clone());
                    }
                    self.confirm_abandon = true;
                }
                QuestOverlayAction::None
            }
            _ => {
                self.confirm_abandon = false;
                QuestOverlayAction::None
            }
        }
    }
}

/// Build compact tracker lines for the Scene screen right rail.
#[must_use]
pub fn render_quest_tracker_lines(
    snap: &QuestsSnapshot,
    tracked_key: Option<&str>,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let tracked = tracked_key
        .and_then(|k| snap.active.iter().find(|q| q.key == k))
        .or_else(|| snap.active.first());

    if let Some(q) = tracked {
        let done = q.checkpoints.iter().filter(|c| c.checked).count();
        let sym = if q.can_finish_now || q.ready_to_finish {
            "[?]"
        } else {
            "[*]"
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{sym} "),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                q.title.clone(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" ({done}/{})", q.checkpoints.len()),
                Style::default().add_modifier(Modifier::DIM),
            ),
        ]));
        for cp in &q.checkpoints {
            let (mark, style) = if cp.checked {
                ("[x] ", Style::default().fg(Color::Green))
            } else if cp.live_ready {
                ("[*] ", Style::default().fg(Color::Yellow))
            } else {
                ("[ ] ", Style::default())
            };
            lines.push(Line::from(vec![
                Span::styled(mark, style),
                Span::styled(cp.title.clone(), style),
            ]));
        }
        if !q.next_step.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("Next: ", Style::default().fg(Color::Cyan)),
                Span::raw(q.next_step.clone()),
            ]));
        }
        lines.push(Line::from(Span::styled(
            "F11 or /quests for dialog",
            Style::default().add_modifier(Modifier::DIM),
        )));
    } else if let Some(offer) = snap.available.first() {
        lines.push(Line::from(vec![
            Span::styled(
                "[!] ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(offer.giver_name.clone(), Style::default().fg(Color::Yellow)),
        ]));
        lines.push(Line::from(offer.title.clone()));
        lines.push(Line::from(Span::styled(
            "Press F11 or /quests",
            Style::default().add_modifier(Modifier::DIM),
        )));
    }
    lines
}

/// Paint the centered Quest Dialog & Journal overlay.
pub fn render_quest_overlay(
    f: &mut Frame<'_>,
    area: Rect,
    state: &QuestOverlayState,
    snap: &QuestsSnapshot,
) {
    let w = area.width.saturating_sub(6).min(96);
    let h = area.height.saturating_sub(4).min(28);
    if w < 30 || h < 10 {
        return;
    }
    let popup = Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, popup);

    let title = format!(
        " Quests — Active({}) | Available Here({}) | Completed({}) [Tab/Arrows/Esc] ",
        snap.active.len(),
        snap.available.len(),
        snap.completed.len()
    );
    let outer = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let inner = outer.inner(popup);
    f.render_widget(outer, popup);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(1),
        ])
        .split(inner);

    // Tab bar
    let tab_span = |label: String, active: bool| {
        if active {
            Span::styled(
                format!(" [{label}] "),
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(
                format!("  {label}  "),
                Style::default().add_modifier(Modifier::DIM),
            )
        }
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            tab_span(
                format!("Active ({})", snap.active.len()),
                state.tab == QuestTab::Active,
            ),
            tab_span(
                format!("Available Here ({})", snap.available.len()),
                state.tab == QuestTab::Available,
            ),
            tab_span(
                format!("Completed ({})", snap.completed.len()),
                state.tab == QuestTab::Completed,
            ),
        ])),
        chunks[0],
    );

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(36), Constraint::Percentage(64)])
        .split(chunks[1]);

    // Left list
    let mut list_lines: Vec<Line<'static>> = Vec::new();
    match state.tab {
        QuestTab::Active => {
            if snap.active.is_empty() {
                list_lines.push(Line::from(Span::styled(
                    "No active quests.",
                    Style::default().add_modifier(Modifier::DIM),
                )));
            }
            for (idx, q) in snap.active.iter().enumerate() {
                let selected = idx == state.selected;
                let badge = if q.can_finish_now || q.ready_to_finish {
                    "[?]"
                } else {
                    "[ ]"
                };
                let star = if state.tracked_key.as_deref() == Some(q.key.as_str()) {
                    "*"
                } else {
                    " "
                };
                let style = if selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                list_lines.push(Line::from(Span::styled(
                    format!("{star}{badge} {}", q.title),
                    style,
                )));
            }
        }
        QuestTab::Available => {
            if snap.available.is_empty() {
                list_lines.push(Line::from(Span::styled(
                    "No quests offered here.",
                    Style::default().add_modifier(Modifier::DIM),
                )));
            }
            for (idx, q) in snap.available.iter().enumerate() {
                let selected = idx == state.selected;
                let style = if selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Yellow)
                };
                list_lines.push(Line::from(Span::styled(format!(" [!] {}", q.title), style)));
            }
        }
        QuestTab::Completed => {
            if snap.completed.is_empty() {
                list_lines.push(Line::from(Span::styled(
                    "No completed quests yet.",
                    Style::default().add_modifier(Modifier::DIM),
                )));
            }
            for (idx, q) in snap.completed.iter().enumerate() {
                let selected = idx == state.selected;
                let style = if selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Green)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Green)
                };
                list_lines.push(Line::from(Span::styled(format!(" [x] {}", q.title), style)));
            }
        }
    }
    f.render_widget(
        Paragraph::new(list_lines)
            .block(Block::default().borders(Borders::RIGHT))
            .wrap(Wrap { trim: true }),
        cols[0],
    );

    // Right detail / dialog pane
    let mut detail_lines: Vec<Line<'static>> = Vec::new();
    match state.tab {
        QuestTab::Active => {
            if let Some(q) = snap.active.get(state.selected) {
                detail_lines.push(Line::from(Span::styled(
                    q.title.clone(),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )));
                detail_lines.push(Line::from(Span::styled(
                    format!("Giver: {}  |  Turn in: {}", q.giver_name, q.turn_in_name),
                    Style::default().fg(Color::Cyan),
                )));
                detail_lines.push(Line::from(""));
                detail_lines.push(Line::from(q.introduction.clone()));
                detail_lines.push(Line::from(""));
                detail_lines.push(Line::from(Span::styled(
                    "Objectives:",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )));
                for cp in &q.checkpoints {
                    let (mark, style) = if cp.checked {
                        ("[x] ", Style::default().fg(Color::Green))
                    } else if cp.live_ready {
                        ("[*] ", Style::default().fg(Color::Yellow))
                    } else {
                        ("[ ] ", Style::default())
                    };
                    detail_lines.push(Line::from(vec![
                        Span::styled(mark, style),
                        Span::styled(cp.title.clone(), style),
                    ]));
                    if let Some(done_text) = &cp.detail_text {
                        detail_lines.push(Line::from(Span::styled(
                            format!("    {done_text}"),
                            Style::default().fg(Color::Green),
                        )));
                    } else {
                        detail_lines.push(Line::from(Span::styled(
                            format!("    ({}) {}", cp.room_name, cp.instructions),
                            Style::default().add_modifier(Modifier::DIM),
                        )));
                    }
                }
                detail_lines.push(Line::from(""));
                detail_lines.push(Line::from(vec![
                    Span::styled(
                        "Reward: ",
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(q.reward_summary.clone()),
                ]));
                if state.confirm_abandon {
                    detail_lines.push(Line::from(Span::styled(
                        "Press 'x' again to confirm abandoning this quest.",
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    )));
                }
            }
        }
        QuestTab::Available => {
            if let Some(q) = snap.available.get(state.selected) {
                detail_lines.push(Line::from(Span::styled(
                    format!("[!] {}", q.title),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )));
                detail_lines.push(Line::from(Span::styled(
                    format!(
                        "Offered by: {}  |  Turn in: {}",
                        q.giver_name, q.turn_in_name
                    ),
                    Style::default().fg(Color::Cyan),
                )));
                detail_lines.push(Line::from(""));
                detail_lines.push(Line::from(q.introduction.clone()));
                detail_lines.push(Line::from(""));
                detail_lines.push(Line::from(Span::styled(
                    "Requirements:",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )));
                for cp in &q.checkpoints {
                    detail_lines.push(Line::from(format!(" [ ] {} ({})", cp.title, cp.room_name)));
                    detail_lines.push(Line::from(Span::styled(
                        format!("     {}", cp.instructions),
                        Style::default().add_modifier(Modifier::DIM),
                    )));
                }
                detail_lines.push(Line::from(""));
                detail_lines.push(Line::from(vec![
                    Span::styled(
                        "Reward: ",
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(q.reward_summary.clone()),
                ]));
            }
        }
        QuestTab::Completed => {
            if let Some(q) = snap.completed.get(state.selected) {
                detail_lines.push(Line::from(Span::styled(
                    format!("[x] {}", q.title),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )));
                detail_lines.push(Line::from(Span::styled(
                    format!("Completed with: {}", q.turn_in_name),
                    Style::default().fg(Color::Cyan),
                )));
                detail_lines.push(Line::from(""));
                detail_lines.push(Line::from(q.completion.clone()));
                detail_lines.push(Line::from(""));
                detail_lines.push(Line::from(vec![
                    Span::styled("Reward: ", Style::default().fg(Color::Yellow)),
                    Span::raw(q.reward_summary.clone()),
                ]));
            }
        }
    }
    f.render_widget(
        Paragraph::new(detail_lines).wrap(Wrap { trim: true }),
        cols[1],
    );

    // Footer legend
    let footer = match state.tab {
        QuestTab::Active => {
            "Enter/c/f: verify or complete  |  t: track  |  x: abandon  |  Tab: switch tab  |  Esc: close"
        }
        QuestTab::Available => "Enter/a: accept quest  |  Tab: switch tab  |  Esc: close",
        QuestTab::Completed => "Up/Down: browse history  |  Tab: switch tab  |  Esc: close",
    };
    f.render_widget(
        Paragraph::new(Span::styled(
            footer,
            Style::default().add_modifier(Modifier::DIM),
        )),
        chunks[2],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn quest_overlay_parses_panel_and_submits_accept_and_abandon_commands() {
        let data = json!({
            "schema": 1,
            "room_name": "Port Watch Arrivals",
            "room_markers": [{
                "npc_id": "#42",
                "npc_name": "Maren Halvorsen",
                "marker": "available",
                "symbol": "!",
                "quest_key": "tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll",
                "quest_title": "First Papers at the Registry",
                "category": "guided"
            }],
            "active": [{
                "key": "tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll",
                "title": "First Papers at the Registry",
                "category": "guided",
                "introduction": "Visit Ilse.",
                "reward_summary": "Step 1/7",
                "giver_name": "Maren Halvorsen",
                "turn_in_name": "Ilse Oduya-Brandt",
                "allow_restart": true,
                "ready_to_finish": true,
                "next_step": "Turn in to Ilse",
                "next_room_name": "Registry",
                "can_check_now": false,
                "can_finish_now": true,
                "check_cmd": "+quest/check #18=k1",
                "finish_cmd": "+quest/finish #18=k1",
                "abandon_cmd": "+quest/abandon #42=k1",
                "checkpoints": []
            }],
            "available": [{
                "key": "tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel",
                "title": "Stock the Locker and Tanks",
                "category": "guided",
                "introduction": "Stock up.",
                "reward_summary": "Step 2/7",
                "giver_name": "Ilse Oduya-Brandt",
                "turn_in_name": "Ilse Oduya-Brandt",
                "allow_restart": true,
                "accept_cmd": "+quest/accept #18=123.abc",
                "decline_cmd": "+quest/decline #18=123.abc",
                "checkpoints": []
            }],
            "completed": []
        });
        let snap = parse_frame(&data);
        assert_eq!(snap.active.len(), 1);
        assert_eq!(snap.available.len(), 1);

        let mut overlay = QuestOverlayState::default();
        overlay.open_for(&snap);
        assert_eq!(overlay.tab, QuestTab::Active);
        assert_eq!(
            overlay.handle_key(Key::Enter, &snap),
            QuestOverlayAction::Submit("+quest/finish #18=k1".into())
        );
        // Abandon requires two 'x' presses.
        assert_eq!(
            overlay.handle_key(Key::Char('x'), &snap),
            QuestOverlayAction::None
        );
        assert!(overlay.confirm_abandon);
        assert_eq!(
            overlay.handle_key(Key::Char('x'), &snap),
            QuestOverlayAction::Submit("+quest/abandon #42=k1".into())
        );

        // Switch to Available tab and accept.
        assert_eq!(
            overlay.handle_key(Key::Tab, &snap),
            QuestOverlayAction::None
        );
        assert_eq!(overlay.tab, QuestTab::Available);
        assert_eq!(
            overlay.handle_key(Key::Char('a'), &snap),
            QuestOverlayAction::Submit("+quest/accept #18=123.abc".into())
        );
    }

    #[test]
    fn extracts_trade_need_destination_port_and_surface_mining_status() {
        let step3 = parse_frame(&json!({
            "schema": 1,
            "room_name": "Exchange Hall",
            "active": [{
                "key": "tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery",
                "title": "Metals Down the Corridor",
                "checkpoints": [{
                    "id": "deliver_metals",
                    "title": "Buy 5 tonnes of metals at Ceres and deliver them to Stickney Yards",
                    "instructions": "Buy 5 metals, check +ship/plan stickney, and run +market/sell.",
                    "checked": false,
                    "live_ready": false
                }]
            }]
        }));
        let need = step3.active_trade_need(None).expect("metals trade need");
        assert_eq!(need.commodity_slug, "metals");
        assert_eq!(need.target_quantity, 5);
        assert!(need.wants_buy && need.wants_sell);
        assert!(need.matches_listing("tacsim:metals", "Metals", false));
        assert!(need.matches_listing("tacsim:metals", "Metals", true));
        assert!(!need.matches_listing("tacsim:provisions", "Provisions", false));

        let dest = step3
            .active_destination_port(None)
            .expect("stickney destination");
        assert_eq!(dest.port_id, "stickney");
        assert_eq!(dest.port_label, "Stickney Yards");
        assert!(step3.surface_mining_status(None).is_none());

        let step6 = parse_frame(&json!({
            "schema": 1,
            "room_name": "Freeport Commons",
            "active": [{
                "key": "tacsim.quests.guided_onboarding/quest.guide_06_surface_mining",
                "title": "Scan, Sound, and Mine a Landing Site",
                "checkpoints": [
                    { "id": "scan_landing_site", "title": "Surface scan", "instructions": "Run +tv/1 scan go.", "checked": true, "live_ready": true },
                    { "id": "sound_mineral_pocket", "title": "Sound pocket", "instructions": "Run +tv/1 sound go.", "checked": false, "live_ready": false },
                    { "id": "mine_carbon_ore", "title": "Mine 1t carbon_ore", "instructions": "Run +tv/1 mine go.", "checked": false, "live_ready": false }
                ]
            }]
        }));
        assert!(step6.active_trade_need(None).is_none());
        let mine = step6
            .surface_mining_status(None)
            .expect("surface mining status");
        assert_eq!(mine.target_ore_slug, "carbon_ore");
        assert_eq!(mine.recommended_family, "carbon");
        assert!(mine.scan_done);
        assert!(!mine.sound_done);
        assert!(!mine.mine_done);
    }
}
