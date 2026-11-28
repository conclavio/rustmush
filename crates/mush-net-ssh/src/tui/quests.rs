//! WoW-style Quest Markers, Tracker, Quest Dialog & Completion Log for the SSH TUI.
//!
//! Consumes the `"tacsim.quests"` panel feed and `RoomView.ambient["quest_markers"]`.
//! Every mutation submits a server-signed `+quest/...` command after an explicit key
//! inside the modal overlay (never from a bare nav key on the main screen).

use std::collections::{BTreeMap, HashSet};

use mush_proto::RoomView;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use serde_json::Value;

use super::keys::Key;
use super::port_walk::QuestExitBreadcrumb;
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
    pub room_entry: String,
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
    pub turn_in_here: bool,
    pub allow_restart: bool,
    pub ready_to_finish: bool,
    pub next_step: String,
    pub next_room_entry: String,
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QuestTradeNeed {
    pub quest_title: String,
    pub checkpoint_title: String,
    pub commodity_slug: String,
    pub target_quantity: i64,
    pub wants_buy: bool,
    pub wants_sell: bool,
    pub room_entry: String,
    pub room_name: String,
}

/// Snapshot of live ship stowage passed into the F11 Quest Overlay so Steps 2, 3, 4, 5, and 7
/// display live hold quantity and Step 5 empty-hold refit readiness beside the quest objectives.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QuestStowageTelemetry {
    pub cargo_kg: i64,
    pub propellant_kg: i64,
    pub provisions_days: i64,
    pub cargo_lots: Vec<(String, String, i64)>,
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

    /// Human-readable station desk label when no matching listing appears on the current board.
    #[must_use]
    pub fn venue_hint_label(&self) -> Option<&str> {
        let trimmed = self.room_name.trim();
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
        match self.room_entry.trim() {
            "room.c08_chandlery" | "room.c06_chandlery" => Some("Co-op Chandlery"),
            "room.c07_exchange" => Some("Exchange Hall"),
            "room.c05_registry" => Some("Registry & Co-op Desk"),
            "room.p07_receiving" => Some("Receiving Counter"),
            "room.p11_service" => Some("Service Counter"),
            _ => match self.commodity_slug.as_str() {
                "provisions" => Some("Co-op Chandlery"),
                "propellant" => Some("Exchange Hall"),
                _ => None,
            },
        }
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestGroundSiteRecommendation {
    pub venue: String,
    pub site_name: String,
    pub here: bool,
}

impl QuestSurfaceMiningStatus {
    /// True when a reading/site text mentions the quest's target ore or recommended wash family.
    #[must_use]
    pub fn matches_ore_text(&self, text: &str) -> bool {
        if text.is_empty() {
            return false;
        }
        let low = text.to_ascii_lowercase();
        let slug = self.target_ore_slug.to_ascii_lowercase();
        let spaced = slug.replace('_', " ");
        let fam = self.recommended_family.to_ascii_lowercase();
        (!slug.is_empty() && (low.contains(&slug) || low.contains(&spaced)))
            || (!fam.is_empty() && low.contains(&fam))
    }

    /// Pick the best non-`here` landing site from a live `body.ground` frame:
    /// prefers a site with matching ore readings, falling back to open Step 6 starter sites
    /// (`commons` / `ahuna`) or the first open site before the pilot has scanned any ground.
    #[must_use]
    pub fn recommended_ground_site(
        &self,
        ground_frame: &Value,
    ) -> Option<QuestGroundSiteRecommendation> {
        let sites = ground_frame.pointer("/ground/sites")?.as_array()?;
        let parse_site = |item: &Value| -> Option<(String, String, bool, String, String, String)> {
            let venue = str_field(item, "venue");
            if venue.is_empty() {
                return None;
            }
            let site_name = str_field(item, "site");
            let here = bool_field(item, "here");
            let best = str_field(item, "best");
            let best_line = str_field(item, "best_line");
            let tenure = str_field(item, "tenure");
            Some((venue, site_name, here, best, best_line, tenure))
        };
        let parsed: Vec<_> = sites.iter().filter_map(parse_site).collect();

        if let Some((venue, site_name, here, _, _, _)) =
            parsed.iter().find(|(_, _, here, best, best_line, _)| {
                !*here && (self.matches_ore_text(best_line) || self.matches_ore_text(best))
            })
        {
            return Some(QuestGroundSiteRecommendation {
                venue: venue.clone(),
                site_name: if site_name.is_empty() {
                    venue.clone()
                } else {
                    site_name.clone()
                },
                here: *here,
            });
        }

        for preferred in ["commons", "ahuna"] {
            if let Some((venue, site_name, here, _, _, _)) = parsed
                .iter()
                .find(|(v, _, here, _, _, _)| !*here && v.eq_ignore_ascii_case(preferred))
            {
                return Some(QuestGroundSiteRecommendation {
                    venue: venue.clone(),
                    site_name: if site_name.is_empty() {
                        venue.clone()
                    } else {
                        site_name.clone()
                    },
                    here: *here,
                });
            }
        }

        parsed
            .into_iter()
            .find(|(_, _, here, _, _, tenure)| {
                !*here && tenure.to_ascii_lowercase().contains("open")
            })
            .map(
                |(venue, site_name, here, _, _, _)| QuestGroundSiteRecommendation {
                    site_name: if site_name.is_empty() {
                        venue.clone()
                    } else {
                        site_name
                    },
                    venue,
                    here,
                },
            )
    }

    /// Compact verified prospecting prep & rig recovery hint for Quarry HUD and Quest Overlay.
    #[must_use]
    pub fn prep_hint_line(&self) -> String {
        if !self.scan_done {
            "Prep: +license | +tv/lease | +chart/ground".to_string()
        } else if !self.mine_done {
            "Prep: +chart/ground | Recover: +tv/1 recover go".to_string()
        } else {
            format!(
                "Recover: +tv/1 recover go (unload {} into hold)",
                self.target_ore_slug
            )
        }
    }

    /// Return the first matching reading target (`"A"`, `"B"`, etc.) from a live `site.map`
    /// frame whose ore family or reading text matches this quest's target ore.
    #[must_use]
    pub fn first_matching_site_reading(&self, site_frame: &Value) -> Option<String> {
        let family = site_frame
            .pointer("/site/family")
            .and_then(Value::as_str)
            .unwrap_or("");
        let family_matches = !self.recommended_family.is_empty()
            && family.eq_ignore_ascii_case(&self.recommended_family);
        let readings = site_frame.pointer("/site/readings")?.as_array()?;
        for item in readings {
            let target = str_field(item, "target");
            if target.is_empty() {
                continue;
            }
            let line = str_field(item, "line");
            if family_matches || self.matches_ore_text(&line) {
                return Some(target);
            }
        }
        None
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuidedTourProgress {
    pub completed: usize,
    pub total: usize,
    pub current_step: Option<usize>,
    pub all_done: bool,
}

impl GuidedTourProgress {
    #[must_use]
    pub fn progress_bar(&self) -> String {
        let mut bar = String::with_capacity(self.total + 2);
        bar.push('[');
        for step in 1..=self.total {
            if step <= self.completed {
                bar.push('#');
            } else if !self.all_done && self.current_step == Some(step) {
                bar.push('>');
            } else {
                bar.push('-');
            }
        }
        bar.push(']');
        bar
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuidedTourStepStatus {
    Completed,
    Active,
    Available,
    Upcoming,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuidedTourRoadmapStep {
    pub step: usize,
    pub key: &'static str,
    pub short_label: &'static str,
    pub giver_label: &'static str,
    pub giver_room_entry: &'static str,
    pub giver_room_name: &'static str,
    pub location_label: &'static str,
    pub status: GuidedTourStepStatus,
    pub ready_to_turn_in: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestSuggestedAction {
    pub label: String,
    pub command: String,
}

pub const GUIDED_TOUR_ROADMAP_META: [(&str, &str, &str, &str, &str, &str); 7] = [
    (
        "tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll",
        "Co-op Charter",
        "Maren Halvorsen",
        "room.c03_arrivals",
        "Port Watch Arrivals",
        "Ceres Arrivals -> Registry",
    ),
    (
        "tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel",
        "Locker & Tanks",
        "Ilse Oduya-Brandt",
        "room.c05_registry",
        "Registry & Co-op Desk",
        "Ceres Chandlery & Exchange",
    ),
    (
        "tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery",
        "Metals Haul",
        "Ilse Oduya-Brandt",
        "room.c05_registry",
        "Registry & Co-op Desk",
        "Ceres -> Stickney Receiving",
    ),
    (
        "tacsim.quests.guided_onboarding/quest.guide_04_return_haul",
        "Return Load",
        "Jonas Achterberg-Nkemelu",
        "room.p07_receiving",
        "Receiving Counter",
        "Stickney -> Ceres Chandlery",
    ),
    (
        "tacsim.quests.guided_onboarding/quest.guide_05_refit_prospector",
        "Dray-P Refit",
        "Ilse Oduya-Brandt",
        "room.c05_registry",
        "Registry & Co-op Desk",
        "Ceres Registry -> Exchange",
    ),
    (
        "tacsim.quests.guided_onboarding/quest.guide_06_surface_mining",
        "Surface Mining",
        "Rosalind Qureshi-Mbeki",
        "room.c07_exchange",
        "Exchange Hall",
        "Ceres Surface Landing Site",
    ),
    (
        "tacsim.quests.guided_onboarding/quest.guide_07_sell_mined_ore",
        "Sell Mined Ore",
        "Rosalind Qureshi-Mbeki",
        "room.c07_exchange",
        "Exchange Hall",
        "Ceres -> Stickney Service",
    ),
];

pub const GUIDED_TOUR_KEYS: [&str; 7] = [
    GUIDED_TOUR_ROADMAP_META[0].0,
    GUIDED_TOUR_ROADMAP_META[1].0,
    GUIDED_TOUR_ROADMAP_META[2].0,
    GUIDED_TOUR_ROADMAP_META[3].0,
    GUIDED_TOUR_ROADMAP_META[4].0,
    GUIDED_TOUR_ROADMAP_META[5].0,
    GUIDED_TOUR_ROADMAP_META[6].0,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuestMilestoneKind {
    Accepted,
    Checkpoint,
    Ready,
    Completed,
    Abandoned,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestMilestone {
    pub kind: QuestMilestoneKind,
    pub title: String,
    pub detail: Option<String>,
    pub tour_all_done: bool,
    pub restartable: bool,
}

impl QuestMilestone {
    #[must_use]
    pub fn notice_text(&self) -> String {
        match self.kind {
            QuestMilestoneKind::Accepted => format!("[QUEST] Accepted: {}", self.title),
            QuestMilestoneKind::Checkpoint => {
                let step = self.detail.as_deref().unwrap_or(&self.title);
                format!("[QUEST] Objective Complete: {step}")
            }
            QuestMilestoneKind::Ready => format!("[QUEST] Ready to turn in: {}", self.title),
            QuestMilestoneKind::Completed => {
                if self.tour_all_done {
                    format!(
                        "[QUEST] ★ Corridor Captain (7/7) — Guided Tour Complete: {}",
                        self.title
                    )
                } else {
                    format!("[QUEST] Completed: {}", self.title)
                }
            }
            QuestMilestoneKind::Abandoned => {
                if self.restartable {
                    format!("[QUEST] Abandoned (restartable anytime): {}", self.title)
                } else {
                    format!("[QUEST] Abandoned: {}", self.title)
                }
            }
        }
    }
}

/// Tracks genuine post-hydration quest state transitions (accepted, checkpoint complete,
/// ready to turn in, completed + 7/7 mastery, abandoned) so initial login/reconnect
/// keyframes never spam duplicate notices.
#[derive(Clone, Debug, Default)]
pub struct QuestMilestoneTracker {
    hydrated: bool,
    seen_active: HashSet<String>,
    seen_active_meta: BTreeMap<String, (String, bool)>,
    seen_ready: HashSet<String>,
    seen_checkpoints: HashSet<String>,
    seen_completed: HashSet<String>,
    seen_tour_all_done: bool,
}

impl QuestMilestoneTracker {
    pub fn reset(&mut self) {
        self.hydrated = false;
        self.seen_active.clear();
        self.seen_active_meta.clear();
        self.seen_ready.clear();
        self.seen_checkpoints.clear();
        self.seen_completed.clear();
        self.seen_tour_all_done = false;
    }

    pub fn observe(&mut self, snap: &QuestsSnapshot) -> Vec<QuestMilestone> {
        let mut out = Vec::new();
        let current_active_keys: HashSet<&str> =
            snap.active.iter().map(|q| q.key.as_str()).collect();
        let current_completed_keys: HashSet<&str> =
            snap.completed.iter().map(|c| c.key.as_str()).collect();
        let tour_all_done = snap.guided_tour_progress().is_some_and(|p| p.all_done);
        let newly_mastered = tour_all_done && !self.seen_tour_all_done;

        if self.hydrated {
            for q in &snap.active {
                let was_active = self.seen_active.contains(&q.key);
                let is_ready = q.ready_to_finish || q.can_finish_now;
                let became_ready = is_ready && !self.seen_ready.contains(&q.key);
                if !was_active {
                    out.push(QuestMilestone {
                        kind: QuestMilestoneKind::Accepted,
                        title: q.title.clone(),
                        detail: None,
                        tour_all_done: false,
                        restartable: false,
                    });
                } else if !became_ready {
                    for cp in &q.checkpoints {
                        if !(cp.checked || cp.live_ready) {
                            continue;
                        }
                        let cp_id = if cp.id.is_empty() { &cp.title } else { &cp.id };
                        let cp_key = format!("{}:{cp_id}", q.key);
                        if !self.seen_checkpoints.contains(&cp_key) {
                            out.push(QuestMilestone {
                                kind: QuestMilestoneKind::Checkpoint,
                                title: q.title.clone(),
                                detail: Some(cp.title.clone()),
                                tour_all_done: false,
                                restartable: false,
                            });
                        }
                    }
                }
                if became_ready {
                    out.push(QuestMilestone {
                        kind: QuestMilestoneKind::Ready,
                        title: q.title.clone(),
                        detail: None,
                        tour_all_done: false,
                        restartable: false,
                    });
                }
            }
            for (prev_key, (prev_title, prev_restartable)) in &self.seen_active_meta {
                if !current_active_keys.contains(prev_key.as_str())
                    && !current_completed_keys.contains(prev_key.as_str())
                {
                    out.push(QuestMilestone {
                        kind: QuestMilestoneKind::Abandoned,
                        title: prev_title.clone(),
                        detail: None,
                        tour_all_done: false,
                        restartable: *prev_restartable,
                    });
                }
            }
            for c in &snap.completed {
                if !self.seen_completed.contains(&c.key) {
                    let is_mastery = newly_mastered && GUIDED_TOUR_KEYS.contains(&c.key.as_str());
                    out.push(QuestMilestone {
                        kind: QuestMilestoneKind::Completed,
                        title: c.title.clone(),
                        detail: None,
                        tour_all_done: is_mastery,
                        restartable: false,
                    });
                }
            }
        }
        self.hydrated = true;
        self.seen_active.clear();
        self.seen_active_meta.clear();
        self.seen_ready.clear();
        self.seen_checkpoints.clear();
        for q in &snap.active {
            self.seen_active.insert(q.key.clone());
            self.seen_active_meta
                .insert(q.key.clone(), (q.title.clone(), q.allow_restart));
            if q.ready_to_finish || q.can_finish_now {
                self.seen_ready.insert(q.key.clone());
            }
            for cp in &q.checkpoints {
                if cp.checked || cp.live_ready {
                    let cp_id = if cp.id.is_empty() { &cp.title } else { &cp.id };
                    self.seen_checkpoints.insert(format!("{}:{cp_id}", q.key));
                }
            }
        }
        self.seen_completed.clear();
        for c in &snap.completed {
            self.seen_completed.insert(c.key.clone());
        }
        self.seen_tour_all_done = tour_all_done;
        out
    }
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

/// Infer a corridor port slug (`"ceres"`, `"stickney"`, `"luna"`, `"deimos"`, `"titan"`)
/// from a canonical station room entry prefix (`room.c*`, `room.p*`, `room.l*`, `room.d*`, `room.t*`).
#[must_use]
pub fn infer_station_port_from_room_entry(room_entry: &str) -> Option<&'static str> {
    let low = room_entry.trim().to_ascii_lowercase();
    let rest = low.strip_prefix("room.")?;
    let mut chars = rest.chars();
    let prefix = chars.next()?;
    let second = chars.next()?;
    if !second.is_ascii_digit() {
        return None;
    }
    match prefix {
        'c' => Some("ceres"),
        'p' => Some("stickney"),
        'l' => Some("luna"),
        'd' => Some("deimos"),
        't' => Some("titan"),
        _ => None,
    }
}

impl QuestsSnapshot {
    /// Default tracked quest key: prefers an active Guided Tour quest over unrelated side jobs.
    #[must_use]
    pub fn default_tracked_key(&self) -> Option<&str> {
        self.active
            .iter()
            .find(|q| {
                GUIDED_TOUR_KEYS.contains(&q.key.as_str())
                    || q.category.eq_ignore_ascii_case("guided")
            })
            .or_else(|| self.active.first())
            .map(|q| q.key.as_str())
    }

    pub(crate) fn ordered_active<'a>(&'a self, tracked_key: Option<&str>) -> Vec<&'a ActiveQuest> {
        let mut out = Vec::with_capacity(self.active.len());
        let effective_key = tracked_key
            .filter(|tk| self.active.iter().any(|q| q.key == *tk))
            .or_else(|| self.default_tracked_key());
        if let Some(tk) = effective_key
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
    pub fn active_trade_needs(&self, tracked_key: Option<&str>) -> Vec<QuestTradeNeed> {
        for q in self.ordered_active(tracked_key) {
            let mut out = Vec::new();
            let mut seen = HashSet::new();
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
                let primary = format!("{id_low} {title_low}");
                let Some(slug) = KNOWN_TRADE_COMMODITIES
                    .iter()
                    .find(|(_, pats)| pats.iter().any(|p| primary.contains(p)))
                    .or_else(|| {
                        KNOWN_TRADE_COMMODITIES
                            .iter()
                            .find(|(_, pats)| pats.iter().any(|p| instr_low.contains(p)))
                    })
                    .map(|(slug, _)| *slug)
                else {
                    continue;
                };
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
                let dedup = (slug, wants_buy, wants_sell);
                if !seen.insert(dedup) {
                    continue;
                }
                let target_quantity = extract_leading_positive_i64(&cp.title)
                    .or_else(|| extract_leading_positive_i64(&cp.instructions))
                    .unwrap_or(1);
                out.push(QuestTradeNeed {
                    quest_title: q.title.clone(),
                    checkpoint_title: cp.title.clone(),
                    commodity_slug: slug.to_string(),
                    target_quantity,
                    wants_buy,
                    wants_sell,
                    room_entry: cp.room_entry.clone(),
                    room_name: cp.room_name.clone(),
                });
            }
            if !out.is_empty() {
                return out;
            }
        }
        Vec::new()
    }

    #[must_use]
    pub fn active_trade_need(&self, tracked_key: Option<&str>) -> Option<QuestTradeNeed> {
        self.active_trade_needs(tracked_key).into_iter().next()
    }

    #[must_use]
    pub fn active_destination_port(
        &self,
        tracked_key: Option<&str>,
    ) -> Option<QuestDestinationHint> {
        for q in self.ordered_active(tracked_key) {
            let mut has_pending_cp = false;
            for cp in &q.checkpoints {
                if cp.checked || cp.live_ready {
                    continue;
                }
                has_pending_cp = true;
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
            if !q.can_finish_now
                && !q.turn_in_here
                && (q.ready_to_finish || (!has_pending_cp && !q.checkpoints.is_empty()))
            {
                let turn_in_room = if !q.next_room_entry.is_empty() {
                    Some(q.next_room_entry.as_str())
                } else {
                    q.checkpoints
                        .last()
                        .map(|c| c.room_entry.as_str())
                        .filter(|s| !s.is_empty())
                };
                if let Some(port_slug) = turn_in_room.and_then(infer_station_port_from_room_entry)
                    && let Some((id, label, _)) = KNOWN_DESTINATIONS
                        .iter()
                        .find(|(id, _, _)| *id == port_slug)
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

    /// Returns `(room_entry, is_turn_in)` for the tracked active quest's next station room objective,
    /// falling back to the next guided NPC giver room between steps.
    #[must_use]
    pub fn active_objective_room(&self, tracked_key: Option<&str>) -> Option<(String, bool)> {
        self.active_objective_room_with_stowage(tracked_key, None)
    }

    /// Stowage-aware variant of [`Self::active_objective_room`]: during Step 5 (`+coop/refit`),
    /// dynamically redirects station wayfinding to `room.c07_exchange` while cargo remains in the
    /// hold and `room.c08_chandlery` while only provisions remain, flipping back to the Co-op
    /// Registry once the hold is clear for refit.
    #[must_use]
    pub fn active_objective_room_with_stowage(
        &self,
        tracked_key: Option<&str>,
        stowage: Option<&QuestStowageTelemetry>,
    ) -> Option<(String, bool)> {
        if let Some(q) = self.ordered_active(tracked_key).into_iter().next() {
            let is_turn_in = q.ready_to_finish || q.can_finish_now;
            if !is_turn_in
                && let Some(st) = stowage
                && Self::suggested_action_for(q)
                    .is_some_and(|a| a.command.to_ascii_lowercase().starts_with("+coop/refit"))
            {
                let has_cargo = st.cargo_kg > 0 || st.cargo_lots.iter().any(|(_, _, qty)| *qty > 0);
                if has_cargo {
                    return Some(("room.c07_exchange".to_string(), false));
                }
                if st.provisions_days > 0 {
                    return Some(("room.c08_chandlery".to_string(), false));
                }
            }
            if !q.next_room_entry.is_empty() {
                return Some((q.next_room_entry.clone(), is_turn_in));
            }
            for cp in &q.checkpoints {
                if !cp.checked && !cp.live_ready && !cp.room_entry.is_empty() {
                    return Some((cp.room_entry.clone(), is_turn_in));
                }
            }
            return None;
        }
        self.next_guided_offer()
            .map(|next| (next.giver_room_entry.to_string(), false))
    }

    /// Resolve the ship interior target compartment (`"airlock"` when docked at the
    /// active quest's objective port, `"bay"` for active surface mining quests, or
    /// `"bridge"` for active corridor flight quests).
    #[must_use]
    pub fn active_ship_target_compartment(
        &self,
        tracked_key: Option<&str>,
    ) -> Option<&'static str> {
        self.active_ship_target_compartment_for_port(tracked_key, None)
    }

    /// Docked-port aware ship compartment resolver for cross-port disembark wayfinding.
    #[must_use]
    pub fn active_ship_target_compartment_for_port(
        &self,
        tracked_key: Option<&str>,
        docked_port: Option<&str>,
    ) -> Option<&'static str> {
        self.active_ship_target_compartment_for_location(tracked_key, docked_port, false)
    }

    /// Return a single-quest snapshot containing only the currently tracked active quest,
    /// preventing station/ship wayfinding from leaking into untracked quests in the log.
    #[must_use]
    pub fn tracked_only(&self, tracked_key: Option<&str>) -> Option<Self> {
        let q = self.ordered_active(tracked_key).into_iter().next()?;
        Some(Self {
            active: vec![q.clone()],
            ..Default::default()
        })
    }

    /// Docked-port and settled-pad aware ship compartment resolver:
    /// - Docked at station (`!settled_pad`) with incomplete Step 6 mining -> `"bridge"` (fly to surface site).
    /// - Settled on a surface pad (or station-docked unset) with incomplete Step 6 mining -> `"bay"`.
    /// - Step 6 mining complete, not yet docked at turn-in station -> `"bridge"` (lift off & return to Ceres).
    /// - Docked at turn-in station (`!settled_pad`) with mining done or non-mining quest -> `"airlock"`.
    #[must_use]
    pub fn active_ship_target_compartment_for_location(
        &self,
        tracked_key: Option<&str>,
        docked_port: Option<&str>,
        settled_pad: bool,
    ) -> Option<&'static str> {
        let tracked_snap = self.tracked_only(tracked_key);
        let mining = tracked_snap
            .as_ref()
            .and_then(|s| s.surface_mining_status(tracked_key));
        let docked_station = if settled_pad {
            None
        } else {
            docked_port.filter(|s| !s.is_empty())
        };
        if let Some(docked) = docked_station
            && let Some(q) = tracked_snap.as_ref().and_then(|s| s.active.first())
        {
            let mining_blocking = mining.as_ref().is_some_and(|m| !m.mine_done);
            if (!mining_blocking || q.ready_to_finish || q.can_finish_now)
                && let Some((room_entry, _)) = self.active_objective_room(tracked_key)
                && let Some(target_port) = infer_station_port_from_room_entry(&room_entry)
                && target_port.eq_ignore_ascii_case(docked)
            {
                return Some("airlock");
            }
        }
        if let Some(m) = mining {
            if !m.mine_done {
                return if docked_station.is_some() {
                    Some("bridge")
                } else {
                    Some("bay")
                };
            }
            return Some("bridge");
        }
        if tracked_snap
            .as_ref()
            .and_then(|s| s.active_destination_port(tracked_key))
            .is_some()
        {
            return Some("bridge");
        }
        if self.active.is_empty()
            && let Some(next) = self.next_guided_offer()
        {
            if let Some(docked) = docked_station
                && let Some(target_port) = infer_station_port_from_room_entry(next.giver_room_entry)
                && target_port.eq_ignore_ascii_case(docked)
            {
                return Some("airlock");
            }
            return Some("bridge");
        }
        None
    }

    /// Compute 7-step guided onboarding progress across active, available, and completed quests.
    #[must_use]
    pub fn guided_tour_progress(&self) -> Option<GuidedTourProgress> {
        let step_of = |key: &str| {
            GUIDED_TOUR_KEYS
                .iter()
                .position(|&k| k == key)
                .map(|i| i + 1)
        };
        let completed_keys: HashSet<&str> = self
            .completed
            .iter()
            .map(|c| c.key.as_str())
            .filter(|k| step_of(k).is_some())
            .collect();
        let active_step = self.active.iter().find_map(|q| step_of(&q.key));
        let avail_step = self.available.iter().find_map(|q| step_of(&q.key));
        if completed_keys.is_empty() && active_step.is_none() && avail_step.is_none() {
            return None;
        }
        let completed = completed_keys.len();
        let total = GUIDED_TOUR_KEYS.len();
        Some(GuidedTourProgress {
            completed,
            total,
            current_step: active_step.or(avail_step),
            all_done: completed >= total,
        })
    }

    /// Build the 7-step Guided Corridor Tour roadmap with live completed / active /
    /// available / upcoming status for each step.
    #[must_use]
    pub fn guided_tour_roadmap(&self) -> Vec<GuidedTourRoadmapStep> {
        GUIDED_TOUR_ROADMAP_META
            .iter()
            .enumerate()
            .map(
                |(
                    idx,
                    &(
                        key,
                        short_label,
                        giver_label,
                        giver_room_entry,
                        giver_room_name,
                        location_label,
                    ),
                )| {
                    let active_match = self.active.iter().find(|q| q.key == key);
                    let status = if self.completed.iter().any(|c| c.key == key) {
                        GuidedTourStepStatus::Completed
                    } else if active_match.is_some() {
                        GuidedTourStepStatus::Active
                    } else if self.available.iter().any(|a| a.key == key) {
                        GuidedTourStepStatus::Available
                    } else {
                        GuidedTourStepStatus::Upcoming
                    };
                    GuidedTourRoadmapStep {
                        step: idx + 1,
                        key,
                        short_label,
                        giver_label,
                        giver_room_entry,
                        giver_room_name,
                        location_label,
                        status,
                        ready_to_turn_in: active_match
                            .is_some_and(|q| q.can_finish_now || q.ready_to_finish),
                    }
                },
            )
            .collect()
    }

    /// Between-steps guided tour locator: when at least 1 guided step is complete (and < 7),
    /// with no active quest and no offer in the current room, returns the next upcoming step.
    #[must_use]
    pub fn next_guided_offer(&self) -> Option<GuidedTourRoadmapStep> {
        if !self.active.is_empty() || !self.available.is_empty() {
            return None;
        }
        let roadmap = self.guided_tour_roadmap();
        let done_count = roadmap
            .iter()
            .filter(|s| s.status == GuidedTourStepStatus::Completed)
            .count();
        if done_count == 0 || done_count >= roadmap.len() {
            return None;
        }
        roadmap
            .into_iter()
            .find(|s| s.status == GuidedTourStepStatus::Upcoming)
    }

    /// Resolve a corridor destination hint for the between-steps Next Guided Offer when the
    /// pilot is viewing the Sky Chart from a different station/body than the next giver's port.
    #[must_use]
    pub fn next_guided_offer_destination(
        &self,
        current_port_or_body: Option<&str>,
    ) -> Option<QuestDestinationHint> {
        let next = self.next_guided_offer()?;
        let port_slug = infer_station_port_from_room_entry(next.giver_room_entry)?;
        let (id, label, _) = KNOWN_DESTINATIONS
            .iter()
            .find(|(id, _, _)| *id == port_slug)?;
        if let Some(cur) = current_port_or_body {
            let cur_low = cur.trim().to_ascii_lowercase();
            let label_low = label.to_ascii_lowercase();
            if !cur_low.is_empty()
                && (cur_low == *id || cur_low.contains(id) || cur_low.contains(&label_low))
            {
                return None;
            }
        }
        Some(QuestDestinationHint {
            quest_title: format!(
                "Next Guided ({}/7): {} — {}",
                next.step, next.short_label, next.giver_label
            ),
            port_id: (*id).to_string(),
            port_label: (*label).to_string(),
        })
    }

    /// Render a compact, color-coded 7-step Guided Corridor Tour roadmap line for the F11 overlay.
    #[must_use]
    pub fn render_guided_tour_roadmap_line(&self) -> Option<Line<'static>> {
        self.guided_tour_progress()?;
        let steps = self.guided_tour_roadmap();
        let mut spans: Vec<Span<'static>> = Vec::with_capacity(steps.len() * 2);
        for (idx, s) in steps.iter().enumerate() {
            if idx > 0 {
                spans.push(Span::raw("  "));
            }
            let (badge, style) = match s.status {
                GuidedTourStepStatus::Completed => ("[x]", Style::default().fg(Color::Green)),
                GuidedTourStepStatus::Active => {
                    let mark = if s.ready_to_turn_in { "[?]" } else { "[>]" };
                    let color = if s.ready_to_turn_in {
                        Color::Green
                    } else {
                        Color::Yellow
                    };
                    (
                        mark,
                        Style::default().fg(color).add_modifier(Modifier::BOLD),
                    )
                }
                GuidedTourStepStatus::Available => (
                    "[!]",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                GuidedTourStepStatus::Upcoming => {
                    ("[ ]", Style::default().add_modifier(Modifier::DIM))
                }
            };
            spans.push(Span::styled(
                format!("{badge} {}. {}", s.step, s.short_label),
                style,
            ));
        }
        Some(Line::from(spans))
    }

    /// Extract a verified, registered next command to prefill for an active quest's next step.
    #[must_use]
    pub fn suggested_action_for(q: &ActiveQuest) -> Option<QuestSuggestedAction> {
        if q.can_finish_now || q.can_check_now || q.ready_to_finish {
            return None;
        }
        let pending = q
            .checkpoints
            .iter()
            .find(|cp| !cp.checked && !cp.live_ready);
        let id_low = pending.map(|cp| cp.id.to_lowercase()).unwrap_or_default();
        let text_low = format!(
            "{id_low} {} {} {}",
            pending.map(|cp| cp.title.as_str()).unwrap_or(""),
            pending.map(|cp| cp.instructions.as_str()).unwrap_or(""),
            q.next_step
        )
        .to_lowercase();

        if text_low.contains("+coop/refit") || id_low.contains("refit_dray") {
            return Some(QuestSuggestedAction {
                label: "Refit Dray-P".into(),
                command: "+coop/refit prospector".into(),
            });
        }
        if text_low.contains("+coop/enroll") || id_low.contains("enroll_coop") {
            return Some(QuestSuggestedAction {
                label: "Enroll Co-op".into(),
                command: "+coop/enroll ".into(),
            });
        }
        if id_low.contains("scan") || text_low.contains("scan go") {
            return Some(QuestSuggestedAction {
                label: "Scan Site".into(),
                command: "+tv/1 scan go".into(),
            });
        }
        if id_low.contains("sound") || text_low.contains("sound go") {
            return Some(QuestSuggestedAction {
                label: "Sound Pocket".into(),
                command: "+tv/1 sound go".into(),
            });
        }
        if id_low.starts_with("mine_") || text_low.contains("mine go") {
            return Some(QuestSuggestedAction {
                label: "Mine Ore".into(),
                command: "+tv/1 mine go".into(),
            });
        }
        let single = QuestsSnapshot {
            active: vec![q.clone()],
            ..Default::default()
        };
        if let Some(mining) = single.surface_mining_status(Some(&q.key))
            && mining.mine_done
        {
            return Some(QuestSuggestedAction {
                label: "Recover Rig".into(),
                command: "+tv/1 recover go".into(),
            });
        }
        if let Some(dest) = single.active_destination_port(Some(&q.key)) {
            return Some(QuestSuggestedAction {
                label: format!("Plan {}", dest.port_label),
                command: format!("+ship/plan {}", dest.port_id),
            });
        }
        if text_low.contains("+market/") {
            return Some(QuestSuggestedAction {
                label: "Market Board".into(),
                command: "+market/list".into(),
            });
        }
        None
    }

    /// Returns a verified `+coop/...` action when the tracked active quest asks for Co-op enrollment or Dray-P refit.
    #[must_use]
    pub fn active_coop_action(&self, tracked_key: Option<&str>) -> Option<QuestSuggestedAction> {
        for q in self.ordered_active(tracked_key) {
            if let Some(act) = Self::suggested_action_for(q)
                && act.command.starts_with("+coop/")
            {
                return Some(act);
            }
        }
        None
    }

    /// Format a live hold-progress or Step 5 empty-hold refit readiness line for an active quest
    /// inside the F11 Quest Overlay. Returns `(formatted_text, is_ready)`.
    #[must_use]
    pub fn hold_readiness_line_for(
        q: &ActiveQuest,
        stowage: &QuestStowageTelemetry,
    ) -> Option<(String, bool)> {
        if let Some(act) = Self::suggested_action_for(q)
            && act.command.starts_with("+coop/refit")
        {
            let cargo_kg = stowage.cargo_kg.max(0);
            let has_cargo_lots = stowage.cargo_lots.iter().any(|(_, _, qty)| *qty > 0);
            let cargo_empty = cargo_kg == 0 && !has_cargo_lots;
            let prov_days = stowage.provisions_days.max(0);
            let prov_empty = prov_days == 0;
            let tank_t = stowage.propellant_kg.max(0) / 1000;
            let ready = cargo_empty && prov_empty;
            let cargo_txt = if cargo_empty {
                "Cargo empty".to_string()
            } else {
                format!("Cargo {:.1}t", cargo_kg as f64 / 1000.0)
            };
            let stores_txt = if prov_empty {
                "Stores empty".to_string()
            } else {
                format!("Stores {prov_days}d")
            };
            let tag = if ready { "[READY]" } else { "[REFIT CHECK]" };
            return Some((
                format!("{tag} {cargo_txt} | {stores_txt} | Tank {tank_t}t OK"),
                ready,
            ));
        }
        let single = QuestsSnapshot {
            active: vec![q.clone()],
            ..Default::default()
        };
        let needs = single.active_trade_needs(Some(&q.key));
        if needs.is_empty() {
            let mine = single.surface_mining_status(Some(&q.key))?;
            let want = normalize_good_slug(&mine.target_ore_slug);
            let current: i64 = stowage
                .cargo_lots
                .iter()
                .filter(|(id, name, _)| {
                    normalize_good_slug(id) == want || normalize_good_slug(name) == want
                })
                .map(|(_, _, qty)| (*qty).max(0))
                .sum();
            let ready = current >= 1;
            let tag = if ready {
                "[ORE HOLD READY]"
            } else {
                "[ORE HOLD]"
            };
            let label = mine.target_ore_slug.replace('_', " ").to_ascii_uppercase();
            return Some((format!("{tag} {label}: {current}/1 t"), ready));
        }
        let eval_need = |need: &QuestTradeNeed| -> (String, i64, i64, &'static str, bool) {
            let target = need.target_quantity.max(1);
            let label = need.commodity_slug.replace('_', " ").to_ascii_uppercase();
            let (current, unit) = match need.commodity_slug.as_str() {
                "propellant" => (stowage.propellant_kg.max(0) / 1000, "t"),
                "provisions" => (stowage.provisions_days.max(0), "d"),
                slug => {
                    let sum: i64 = stowage
                        .cargo_lots
                        .iter()
                        .filter(|(id, name, _)| {
                            normalize_good_slug(id) == slug || normalize_good_slug(name) == slug
                        })
                        .map(|(_, _, qty)| (*qty).max(0))
                        .sum();
                    (sum, "u")
                }
            };
            (label, current, target, unit, current >= target)
        };
        let evaluated: Vec<_> = needs.iter().map(eval_need).collect();
        let primary_idx = evaluated
            .iter()
            .position(|(_, _, _, _, ready)| !*ready)
            .unwrap_or(0);
        let (label, current, target, unit, ready) = &evaluated[primary_idx];
        let tag = if *ready { "[HOLD READY]" } else { "[HOLD]" };
        let mut text = format!("{tag} {label}: {current}/{target} {unit}");
        if evaluated.len() > 1 {
            let extras: Vec<String> = evaluated
                .iter()
                .enumerate()
                .filter(|(idx, _)| *idx != primary_idx)
                .map(|(_, (l, c, t, u, r))| {
                    if *r {
                        format!("{l} {c}/{t}{u} OK")
                    } else {
                        format!("{l} {c}/{t}{u}")
                    }
                })
                .collect();
            if !extras.is_empty() {
                text.push_str(" | ");
                text.push_str(&extras.join(", "));
            }
        }
        Some((text, *ready))
    }

    /// Format a compact cross-port sailing pre-flight fuel & provisions readiness line for
    /// inter-station quests (Steps 3, 4, and 7) inside the F11 Quest Overlay.
    #[must_use]
    pub fn preflight_readiness_line_for(
        q: &ActiveQuest,
        stowage: &QuestStowageTelemetry,
    ) -> Option<(String, bool)> {
        let single = QuestsSnapshot {
            active: vec![q.clone()],
            ..Default::default()
        };
        let dest = single.active_destination_port(Some(&q.key))?;
        let fuel_t = stowage.propellant_kg.max(0) / 1000;
        let prov_d = stowage.provisions_days.max(0);
        let fuel_ok = fuel_t > 0;
        let prov_ok = prov_d > 0;
        let ready = fuel_ok && prov_ok;
        if ready {
            Some((
                format!(
                    "[PRE-FLIGHT OK] Fuel {fuel_t}t | Provisions {prov_d}d -> {}",
                    dest.port_label
                ),
                true,
            ))
        } else {
            let fuel_txt = if fuel_ok {
                format!("{fuel_t}t OK")
            } else {
                "0t!".to_string()
            };
            let prov_txt = if prov_ok {
                format!("{prov_d}d OK")
            } else {
                "0d!".to_string()
            };
            Some((
                format!(
                    "[CHECK BEFORE UNDOCK] Fuel {fuel_txt} | Provisions {prov_txt} -> {}",
                    dest.port_label
                ),
                false,
            ))
        }
    }

    /// Order room NPC quest markers by action urgency:
    /// gold turn-in (`?`) -> gold available offer (`!`) -> silver in-progress (`?`).
    #[must_use]
    pub fn prioritized_room_markers(&self) -> Vec<&QuestMarker> {
        let rank = |m: &QuestMarker| match m.marker.as_str() {
            "turn_in" => 0_u8,
            "available" => 1_u8,
            _ => 2_u8,
        };
        let mut out: Vec<&QuestMarker> = self.room_markers.iter().collect();
        out.sort_by_key(|m| rank(m));
        out
    }

    /// Pick the highest-priority available quest offer in the current room, preferring
    /// Guided Captain's Tour steps ahead of optional side offers.
    #[must_use]
    pub fn prioritized_available_offer(&self) -> Option<&AvailableQuest> {
        self.available
            .iter()
            .find(|a| {
                a.category.eq_ignore_ascii_case("guided")
                    || GUIDED_TOUR_KEYS.contains(&a.key.as_str())
            })
            .or_else(|| self.available.first())
    }

    /// Concise persistent status-bar badge when a quest can be turned in or accepted
    /// right here in the player's current room (`[?] Turn-In (F11)` / `[!] Offer (F11)`).
    #[must_use]
    pub fn status_bar_quest_badge(&self) -> Option<(String, Color)> {
        let prioritized = self.prioritized_room_markers();
        if let Some(m) = prioritized.iter().find(|m| m.marker == "turn_in") {
            let text = if m.npc_name.is_empty() {
                "[?] Turn-In (F11)".to_string()
            } else {
                format!("[?] {}: Turn-In (F11)", m.npc_name)
            };
            return Some((text, Color::Green));
        }
        if let Some(q) = self.active.iter().find(|q| q.can_finish_now) {
            let text = if q.turn_in_name.is_empty() {
                "[?] Turn-In (F11)".to_string()
            } else {
                format!("[?] {}: Turn-In (F11)", q.turn_in_name)
            };
            return Some((text, Color::Green));
        }
        if self.available.len() > 1 {
            return Some((
                format!("[!] {} offers (F11)", self.available.len()),
                Color::Yellow,
            ));
        }
        if let Some(m) = prioritized.iter().find(|m| m.marker == "available") {
            let text = if m.npc_name.is_empty() {
                "[!] Offer (F11)".to_string()
            } else {
                format!("[!] {}: Offer (F11)", m.npc_name)
            };
            return Some((text, Color::Yellow));
        }
        if let Some(offer) = self.prioritized_available_offer() {
            let text = if offer.giver_name.is_empty() {
                "[!] Offer (F11)".to_string()
            } else {
                format!("[!] {}: Offer (F11)", offer.giver_name)
            };
            return Some((text, Color::Yellow));
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
            room_entry: str_field(cp, "room_entry"),
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

fn marker_priority(marker: &str) -> u8 {
    match marker {
        "turn_in" => 0,
        "available" => 1,
        _ => 2,
    }
}

/// Return a compact colored badge (`" [!]"`, `" [?]"`) for an NPC listed in `RoomView`,
/// prioritizing `turn_in` -> `available` -> `in_progress` when multiple markers match.
#[must_use]
pub fn marker_badge_for_npc(view: &RoomView, npc_id: &str) -> Option<(&'static str, Style)> {
    let markers = parse_room_markers(view);
    let found = markers
        .iter()
        .filter(|m| m.npc_id == npc_id)
        .min_by_key(|m| marker_priority(&m.marker))?;
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

/// Return a compact colored badge for an NPC listed in `RoomView`, prioritizing
/// `turn_in` -> `available` -> `in_progress` and appending ` xN` when multiple quests apply.
#[must_use]
pub fn marker_badge_with_count_for_npc(view: &RoomView, npc_id: &str) -> Option<(String, Style)> {
    let markers = parse_room_markers(view);
    let mut best: Option<&QuestMarker> = None;
    let mut keys: Vec<String> = Vec::new();
    for m in &markers {
        if m.npc_id != npc_id {
            continue;
        }
        let k = if m.quest_key.is_empty() {
            format!("{}:{}", m.marker, m.quest_title)
        } else {
            m.quest_key.clone()
        };
        if !keys.contains(&k) {
            keys.push(k);
        }
        if best.is_none_or(|cur| marker_priority(&m.marker) < marker_priority(&cur.marker)) {
            best = Some(m);
        }
    }
    let found = best?;
    let count = keys.len();
    let count_suffix = if count > 1 {
        format!(" x{count}")
    } else {
        String::new()
    };
    match found.marker.as_str() {
        "turn_in" => Some((
            format!(" [?{count_suffix}]"),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        "available" => Some((
            format!(" [!{count_suffix}]"),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        "in_progress" => Some((
            format!(" [?{count_suffix}]"),
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
                        turn_in_here: bool_field(item, "turn_in_here"),
                        allow_restart: bool_field(item, "allow_restart"),
                        ready_to_finish: bool_field(item, "ready_to_finish"),
                        next_step: str_field(item, "next_step"),
                        next_room_entry: str_field(item, "next_room_entry"),
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
    Prefill(String),
    Submit(String),
}

#[derive(Clone, Debug, Default)]
pub struct QuestOverlayState {
    pub open: bool,
    pub tab: QuestTab,
    pub selected: usize,
    pub tracked_key: Option<String>,
    pub confirm_abandon: bool,
    pub confirm_decline: bool,
}

impl QuestOverlayState {
    pub fn open_for(&mut self, snap: &QuestsSnapshot) {
        self.open = true;
        self.confirm_abandon = false;
        self.confirm_decline = false;
        self.tab = if !snap.available.is_empty() && snap.active.is_empty() {
            QuestTab::Available
        } else if !snap.active.is_empty() {
            QuestTab::Active
        } else if !snap.available.is_empty() {
            QuestTab::Available
        } else {
            QuestTab::Completed
        };
        self.selected = match self.tab {
            QuestTab::Active => snap
                .active
                .iter()
                .position(|q| q.can_finish_now)
                .or_else(|| {
                    snap.active
                        .iter()
                        .position(|q| q.can_check_now || q.ready_to_finish)
                })
                .or_else(|| {
                    self.tracked_key
                        .as_deref()
                        .and_then(|tk| snap.active.iter().position(|q| q.key == tk))
                })
                .unwrap_or(0),
            QuestTab::Available => snap
                .available
                .iter()
                .position(|q| {
                    q.category.eq_ignore_ascii_case("guided")
                        || GUIDED_TOUR_KEYS.contains(&q.key.as_str())
                })
                .unwrap_or(0),
            QuestTab::Completed => 0,
        };
    }

    /// Open the quest dialog directly on a specific tab and list index (used by Command Palette).
    pub fn open_tab_index(&mut self, tab: QuestTab, index: usize) {
        self.open = true;
        self.confirm_abandon = false;
        self.confirm_decline = false;
        self.tab = tab;
        self.selected = index;
    }

    /// Open the quest dialog focused directly on the highest-priority quest offered by
    /// or turn-in ready at `npc_id` / `npc_name` (`turn_in` -> `available` -> `in_progress`).
    pub fn open_for_npc(&mut self, snap: &QuestsSnapshot, npc_id: &str, npc_name: &str) {
        self.open = true;
        self.confirm_abandon = false;
        self.confirm_decline = false;

        let prioritized = snap.prioritized_room_markers();
        let marker = prioritized.into_iter().find(|m| {
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
            if let Some(idx) = snap.active.iter().position(|q| {
                (q.can_finish_now || q.ready_to_finish || q.can_check_now)
                    && (q.turn_in_name.eq_ignore_ascii_case(npc_name)
                        || q.giver_name.eq_ignore_ascii_case(npc_name))
            }) {
                self.tab = QuestTab::Active;
                self.selected = idx;
                return;
            }
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
        self.confirm_decline = false;
    }

    /// Ensure `tracked_key` points at an active quest, preferring the active Guided Tour
    /// quest whenever no manual selection is active (or after the tracked quest completes).
    pub fn sync_tracked_for(&mut self, snap: &QuestsSnapshot) {
        let still_active = self
            .tracked_key
            .as_deref()
            .is_some_and(|k| snap.active.iter().any(|q| q.key == k));
        if !still_active {
            self.tracked_key = snap.default_tracked_key().map(ToOwned::to_owned);
        }
    }

    /// Cycle the HUD-tracked active quest across `snap.active` in a single action.
    pub fn cycle_tracked(&mut self, snap: &QuestsSnapshot) -> Option<&str> {
        if snap.active.is_empty() {
            self.tracked_key = None;
            return None;
        }
        if snap.active.len() == 1 {
            self.tracked_key = Some(snap.active[0].key.clone());
            return self.tracked_key.as_deref();
        }
        let cur = self
            .tracked_key
            .as_deref()
            .and_then(|k| snap.active.iter().position(|q| q.key == k));
        let next_idx = cur.map_or(1, |i| (i + 1) % snap.active.len());
        self.tracked_key = Some(snap.active[next_idx].key.clone());
        self.tracked_key.as_deref()
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
        self.handle_key_with_port(key, snap, None)
    }

    pub fn handle_key_with_port(
        &mut self,
        key: Key,
        snap: &QuestsSnapshot,
        current_port: Option<&str>,
    ) -> QuestOverlayAction {
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
                self.confirm_decline = false;
                QuestOverlayAction::None
            }
            Key::BackTab | Key::Left => {
                self.tab = self.tab.prev();
                self.selected = 0;
                self.confirm_abandon = false;
                self.confirm_decline = false;
                QuestOverlayAction::None
            }
            Key::Up | Key::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                self.confirm_abandon = false;
                self.confirm_decline = false;
                QuestOverlayAction::None
            }
            Key::Down | Key::Char('j') => {
                let len = self.tab_len(snap);
                if len > 0 && self.selected + 1 < len {
                    self.selected += 1;
                }
                self.confirm_abandon = false;
                self.confirm_decline = false;
                QuestOverlayAction::None
            }
            Key::Char('a') | Key::Enter if self.tab == QuestTab::Available => {
                self.confirm_abandon = false;
                self.confirm_decline = false;
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
            Key::Char('d') if self.tab == QuestTab::Available => {
                self.confirm_abandon = false;
                if let Some(offer) = snap.available.get(self.selected)
                    && !offer.allow_restart
                    && !offer.decline_cmd.is_empty()
                {
                    if self.confirm_decline {
                        self.confirm_decline = false;
                        return QuestOverlayAction::Submit(offer.decline_cmd.clone());
                    }
                    self.confirm_decline = true;
                } else {
                    self.confirm_decline = false;
                }
                QuestOverlayAction::None
            }
            Key::Enter | Key::Char('c') | Key::Char('f') if self.tab == QuestTab::Active => {
                self.confirm_abandon = false;
                self.confirm_decline = false;
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
                self.confirm_decline = false;
                if let Some(item) = snap.active.get(self.selected) {
                    self.tracked_key = Some(item.key.clone());
                }
                QuestOverlayAction::None
            }
            Key::Char('p') if self.tab == QuestTab::Active => {
                self.confirm_abandon = false;
                self.confirm_decline = false;
                if let Some(item) = snap.active.get(self.selected)
                    && let Some(act) = QuestsSnapshot::suggested_action_for(item)
                {
                    self.close();
                    return QuestOverlayAction::Prefill(act.command);
                }
                if snap.active.is_empty()
                    && current_port.is_some()
                    && let Some(dest) = snap.next_guided_offer_destination(current_port)
                {
                    self.close();
                    return QuestOverlayAction::Prefill(format!("+ship/plan {}", dest.port_id));
                }
                if snap.active.is_empty() && snap.guided_tour_progress().is_some_and(|t| t.all_done)
                {
                    self.close();
                    return QuestOverlayAction::Prefill("+market/jobs".into());
                }
                QuestOverlayAction::None
            }
            Key::Char('x') if self.tab == QuestTab::Active => {
                self.confirm_decline = false;
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
                self.confirm_decline = false;
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
    render_quest_tracker_lines_with_route(snap, tracked_key, None)
}

/// Build compact tracker lines for the Scene screen right rail, including the
/// 7-step Guided Tour progress bar, next-hop station route breadcrumb, and
/// contextual F11 action cue.
#[must_use]
pub fn render_quest_tracker_lines_with_route(
    snap: &QuestsSnapshot,
    tracked_key: Option<&str>,
    route_hop: Option<&QuestExitBreadcrumb>,
) -> Vec<Line<'static>> {
    render_quest_tracker_lines_with_route_and_stowage(snap, tracked_key, route_hop, None)
}

/// Build compact tracker lines for the Scene screen right rail, including live
/// ship hold & Step 5 refit readiness telemetry alongside route breadcrumbs.
#[must_use]
pub fn render_quest_tracker_lines_with_route_and_stowage(
    snap: &QuestsSnapshot,
    tracked_key: Option<&str>,
    route_hop: Option<&QuestExitBreadcrumb>,
    stowage: Option<&QuestStowageTelemetry>,
) -> Vec<Line<'static>> {
    render_quest_tracker_lines_full(snap, tracked_key, route_hop, stowage, None)
}

/// Build compact tracker lines for the Scene screen right rail, including live
/// ship hold readiness, secondary room turn-in alerts, guided offer priority,
/// and off-port sail hints for between-steps Guided Tour offers.
#[must_use]
pub fn render_quest_tracker_lines_full(
    snap: &QuestsSnapshot,
    tracked_key: Option<&str>,
    route_hop: Option<&QuestExitBreadcrumb>,
    stowage: Option<&QuestStowageTelemetry>,
    current_port: Option<&str>,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let tour = snap.guided_tour_progress();
    let tracked = snap.ordered_active(tracked_key).into_iter().next();

    if let Some(q) = tracked {
        if let Some(ref t) = tour {
            let text = if t.all_done {
                format!(
                    "Corridor Captain {} {}/{}",
                    t.progress_bar(),
                    t.completed,
                    t.total
                )
            } else {
                format!("Tour {} {}/{}", t.progress_bar(), t.completed, t.total)
            };
            let col = if t.all_done {
                Color::Green
            } else {
                Color::Yellow
            };
            lines.push(Line::from(Span::styled(
                text,
                Style::default().fg(col).add_modifier(Modifier::BOLD),
            )));
        }
        let done = q.checkpoints.iter().filter(|c| c.checked).count();
        let sym = if q.can_finish_now || q.ready_to_finish {
            "[?]"
        } else {
            "[*]"
        };
        let multi_tag = if snap.active.len() > 1 {
            let pos = snap
                .active
                .iter()
                .position(|item| item.key == q.key)
                .unwrap_or(0);
            format!(" [{}/{}]", pos + 1, snap.active.len())
        } else {
            String::new()
        };
        let step_badge = guided_step_badge(&q.key);
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
                format!(" ({done}/{}){step_badge}{multi_tag}", q.checkpoints.len()),
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
        if let Some(st) = stowage
            && let Some((hold_txt, hold_ready)) = QuestsSnapshot::hold_readiness_line_for(q, st)
        {
            let style = if hold_ready {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Yellow)
            };
            lines.push(Line::from(Span::styled(hold_txt, style)));
        }
        if let Some(st) = stowage
            && let Some((pf_txt, pf_ready)) = QuestsSnapshot::preflight_readiness_line_for(q, st)
        {
            let style = if pf_ready {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(Color::LightYellow)
                    .add_modifier(Modifier::BOLD)
            };
            lines.push(Line::from(Span::styled(pf_txt, style)));
        }
        if !q.can_finish_now
            && !q.ready_to_finish
            && let Some(mine) = snap
                .tracked_only(tracked_key)
                .and_then(|s| s.surface_mining_status(Some(q.key.as_str())))
        {
            lines.push(Line::from(Span::styled(
                mine.prep_hint_line(),
                Style::default().fg(Color::Cyan),
            )));
        }
        if !q.can_finish_now
            && !q.can_check_now
            && let Some(hop) = route_hop
        {
            let route_text = if hop.next_exit_command == "+ship/board" {
                "+ship/board (Board Ship)".to_string()
            } else if hop.next_exit_command == "+ship/leave" {
                "+ship/leave (Cycle Ashore)".to_string()
            } else if hop.next_exit_command == "+tv/bay in" || hop.next_exit_command == "+tv/bay" {
                format!("{} (Vehicle Bay)", hop.next_exit_command)
            } else {
                format!("-> {} ({}h)", hop.next_room_name, hop.hops)
            };
            lines.push(Line::from(vec![
                Span::styled(
                    "Route: ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(route_text, Style::default().fg(Color::Yellow)),
            ]));
        }
        if !q.next_step.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("Next: ", Style::default().fg(Color::Cyan)),
                Span::raw(q.next_step.clone()),
            ]));
        }
        let other_ready_here: Vec<&ActiveQuest> = snap
            .active
            .iter()
            .filter(|item| item.key != q.key && item.can_finish_now)
            .collect();
        if !other_ready_here.is_empty() {
            let ready_txt = if other_ready_here.len() == 1 {
                format!("[?] Ready here: {} (F11)", other_ready_here[0].title)
            } else {
                format!("[?] {} ready to turn in here (F11)", other_ready_here.len())
            };
            lines.push(Line::from(Span::styled(
                ready_txt,
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )));
        }
        if !snap.available.is_empty() {
            let offer_txt = if snap.available.len() == 1 {
                "[!] 1 offer here (F11 -> a)".to_string()
            } else {
                format!("[!] {} offers here (F11 -> a)", snap.available.len())
            };
            lines.push(Line::from(Span::styled(
                offer_txt,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )));
        }
        let hint_line = if q.can_finish_now {
            Line::from(Span::styled(
                "F11 -> Enter: Complete [?]",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ))
        } else if q.can_check_now {
            Line::from(Span::styled(
                "F11 -> Enter: Verify Step",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ))
        } else if let Some(sug) = QuestsSnapshot::suggested_action_for(q) {
            Line::from(Span::styled(
                format!("F11 -> p: {}", sug.command),
                Style::default().fg(Color::Cyan),
            ))
        } else {
            Line::from(Span::styled(
                "F11 or /quests for dialog",
                Style::default().add_modifier(Modifier::DIM),
            ))
        };
        lines.push(hint_line);
    } else if let Some(offer) = snap.prioritized_available_offer() {
        if let Some(ref t) = tour {
            lines.push(Line::from(Span::styled(
                format!("Tour {} {}/{}", t.progress_bar(), t.completed, t.total),
                Style::default().fg(Color::Yellow),
            )));
        }
        lines.push(Line::from(vec![
            Span::styled(
                "[!] ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(offer.giver_name.clone(), Style::default().fg(Color::Yellow)),
        ]));
        let step_badge = guided_step_badge(&offer.key);
        let offer_title = if snap.available.len() > 1 {
            format!(
                "{} (+{} more){step_badge}",
                offer.title,
                snap.available.len() - 1
            )
        } else {
            format!("{}{step_badge}", offer.title)
        };
        lines.push(Line::from(offer_title));
        lines.push(Line::from(Span::styled(
            "F11 -> a: Accept Quest",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )));
    } else if let Some(next) = snap.next_guided_offer() {
        if let Some(ref t) = tour {
            lines.push(Line::from(Span::styled(
                format!("Tour {} {}/{}", t.progress_bar(), t.completed, t.total),
                Style::default().fg(Color::Yellow),
            )));
        }
        lines.push(Line::from(vec![
            Span::styled(
                "[!] ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("Next ({}/7): {}", next.step, next.short_label),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Speak: ", Style::default().fg(Color::Cyan)),
            Span::raw(format!("{} ({})", next.giver_label, next.giver_room_name)),
        ]));
        if let Some(hop) = route_hop {
            let route_text = if hop.next_exit_command == "+ship/board" {
                "+ship/board (Board Ship)".to_string()
            } else if hop.next_exit_command == "+ship/leave" {
                "+ship/leave (Cycle Ashore)".to_string()
            } else {
                format!("-> {} ({}h)", hop.next_room_name, hop.hops)
            };
            lines.push(Line::from(vec![
                Span::styled(
                    "Route: ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(route_text, Style::default().fg(Color::Yellow)),
            ]));
        }
        if current_port.is_some()
            && let Some(dest) = snap.next_guided_offer_destination(current_port)
        {
            lines.push(Line::from(vec![
                Span::styled(
                    "Sail: ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("+ship/plan {} ({})", dest.port_id, dest.port_label),
                    Style::default().fg(Color::Cyan),
                ),
            ]));
        }
    } else if let Some(t) = tour
        && t.all_done
    {
        lines.push(Line::from(Span::styled(
            format!("[*] Corridor Captain ({}/{})", t.completed, t.total),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            "Guided Tour Complete",
            Style::default().fg(Color::Green),
        )));
        lines.push(Line::from(Span::styled(
            "Jobs: +market/jobs",
            Style::default().fg(Color::Cyan),
        )));
    }
    lines
}

/// Build golden Guided Quests summary lines for the TUI Character Sheet screen (`2 sheet`).
#[must_use]
pub fn render_quest_sheet_lines(
    snap: &QuestsSnapshot,
    tracked_key: Option<&str>,
) -> Vec<Line<'static>> {
    render_quest_sheet_lines_with_stowage(snap, tracked_key, None)
}

/// Build golden Guided Quests summary lines for the TUI Character Sheet screen (`2 sheet`),
/// including live ship hold / Step 5 refit / cross-port pre-flight readiness and the
/// post-graduation `Corridor Captain (7/7)` Jobs Board pointer.
#[must_use]
pub fn render_quest_sheet_lines_with_stowage(
    snap: &QuestsSnapshot,
    tracked_key: Option<&str>,
    stowage: Option<&QuestStowageTelemetry>,
) -> Vec<Line<'static>> {
    if snap.active.is_empty() && snap.available.is_empty() && snap.completed.is_empty() {
        return Vec::new();
    }
    let header = if let Some(tour) = snap.guided_tour_progress() {
        if tour.all_done {
            format!(
                "Guided Quests  —  Corridor Captain {} {}/{}",
                tour.progress_bar(),
                tour.completed,
                tour.total
            )
        } else {
            format!(
                "Guided Quests  —  Tour {} {}/{}",
                tour.progress_bar(),
                tour.completed,
                tour.total
            )
        }
    } else {
        "Guided Quests".to_string()
    };
    let mut lines: Vec<Line<'static>> = vec![
        Line::from(""),
        Line::from(Span::styled(
            header,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
    ];
    if let Some(mut roadmap_line) = snap.render_guided_tour_roadmap_line() {
        roadmap_line.spans.insert(0, Span::raw("  "));
        lines.push(roadmap_line);
    }
    if let Some(q) = snap.ordered_active(tracked_key).into_iter().next() {
        let done = q.checkpoints.iter().filter(|c| c.checked).count();
        let step_badge = guided_step_badge(&q.key);
        let ready = if q.ready_to_finish || q.can_finish_now {
            " [READY]"
        } else {
            ""
        };
        lines.push(Line::from(vec![
            Span::styled("  Active: ", Style::default().fg(Color::Cyan)),
            Span::styled(
                format!(
                    "{}{step_badge} ({done}/{}){ready}",
                    q.title,
                    q.checkpoints.len()
                ),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        if !q.next_step.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("  Next: ", Style::default().fg(Color::Cyan)),
                Span::raw(q.next_step.clone()),
            ]));
        }
        if let Some(st) = stowage
            && let Some((hold_txt, hold_ready)) = QuestsSnapshot::hold_readiness_line_for(q, st)
        {
            let style = if hold_ready {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Yellow)
            };
            lines.push(Line::from(Span::styled(format!("  {hold_txt}"), style)));
        }
        if let Some(st) = stowage
            && let Some((pf_txt, pf_ready)) = QuestsSnapshot::preflight_readiness_line_for(q, st)
        {
            let style = if pf_ready {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(Color::LightYellow)
                    .add_modifier(Modifier::BOLD)
            };
            lines.push(Line::from(Span::styled(format!("  {pf_txt}"), style)));
        }
        let other_ready_here: Vec<&ActiveQuest> = snap
            .active
            .iter()
            .filter(|item| item.key != q.key && item.can_finish_now)
            .collect();
        if !other_ready_here.is_empty() {
            let ready_txt = if other_ready_here.len() == 1 {
                format!("  [?] Ready here: {} (F11)", other_ready_here[0].title)
            } else {
                format!(
                    "  [?] {} ready to turn in here (F11)",
                    other_ready_here.len()
                )
            };
            lines.push(Line::from(Span::styled(
                ready_txt,
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )));
        }
        if !snap.available.is_empty() {
            let offer_txt = if snap.available.len() == 1 {
                "  [!] 1 offer here (F11 -> a)".to_string()
            } else {
                format!("  [!] {} offers here (F11 -> a)", snap.available.len())
            };
            lines.push(Line::from(Span::styled(
                offer_txt,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )));
        }
    } else if let Some(offer) = snap.prioritized_available_offer() {
        let extra = if snap.available.len() > 1 {
            format!(" (+{} more)", snap.available.len() - 1)
        } else {
            String::new()
        };
        let step_badge = guided_step_badge(&offer.key);
        lines.push(Line::from(vec![
            Span::styled(
                "  Available: [!] ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "{} ({}){extra}{step_badge}",
                offer.title, offer.giver_name
            )),
        ]));
    } else if let Some(next) = snap.next_guided_offer() {
        lines.push(Line::from(vec![
            Span::styled(
                format!("  Next Guided ({}/7): ", next.step),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "{} — speak with {} ({})",
                next.short_label, next.giver_label, next.giver_room_name
            )),
        ]));
    } else if let Some(tour) = snap.guided_tour_progress()
        && tour.all_done
    {
        lines.push(Line::from(vec![
            Span::styled(
                format!(
                    "  Guided Tour Complete ({}/{}) — ",
                    tour.completed, tour.total
                ),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Jobs Board: +market/jobs", Style::default().fg(Color::Cyan)),
        ]));
    }
    lines.push(Line::from(vec![
        Span::styled("  Completed: ", Style::default().fg(Color::Green)),
        Span::raw(format!("{}  |  ", snap.completed.len())),
        Span::styled(
            "F11 or /quests for Quest Log",
            Style::default().add_modifier(Modifier::DIM),
        ),
    ]));
    lines
}

/// Paint the centered Quest Dialog & Journal overlay.
pub fn render_quest_overlay(
    f: &mut Frame<'_>,
    area: Rect,
    state: &QuestOverlayState,
    snap: &QuestsSnapshot,
) {
    render_quest_overlay_with_stowage(f, area, state, snap, None);
}

pub(crate) fn guided_step_badge(key: &str) -> String {
    GUIDED_TOUR_KEYS
        .iter()
        .position(|&k| k == key)
        .map(|i| format!(" [Step {}/7]", i + 1))
        .unwrap_or_default()
}

pub(crate) fn is_guided_quest(key: &str, category: &str) -> bool {
    category.eq_ignore_ascii_case("guided") || GUIDED_TOUR_KEYS.contains(&key)
}

/// Paint the centered Quest Dialog & Journal overlay with live ship stowage & refit hold telemetry.
pub fn render_quest_overlay_with_stowage(
    f: &mut Frame<'_>,
    area: Rect,
    state: &QuestOverlayState,
    snap: &QuestsSnapshot,
    stowage: Option<&QuestStowageTelemetry>,
) {
    render_quest_overlay_full(f, area, state, snap, stowage, None);
}

/// Paint the centered Quest Dialog & Journal overlay with live ship stowage, refit hold telemetry,
/// and current corridor port context for between-steps off-port Sail shortcuts.
pub fn render_quest_overlay_full(
    f: &mut Frame<'_>,
    area: Rect,
    state: &QuestOverlayState,
    snap: &QuestsSnapshot,
    stowage: Option<&QuestStowageTelemetry>,
    current_port: Option<&str>,
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
    let mut tab_spans = vec![
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
    ];
    if let Some(tour) = snap.guided_tour_progress() {
        let label = if tour.all_done {
            format!(
                "  Corridor Captain {} {}/{}",
                tour.progress_bar(),
                tour.completed,
                tour.total
            )
        } else {
            format!(
                "  Tour {} {}/{}",
                tour.progress_bar(),
                tour.completed,
                tour.total
            )
        };
        tab_spans.push(Span::styled(
            label,
            Style::default()
                .fg(if tour.all_done {
                    Color::Green
                } else {
                    Color::Yellow
                })
                .add_modifier(Modifier::BOLD),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(tab_spans)), chunks[0]);

    let body_area = if let Some(roadmap_line) = snap.render_guided_tour_roadmap_line()
        && chunks[1].height >= 9
    {
        let body_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(5)])
            .split(chunks[1]);
        f.render_widget(
            Paragraph::new(roadmap_line).wrap(Wrap { trim: true }),
            body_chunks[0],
        );
        body_chunks[1]
    } else {
        chunks[1]
    };

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(36), Constraint::Percentage(64)])
        .split(body_area);

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
                let step_tag = guided_step_badge(&q.key);
                let style = if selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(if q.can_finish_now {
                            Color::Green
                        } else {
                            Color::Yellow
                        })
                        .add_modifier(Modifier::BOLD)
                } else if q.can_finish_now {
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                list_lines.push(Line::from(Span::styled(
                    format!("{star}{badge} {}{step_tag}", q.title),
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
                let step_tag = guided_step_badge(&q.key);
                let style = if selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Yellow)
                };
                list_lines.push(Line::from(Span::styled(
                    format!(" [!] {}{step_tag}", q.title),
                    style,
                )));
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
                let step_tag = guided_step_badge(&q.key);
                let style = if selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Green)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Green)
                };
                list_lines.push(Line::from(Span::styled(
                    format!(" [x] {}{step_tag}", q.title),
                    style,
                )));
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
                let step_tag = guided_step_badge(&q.key);
                detail_lines.push(Line::from(Span::styled(
                    format!("{}{step_tag}", q.title),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )));
                if q.can_finish_now {
                    detail_lines.push(Line::from(Span::styled(
                        "[?] Ready to complete here (Enter/c/f)",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    )));
                }
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
                if q.allow_restart {
                    detail_lines.push(Line::from(Span::styled(
                        "Optional · Restartable (abandon & restart any time without penalty)",
                        Style::default().add_modifier(Modifier::DIM),
                    )));
                }
                if let Some(act) = QuestsSnapshot::suggested_action_for(q) {
                    detail_lines.push(Line::from(vec![
                        Span::styled(
                            "Suggested [p]: ",
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(act.command, Style::default().fg(Color::Green)),
                        Span::styled(
                            format!("  ({})", act.label),
                            Style::default().add_modifier(Modifier::DIM),
                        ),
                    ]));
                }
                if let Some(st) = stowage
                    && let Some((hold_line, ready)) = QuestsSnapshot::hold_readiness_line_for(q, st)
                {
                    detail_lines.push(Line::from(Span::styled(
                        hold_line,
                        Style::default()
                            .fg(if ready { Color::Green } else { Color::Yellow })
                            .add_modifier(Modifier::BOLD),
                    )));
                }
                if let Some(st) = stowage
                    && let Some((pf_line, pf_ready)) =
                        QuestsSnapshot::preflight_readiness_line_for(q, st)
                {
                    detail_lines.push(Line::from(Span::styled(
                        pf_line,
                        Style::default()
                            .fg(if pf_ready {
                                Color::Green
                            } else {
                                Color::LightYellow
                            })
                            .add_modifier(Modifier::BOLD),
                    )));
                }
                let single_for_mine = QuestsSnapshot {
                    active: vec![q.clone()],
                    ..Default::default()
                };
                if let Some(mine) = single_for_mine.surface_mining_status(Some(&q.key)) {
                    detail_lines.push(Line::from(vec![
                        Span::styled(
                            "[PROSPECTING] ",
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(mine.prep_hint_line(), Style::default().fg(Color::Cyan)),
                    ]));
                }
                if state.confirm_abandon {
                    detail_lines.push(Line::from(Span::styled(
                        "Press 'x' again to confirm abandoning this quest.",
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    )));
                }
            } else if let Some(next) = snap.next_guided_offer() {
                detail_lines.push(Line::from(Span::styled(
                    format!("Next Guided Step ({}/7): {}", next.step, next.short_label),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )));
                detail_lines.push(Line::from(Span::styled(
                    format!(
                        "Speak with: {} ({})",
                        next.giver_label, next.giver_room_name
                    ),
                    Style::default().fg(Color::Cyan),
                )));
                detail_lines.push(Line::from(Span::styled(
                    format!("Route: {}", next.location_label),
                    Style::default().add_modifier(Modifier::DIM),
                )));
                if current_port.is_some()
                    && let Some(dest) = snap.next_guided_offer_destination(current_port)
                {
                    detail_lines.push(Line::from(vec![
                        Span::styled(
                            "Sail [p]: ",
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("+ship/plan {} ({})", dest.port_id, dest.port_label),
                            Style::default().fg(Color::Green),
                        ),
                    ]));
                }
                if !snap.available.is_empty() {
                    let plural = if snap.available.len() == 1 {
                        "offer"
                    } else {
                        "offers"
                    };
                    detail_lines.push(Line::from(Span::styled(
                        format!(
                            "[!] {} quest {plural} here (Tab -> Available)",
                            snap.available.len()
                        ),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    )));
                }
            } else if snap.guided_tour_progress().is_some_and(|t| t.all_done) {
                detail_lines.push(Line::from(Span::styled(
                    "* Corridor Captain (7/7) *",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )));
                detail_lines.push(Line::from(Span::styled(
                    "All 7 guided tour steps complete.",
                    Style::default().fg(Color::Cyan),
                )));
                detail_lines.push(Line::from(vec![
                    Span::styled(
                        "Jobs [p]: ",
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("+market/jobs", Style::default().fg(Color::Green)),
                ]));
                detail_lines.push(Line::from(Span::styled(
                    format!("Log: Tab -> Completed ({})", snap.completed.len()),
                    Style::default().add_modifier(Modifier::DIM),
                )));
                if !snap.available.is_empty() {
                    let plural = if snap.available.len() == 1 {
                        "offer"
                    } else {
                        "offers"
                    };
                    detail_lines.push(Line::from(Span::styled(
                        format!(
                            "[!] {} quest {plural} here (Tab -> Available)",
                            snap.available.len()
                        ),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    )));
                }
            } else if !snap.available.is_empty() {
                let plural = if snap.available.len() == 1 {
                    "offer"
                } else {
                    "offers"
                };
                detail_lines.push(Line::from(Span::styled(
                    format!(
                        "[!] {} quest {plural} here (Tab -> Available)",
                        snap.available.len()
                    ),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )));
            }
        }
        QuestTab::Available => {
            if let Some(q) = snap.available.get(state.selected) {
                let step_tag = guided_step_badge(&q.key);
                detail_lines.push(Line::from(Span::styled(
                    format!("[!] {}{step_tag}", q.title),
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
                if q.allow_restart {
                    detail_lines.push(Line::from(Span::styled(
                        "Optional · Restartable (abandon & restart any time without penalty)",
                        Style::default().add_modifier(Modifier::DIM),
                    )));
                } else if !q.decline_cmd.is_empty() && state.confirm_decline {
                    detail_lines.push(Line::from(Span::styled(
                        "Press 'd' again to permanently decline this optional quest.",
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    )));
                }
            }
        }
        QuestTab::Completed => {
            if snap.guided_tour_progress().is_some_and(|t| t.all_done) {
                detail_lines.push(Line::from(Span::styled(
                    "* Guided Corridor Tour Complete (7/7) — Corridor Captain *",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )));
                detail_lines.push(Line::from(""));
            }
            if let Some(q) = snap.completed.get(state.selected) {
                let step_tag = guided_step_badge(&q.key);
                detail_lines.push(Line::from(Span::styled(
                    format!("[x] {}{step_tag}", q.title),
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
    let can_decline_selected = state.tab == QuestTab::Available
        && snap
            .available
            .get(state.selected)
            .is_some_and(|q| !q.allow_restart && !q.decline_cmd.is_empty());
    let footer = match state.tab {
        QuestTab::Active => {
            "Enter/c/f: verify/complete  |  p: prefill cmd  |  t: track  |  x: abandon  |  Tab: tab  |  Esc: close"
        }
        QuestTab::Available if can_decline_selected => {
            "Enter/a: accept quest  |  d: decline permanently  |  Tab: switch tab  |  Esc: close"
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

    #[test]
    fn milestone_tracker_suppresses_initial_hydration_and_emits_genuine_transitions() {
        let mut tracker = QuestMilestoneTracker::default();
        let initial = parse_frame(&json!({
            "schema": 1,
            "active": [{ "key": "q1", "title": "First Papers", "ready_to_finish": false }],
            "completed": [{ "key": "q0", "title": "Arrival" }]
        }));
        assert!(
            tracker.observe(&initial).is_empty(),
            "initial login/reconnect hydration must not spam milestone notices"
        );

        let accepted_and_ready = parse_frame(&json!({
            "schema": 1,
            "active": [
                { "key": "q1", "title": "First Papers", "ready_to_finish": true },
                { "key": "q2", "title": "Stock the Locker", "ready_to_finish": false }
            ],
            "completed": [{ "key": "q0", "title": "Arrival" }]
        }));
        let m1 = tracker.observe(&accepted_and_ready);
        assert_eq!(
            m1,
            vec![
                QuestMilestone {
                    kind: QuestMilestoneKind::Ready,
                    title: "First Papers".into(),
                    detail: None,
                    tour_all_done: false,
                    restartable: false,
                },
                QuestMilestone {
                    kind: QuestMilestoneKind::Accepted,
                    title: "Stock the Locker".into(),
                    detail: None,
                    tour_all_done: false,
                    restartable: false,
                },
            ]
        );
        assert_eq!(
            m1[0].notice_text(),
            "[QUEST] Ready to turn in: First Papers"
        );
        assert_eq!(m1[1].notice_text(), "[QUEST] Accepted: Stock the Locker");

        let completed = parse_frame(&json!({
            "schema": 1,
            "active": [{ "key": "q2", "title": "Stock the Locker", "ready_to_finish": false }],
            "completed": [
                { "key": "q0", "title": "Arrival" },
                { "key": "q1", "title": "First Papers" }
            ]
        }));
        let m2 = tracker.observe(&completed);
        assert_eq!(
            m2,
            vec![QuestMilestone {
                kind: QuestMilestoneKind::Completed,
                title: "First Papers".into(),
                detail: None,
                tour_all_done: false,
                restartable: false,
            }]
        );
        assert_eq!(m2[0].notice_text(), "[QUEST] Completed: First Papers");

        // Resetting suppresses next hydration again
        tracker.reset();
        assert!(tracker.observe(&completed).is_empty());
    }

    #[test]
    fn milestone_tracker_emits_checkpoint_abandon_and_corridor_captain_mastery() {
        let mut tracker = QuestMilestoneTracker::default();
        let hydrated = parse_frame(&json!({
            "schema": 1,
            "active": [{
                "key": GUIDED_TOUR_KEYS[1],
                "title": "Stock the Locker and Fuel the Tank",
                "allow_restart": true,
                "ready_to_finish": false,
                "checkpoints": [
                    { "id": "buy_provisions", "title": "Buy 10 provisions", "checked": false, "live_ready": false },
                    { "id": "buy_propellant", "title": "Buy 5t propellant", "checked": false, "live_ready": false }
                ]
            }],
            "completed": [{ "key": GUIDED_TOUR_KEYS[0], "title": "Step 1" }]
        }));
        assert!(tracker.observe(&hydrated).is_empty());

        // Individual objective completion emits Checkpoint milestone without Ready
        let step_one_done = parse_frame(&json!({
            "schema": 1,
            "active": [{
                "key": GUIDED_TOUR_KEYS[1],
                "title": "Stock the Locker and Fuel the Tank",
                "allow_restart": true,
                "ready_to_finish": false,
                "checkpoints": [
                    { "id": "buy_provisions", "title": "Buy 10 provisions", "checked": true, "live_ready": true },
                    { "id": "buy_propellant", "title": "Buy 5t propellant", "checked": false, "live_ready": false }
                ]
            }],
            "completed": [{ "key": GUIDED_TOUR_KEYS[0], "title": "Step 1" }]
        }));
        let m_cp = tracker.observe(&step_one_done);
        assert_eq!(m_cp.len(), 1);
        assert_eq!(m_cp[0].kind, QuestMilestoneKind::Checkpoint);
        assert_eq!(
            m_cp[0].notice_text(),
            "[QUEST] Objective Complete: Buy 10 provisions"
        );

        // Final checkpoint turning quest ready emits Ready only (no duplicate Checkpoint)
        let now_ready = parse_frame(&json!({
            "schema": 1,
            "active": [{
                "key": GUIDED_TOUR_KEYS[1],
                "title": "Stock the Locker and Fuel the Tank",
                "allow_restart": true,
                "ready_to_finish": true,
                "checkpoints": [
                    { "id": "buy_provisions", "title": "Buy 10 provisions", "checked": true, "live_ready": true },
                    { "id": "buy_propellant", "title": "Buy 5t propellant", "checked": true, "live_ready": true }
                ]
            }],
            "completed": [{ "key": GUIDED_TOUR_KEYS[0], "title": "Step 1" }]
        }));
        let m_ready = tracker.observe(&now_ready);
        assert_eq!(m_ready.len(), 1);
        assert_eq!(m_ready[0].kind, QuestMilestoneKind::Ready);

        // Abandoning a restartable quest emits Abandoned with restartable note
        let abandoned = parse_frame(&json!({
            "schema": 1,
            "active": [],
            "completed": [{ "key": GUIDED_TOUR_KEYS[0], "title": "Step 1" }]
        }));
        let m_ab = tracker.observe(&abandoned);
        assert_eq!(m_ab.len(), 1);
        assert_eq!(m_ab[0].kind, QuestMilestoneKind::Abandoned);
        assert!(m_ab[0].restartable);
        assert_eq!(
            m_ab[0].notice_text(),
            "[QUEST] Abandoned (restartable anytime): Stock the Locker and Fuel the Tank"
        );

        // Completing the 7th guided quest emits Corridor Captain mastery celebration
        tracker.reset();
        let before_7th = parse_frame(&json!({
            "schema": 1,
            "active": [{
                "key": GUIDED_TOUR_KEYS[6],
                "title": "Deliver Your Mined Ore to Market",
                "ready_to_finish": true
            }],
            "completed": [
                { "key": GUIDED_TOUR_KEYS[0], "title": "Step 1" },
                { "key": GUIDED_TOUR_KEYS[1], "title": "Step 2" },
                { "key": GUIDED_TOUR_KEYS[2], "title": "Step 3" },
                { "key": GUIDED_TOUR_KEYS[3], "title": "Step 4" },
                { "key": GUIDED_TOUR_KEYS[4], "title": "Step 5" },
                { "key": GUIDED_TOUR_KEYS[5], "title": "Step 6" }
            ]
        }));
        assert!(tracker.observe(&before_7th).is_empty());

        let all_7_done = parse_frame(&json!({
            "schema": 1,
            "active": [],
            "completed": [
                { "key": GUIDED_TOUR_KEYS[0], "title": "Step 1" },
                { "key": GUIDED_TOUR_KEYS[1], "title": "Step 2" },
                { "key": GUIDED_TOUR_KEYS[2], "title": "Step 3" },
                { "key": GUIDED_TOUR_KEYS[3], "title": "Step 4" },
                { "key": GUIDED_TOUR_KEYS[4], "title": "Step 5" },
                { "key": GUIDED_TOUR_KEYS[5], "title": "Step 6" },
                { "key": GUIDED_TOUR_KEYS[6], "title": "Deliver Your Mined Ore to Market" }
            ]
        }));
        let m_master = tracker.observe(&all_7_done);
        assert_eq!(m_master.len(), 1);
        assert_eq!(m_master[0].kind, QuestMilestoneKind::Completed);
        assert!(m_master[0].tour_all_done);
        assert!(m_master[0].notice_text().contains("Corridor Captain (7/7)"));
    }

    #[test]
    fn active_objective_room_and_open_tab_index() {
        let snap = parse_frame(&json!({
            "schema": 1,
            "active": [{
                "key": "q1",
                "title": "First Papers",
                "ready_to_finish": false,
                "next_room_entry": "room.c04_coop_registry",
                "checkpoints": [
                    { "id": "cp1", "title": "Enroll", "room_entry": "room.c04_coop_registry", "checked": false, "live_ready": false }
                ]
            }]
        }));
        assert_eq!(
            snap.active_objective_room(None),
            Some(("room.c04_coop_registry".to_string(), false))
        );

        let fallback = parse_frame(&json!({
            "schema": 1,
            "active": [{
                "key": "q2",
                "title": "Return Report",
                "ready_to_finish": true,
                "checkpoints": [
                    { "id": "cp1", "title": "Report back", "room_entry": "room.c03_arrivals", "checked": false, "live_ready": false }
                ]
            }]
        }));
        assert_eq!(
            fallback.active_objective_room(None),
            Some(("room.c03_arrivals".to_string(), true))
        );

        let mut overlay = QuestOverlayState {
            confirm_abandon: true,
            ..Default::default()
        };
        overlay.open_tab_index(QuestTab::Available, 2);
        assert!(overlay.open);
        assert!(!overlay.confirm_abandon);
        assert_eq!(overlay.tab, QuestTab::Available);
        assert_eq!(overlay.selected, 2);

        let empty_lines = render_quest_sheet_lines(&QuestsSnapshot::default(), None);
        assert!(empty_lines.is_empty());

        let sheet_lines = render_quest_sheet_lines(&snap, None);
        let sheet_text: String = sheet_lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(sheet_text.contains("Guided Quests"));
        assert!(sheet_text.contains("First Papers (0/1)"));
        assert!(sheet_text.contains("Completed: 0"));

        // Completing all 7 guided tour quests surfaces Corridor Captain (7/7) mastery
        let mastered = parse_frame(&json!({
            "schema": 1,
            "completed": GUIDED_TOUR_KEYS
                .iter()
                .map(|k| json!({ "key": k, "title": "Done" }))
                .collect::<Vec<_>>()
        }));
        let m_tour = mastered.guided_tour_progress().expect("mastered tour");
        assert!(m_tour.all_done);
        let m_text: String = render_quest_sheet_lines(&mastered, None)
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(m_text.contains("Corridor Captain [#######] 7/7"));
    }

    #[test]
    fn tracker_lines_show_tour_progress_route_hop_action_cues_and_mastery() {
        let join = |lines: Vec<Line<'static>>| -> String {
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

        // 1. Active guided quest with route hop + suggested command cue
        let active_snap = parse_frame(&json!({
            "schema": 1,
            "completed": [{ "key": GUIDED_TOUR_KEYS[0], "title": "Step 1" }],
            "active": [{
                "key": GUIDED_TOUR_KEYS[4],
                "title": "Refit the Dray for Surface Mining",
                "next_step": "Run +coop/refit prospector",
                "checkpoints": [{
                    "id": "refit_dray_p",
                    "title": "Refit Dray-P",
                    "instructions": "Run +coop/refit prospector.",
                    "checked": false,
                    "live_ready": false
                }]
            }]
        }));
        let hop = QuestExitBreadcrumb {
            next_room_entry: "room.c02_pressure".into(),
            next_room_name: "Pressure Lobby".into(),
            next_exit_command: "goto n".into(),
            target_room_name: "Registry & Co-op Desk".into(),
            hops: 3,
        };
        let active_text = join(render_quest_tracker_lines_with_route(
            &active_snap,
            None,
            Some(&hop),
        ));
        assert!(active_text.contains("Tour [#--->--] 1/7"), "{active_text}");
        assert!(
            active_text.contains("Route: -> Pressure Lobby (3h)"),
            "{active_text}"
        );
        assert!(
            active_text.contains("F11 -> p: +coop/refit prospector"),
            "{active_text}"
        );

        // 2. Room offer without active quest shows quick-accept hint
        let offer_snap = parse_frame(&json!({
            "schema": 1,
            "available": [{
                "key": GUIDED_TOUR_KEYS[0],
                "title": "First Papers",
                "giver_name": "Maren Halvorsen",
                "accept_cmd": "+quest/accept #99 tok"
            }]
        }));
        let offer_text = join(render_quest_tracker_lines_with_route(
            &offer_snap,
            None,
            None,
        ));
        assert!(offer_text.contains("Tour [>------] 0/7"), "{offer_text}");
        assert!(
            offer_text.contains("F11 -> a: Accept Quest"),
            "{offer_text}"
        );

        // 3. 7/7 mastered tour with no active/available quest shows emerald completion crest
        let mastered_snap = parse_frame(&json!({
            "schema": 1,
            "completed": GUIDED_TOUR_KEYS
                .iter()
                .map(|k| json!({ "key": k, "title": "Done" }))
                .collect::<Vec<_>>()
        }));
        let mastered_text = join(render_quest_tracker_lines_with_route(
            &mastered_snap,
            None,
            None,
        ));
        assert!(
            mastered_text.contains("[*] Corridor Captain (7/7)"),
            "{mastered_text}"
        );
        assert!(
            mastered_text.contains("Guided Tour Complete"),
            "{mastered_text}"
        );
    }

    #[test]
    fn surface_mining_prep_hints_and_post_mine_recover_action() {
        let before_scan = parse_frame(&json!({
            "schema": 1,
            "active": [{
                "key": GUIDED_TOUR_KEYS[5],
                "title": "Scan, Sound, and Mine a Landing Site",
                "checkpoints": [
                    { "id": "scan_landing_site", "title": "Surface scan", "instructions": "Run +tv/1 scan go.", "checked": false, "live_ready": false },
                    { "id": "sound_mineral_pocket", "title": "Sound pocket", "instructions": "Run +tv/1 sound go.", "checked": false, "live_ready": false },
                    { "id": "mine_carbon_ore", "title": "Mine 1t carbon_ore", "instructions": "Run +tv/1 mine go.", "checked": false, "live_ready": false }
                ]
            }]
        }));
        let m0 = before_scan
            .surface_mining_status(None)
            .expect("mining status");
        assert_eq!(
            m0.prep_hint_line(),
            "Prep: +license | +tv/lease | +chart/ground"
        );

        let after_mine = parse_frame(&json!({
            "schema": 1,
            "active": [{
                "key": GUIDED_TOUR_KEYS[5],
                "title": "Scan, Sound, and Mine a Landing Site",
                "ready_to_finish": false,
                "can_check_now": false,
                "can_finish_now": false,
                "checkpoints": [
                    { "id": "scan_landing_site", "title": "Surface scan", "instructions": "Run +tv/1 scan go.", "checked": true, "live_ready": true },
                    { "id": "sound_mineral_pocket", "title": "Sound pocket", "instructions": "Run +tv/1 sound go.", "checked": true, "live_ready": true },
                    { "id": "mine_carbon_ore", "title": "Mine 1t carbon_ore", "instructions": "Run +tv/1 mine go.", "checked": true, "live_ready": true }
                ]
            }]
        }));
        let m1 = after_mine
            .surface_mining_status(None)
            .expect("mining status");
        assert!(m1.prep_hint_line().contains("+tv/1 recover go"));
        assert!(m1.prep_hint_line().contains("carbon_ore"));
        let act = QuestsSnapshot::suggested_action_for(&after_mine.active[0])
            .expect("recover rig suggested action");
        assert_eq!(act.command, "+tv/1 recover go");

        // Pressing 'p' in overlay prefills +tv/1 recover go
        let mut overlay = QuestOverlayState::default();
        overlay.open_for(&after_mine);
        assert_eq!(
            overlay.handle_key(Key::Char('p'), &after_mine),
            QuestOverlayAction::Prefill("+tv/1 recover go".into())
        );
    }

    #[test]
    fn infers_station_port_from_room_prefix_and_routes_docked_ship_to_airlock() {
        assert_eq!(
            infer_station_port_from_room_entry("room.c08_chandlery"),
            Some("ceres")
        );
        assert_eq!(
            infer_station_port_from_room_entry("room.p07_receiving"),
            Some("stickney")
        );
        assert_eq!(
            infer_station_port_from_room_entry("room.l01_berth_gallery"),
            Some("luna")
        );
        assert_eq!(
            infer_station_port_from_room_entry("room.d02_concourse"),
            Some("deimos")
        );
        assert_eq!(
            infer_station_port_from_room_entry("room.t04_deck"),
            Some("titan")
        );
        assert_eq!(
            infer_station_port_from_room_entry("room.welcome_hall"),
            None
        );

        let snap = parse_frame(&json!({
            "schema": 1,
            "active": [{
                "key": GUIDED_TOUR_KEYS[2],
                "title": "Metals Down the Corridor",
                "next_room_entry": "room.p07_receiving",
                "checkpoints": [{
                    "id": "deliver_metals",
                    "title": "Deliver 5t metals to Stickney Yards",
                    "instructions": "Run +ship/plan stickney and sell at Receiving.",
                    "room_entry": "room.p07_receiving",
                    "checked": false,
                    "live_ready": false
                }]
            }]
        }));
        assert_eq!(
            snap.active_ship_target_compartment_for_port(None, Some("ceres")),
            Some("bridge")
        );
        assert_eq!(
            snap.active_ship_target_compartment_for_port(None, Some("stickney")),
            Some("airlock")
        );
    }

    #[test]
    fn tracker_cycles_multiple_active_quests_and_shows_simultaneous_room_offers_and_boarding_route()
    {
        let join = |lines: Vec<Line<'static>>| -> String {
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

        let snap = parse_frame(&json!({
            "schema": 1,
            "active": [
                { "key": "q.first", "title": "First Quest", "next_step": "Do A", "checkpoints": [] },
                { "key": "q.second", "title": "Second Quest", "next_step": "Do B", "checkpoints": [] }
            ],
            "available": [
                { "key": "q.offer", "title": "New Room Offer", "giver_name": "Ilse", "accept_cmd": "+quest/accept #18 tok" }
            ]
        }));
        let mut overlay = QuestOverlayState::default();
        let board_hop = QuestExitBreadcrumb {
            next_room_entry: "ship".into(),
            next_room_name: "Board Ship".into(),
            next_exit_command: "+ship/board".into(),
            target_room_name: "Board Ship".into(),
            hops: 1,
        };
        let first_txt = join(render_quest_tracker_lines_with_route(
            &snap,
            overlay.tracked_key.as_deref(),
            Some(&board_hop),
        ));
        assert!(first_txt.contains("First Quest (0/0) [1/2]"), "{first_txt}");
        assert!(
            first_txt.contains("Route: +ship/board (Board Ship)"),
            "{first_txt}"
        );
        assert!(
            first_txt.contains("[!] 1 offer here (F11 -> a)"),
            "{first_txt}"
        );

        assert_eq!(overlay.cycle_tracked(&snap), Some("q.second"));
        let second_txt = join(render_quest_tracker_lines_with_route(
            &snap,
            overlay.tracked_key.as_deref(),
            None,
        ));
        assert!(
            second_txt.contains("Second Quest (0/0) [2/2]"),
            "{second_txt}"
        );
        assert_eq!(overlay.cycle_tracked(&snap), Some("q.first"));
    }

    #[test]
    fn active_destination_port_falls_back_to_turn_in_station_when_objectives_complete() {
        let ready_off_world = parse_frame(&json!({
            "schema": 1,
            "active": [{
                "key": "tacsim.quests.guided_onboarding/quest.guide_06_surface_mining",
                "title": "Scan, Sound, and Mine a Landing Site",
                "next_room_entry": "room.c07_exchange",
                "turn_in_here": false,
                "ready_to_finish": true,
                "can_finish_now": false,
                "checkpoints": [
                    { "id": "scan_landing_site", "title": "Surface scan", "room_entry": "room.c07_exchange", "checked": true, "live_ready": true },
                    { "id": "mine_carbon_ore", "title": "Mine carbon_ore", "room_entry": "room.c07_exchange", "checked": true, "live_ready": true }
                ]
            }]
        }));
        let dest = ready_off_world
            .active_destination_port(None)
            .expect("expected Ceres Freeport turn-in destination");
        assert_eq!(dest.port_id, "ceres");
        assert_eq!(dest.port_label, "Ceres Freeport");

        // Standing in Rosalind's room (turn_in_here: true) clears corridor destination
        let at_turn_in = parse_frame(&json!({
            "schema": 1,
            "active": [{
                "key": "tacsim.quests.guided_onboarding/quest.guide_06_surface_mining",
                "title": "Scan, Sound, and Mine a Landing Site",
                "next_room_entry": "room.c07_exchange",
                "turn_in_here": true,
                "ready_to_finish": true,
                "can_finish_now": true,
                "checkpoints": [
                    { "id": "mine_carbon_ore", "title": "Mine carbon_ore", "room_entry": "room.c07_exchange", "checked": true, "live_ready": true }
                ]
            }]
        }));
        assert!(at_turn_in.active_destination_port(None).is_none());
    }

    #[test]
    fn guided_tour_roadmap_tracks_all_seven_steps_and_renders_overlay_line() {
        let snap = parse_frame(&json!({
            "schema": 1,
            "completed": [
                { "key": GUIDED_TOUR_KEYS[0], "title": "First Papers" },
                { "key": GUIDED_TOUR_KEYS[1], "title": "Stock Locker" }
            ],
            "active": [{
                "key": GUIDED_TOUR_KEYS[2],
                "title": "Metals Down the Corridor",
                "ready_to_finish": true,
                "checkpoints": []
            }],
            "available": [{
                "key": GUIDED_TOUR_KEYS[3],
                "title": "Return Load",
                "checkpoints": []
            }]
        }));
        let roadmap = snap.guided_tour_roadmap();
        assert_eq!(roadmap.len(), 7);
        assert_eq!(roadmap[0].status, GuidedTourStepStatus::Completed);
        assert_eq!(roadmap[1].status, GuidedTourStepStatus::Completed);
        assert_eq!(roadmap[2].status, GuidedTourStepStatus::Active);
        assert!(roadmap[2].ready_to_turn_in);
        assert_eq!(roadmap[3].status, GuidedTourStepStatus::Available);
        assert_eq!(roadmap[4].status, GuidedTourStepStatus::Upcoming);
        assert_eq!(roadmap[6].short_label, "Sell Mined Ore");

        let line = snap
            .render_guided_tour_roadmap_line()
            .expect("expected rendered roadmap line");
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("[x] 1. Co-op Charter"), "{text}");
        assert!(text.contains("[x] 2. Locker & Tanks"), "{text}");
        assert!(text.contains("[?] 3. Metals Haul"), "{text}");
        assert!(text.contains("[!] 4. Return Load"), "{text}");
        assert!(text.contains("[ ] 5. Dray-P Refit"), "{text}");
        assert!(text.contains("[ ] 7. Sell Mined Ore"), "{text}");
    }

    #[test]
    fn next_guided_offer_locates_giver_and_routes_tracker_and_ship_between_steps() {
        let between = parse_frame(&json!({
            "schema": 1,
            "completed": [
                { "key": GUIDED_TOUR_KEYS[0], "title": "Step 1" },
                { "key": GUIDED_TOUR_KEYS[1], "title": "Step 2" },
                { "key": GUIDED_TOUR_KEYS[2], "title": "Step 3" }
            ],
            "active": [],
            "available": []
        }));
        let next = between
            .next_guided_offer()
            .expect("expected Step 4 next guided offer");
        assert_eq!(next.step, 4);
        assert_eq!(next.giver_label, "Jonas Achterberg-Nkemelu");
        assert_eq!(next.giver_room_entry, "room.p07_receiving");
        assert_eq!(
            between.active_objective_room(None),
            Some(("room.p07_receiving".to_string(), false))
        );
        // Docked at Stickney -> airlock (+ship/leave); docked at Ceres -> bridge
        assert_eq!(
            between.active_ship_target_compartment_for_location(None, Some("stickney"), false),
            Some("airlock")
        );
        assert_eq!(
            between.active_ship_target_compartment_for_location(None, Some("ceres"), false),
            Some("bridge")
        );

        let hop = QuestExitBreadcrumb {
            next_room_entry: "room.p07_receiving".into(),
            next_room_name: "Receiving Counter".into(),
            next_exit_command: "goto receiving".into(),
            target_room_name: "Receiving Counter".into(),
            hops: 1,
        };
        let tracker_txt: String = render_quest_tracker_lines_with_route(&between, None, Some(&hop))
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            tracker_txt.contains("Next (4/7): Return Load"),
            "{tracker_txt}"
        );
        assert!(
            tracker_txt.contains("Jonas Achterberg-Nkemelu (Receiving Counter)"),
            "{tracker_txt}"
        );
        assert!(
            tracker_txt.contains("Route: -> Receiving Counter (1h)"),
            "{tracker_txt}"
        );

        // Fresh player (0 completed) -> None
        let empty =
            parse_frame(&json!({ "schema": 1, "completed": [], "active": [], "available": [] }));
        assert!(empty.next_guided_offer().is_none());
        assert!(empty.active_objective_room(None).is_none());
    }

    #[test]
    fn recommended_ground_site_prefers_matching_ore_then_open_starter_site() {
        let qm = QuestSurfaceMiningStatus {
            quest_title: "Scan, Sound, and Mine a Landing Site".into(),
            target_ore_slug: "carbon_ore".into(),
            recommended_family: "carbon".into(),
            scan_done: false,
            sound_done: false,
            mine_done: false,
        };

        // 1. Explicit carbon_ore reading wins over commons fallback
        let scanned_ground = json!({
            "ground": {
                "sites": [
                    { "venue": "commons", "site": "Freeport Commons", "here": false, "tenure": "open", "best": "", "best_line": "Your readings: none." },
                    { "venue": "ahuna", "site": "Ahuna Flank", "here": false, "tenure": "open", "best": "carbon_ore", "best_line": "Your readings: carbon_ore rich." }
                ]
            }
        });
        let rec = qm
            .recommended_ground_site(&scanned_ground)
            .expect("expected Ahuna Flank carbon match");
        assert_eq!(rec.venue, "ahuna");
        assert_eq!(rec.site_name, "Ahuna Flank");
        assert!(!rec.here);

        // 2. Unscanned ground chart falls back to open starter site Freeport Commons
        let unscanned_ground = json!({
            "ground": {
                "sites": [
                    { "venue": "occator", "site": "Occator Rim", "here": false, "tenure": "open", "best": "", "best_line": "Your readings: none." },
                    { "venue": "commons", "site": "Freeport Commons", "here": false, "tenure": "open", "best": "", "best_line": "Your readings: none." }
                ]
            }
        });
        let starter = qm
            .recommended_ground_site(&unscanned_ground)
            .expect("expected Freeport Commons starter site");
        assert_eq!(starter.venue, "commons");
        assert_eq!(starter.site_name, "Freeport Commons");
    }

    #[test]
    fn hold_readiness_line_formats_trade_and_refit_progress() {
        let trade_snap = parse_frame(&json!({
            "schema": 1,
            "active": [{
                "key": GUIDED_TOUR_KEYS[2],
                "title": "Metals Down the Corridor",
                "checkpoints": [{
                    "id": "deliver_metals",
                    "title": "Buy 5 tonnes of metals at Ceres and deliver to Stickney",
                    "instructions": "Buy 5t metals and +market/sell at Stickney.",
                    "checked": false,
                    "live_ready": false
                }]
            }]
        }));
        let partial = QuestStowageTelemetry {
            cargo_kg: 2000,
            propellant_kg: 10000,
            provisions_days: 8,
            cargo_lots: vec![("tacsim:metals".into(), "Metals".into(), 2)],
        };
        let (txt, ready) = QuestsSnapshot::hold_readiness_line_for(&trade_snap.active[0], &partial)
            .expect("expected trade hold progress");
        assert!(!ready);
        assert_eq!(txt, "[HOLD] METALS: 2/5 u");

        let full = QuestStowageTelemetry {
            cargo_kg: 5000,
            propellant_kg: 10000,
            provisions_days: 8,
            cargo_lots: vec![("tacsim:metals".into(), "Metals".into(), 5)],
        };
        let (txt_ok, ready_ok) =
            QuestsSnapshot::hold_readiness_line_for(&trade_snap.active[0], &full)
                .expect("expected full trade hold progress");
        assert!(ready_ok);
        assert_eq!(txt_ok, "[HOLD READY] METALS: 5/5 u");

        let refit_snap = parse_frame(&json!({
            "schema": 1,
            "active": [{
                "key": GUIDED_TOUR_KEYS[4],
                "title": "Refit the Dray for Surface Mining",
                "checkpoints": [{
                    "id": "refit_dray_p",
                    "title": "Refit Dray-P",
                    "instructions": "Run +coop/refit prospector.",
                    "checked": false,
                    "live_ready": false
                }]
            }]
        }));
        let (refit_warn, refit_rdy) =
            QuestsSnapshot::hold_readiness_line_for(&refit_snap.active[0], &partial)
                .expect("expected refit hold check");
        assert!(!refit_rdy);
        assert!(refit_warn.contains("[REFIT CHECK]"), "{refit_warn}");
        assert!(refit_warn.contains("Stores 8d"), "{refit_warn}");

        let empty_hold = QuestStowageTelemetry {
            cargo_kg: 0,
            propellant_kg: 6000,
            provisions_days: 0,
            cargo_lots: Vec::new(),
        };
        let (refit_ok, refit_ok_flag) =
            QuestsSnapshot::hold_readiness_line_for(&refit_snap.active[0], &empty_hold)
                .expect("expected refit ready");
        assert!(refit_ok_flag);
        assert_eq!(refit_ok, "[READY] Cargo empty | Stores empty | Tank 6t OK");
    }

    #[test]
    fn render_quest_sheet_lines_includes_roadmap_and_between_steps_next_guided_offer() {
        let snap = parse_frame(&json!({
            "schema": 1,
            "completed": [
                { "key": GUIDED_TOUR_KEYS[0], "title": "S1" },
                { "key": GUIDED_TOUR_KEYS[1], "title": "S2" },
                { "key": GUIDED_TOUR_KEYS[2], "title": "S3" }
            ],
            "active": [],
            "available": []
        }));
        let txt: String = render_quest_sheet_lines(&snap, None)
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(txt.contains("Tour [###----] 3/7"), "{txt}");
        assert!(txt.contains("[x] 1. Co-op Charter"), "{txt}");
        assert!(txt.contains("[ ] 4. Return Load"), "{txt}");
        assert!(
            txt.contains("Next Guided (4/7): Return Load — speak with Jonas Achterberg-Nkemelu (Receiving Counter)"),
            "{txt}"
        );
    }

    #[test]
    fn next_guided_offer_destination_routes_off_station_and_clears_when_already_at_port() {
        let snap = parse_frame(&json!({
            "schema": 1,
            "completed": [
                { "key": GUIDED_TOUR_KEYS[0], "title": "S1" },
                { "key": GUIDED_TOUR_KEYS[1], "title": "S2" },
                { "key": GUIDED_TOUR_KEYS[2], "title": "S3" }
            ],
            "active": [],
            "available": []
        }));
        // Viewing sky chart from Ceres -> Step 4 giver (Jonas) is at Stickney Yards
        let dest = snap
            .next_guided_offer_destination(Some("Ceres Freeport"))
            .expect("expected off-station destination hint");
        assert_eq!(dest.port_id, "stickney");
        assert_eq!(dest.port_label, "Stickney Yards");
        assert!(
            dest.quest_title.contains("Next Guided (4/7)"),
            "{}",
            dest.quest_title
        );
        // Viewing sky chart already at Stickney Yards -> clears
        assert!(
            snap.next_guided_offer_destination(Some("Stickney Yards"))
                .is_none()
        );
        assert!(
            snap.next_guided_offer_destination(Some("stickney"))
                .is_none()
        );
    }

    #[test]
    fn overlay_shows_optional_restartable_and_completed_tour_mastery() {
        let mut completed_json = Vec::new();
        for (idx, key) in GUIDED_TOUR_KEYS.iter().enumerate() {
            completed_json.push(json!({
                "key": key,
                "title": format!("Guided Step {}", idx + 1),
                "turn_in_name": "Corridor Host",
                "completion": format!("Finished step {}.", idx + 1),
                "reward_summary": format!("Tour Step {}/7", idx + 1)
            }));
        }
        let snap = parse_frame(&json!({
            "schema": 1,
            "active": [{
                "key": GUIDED_TOUR_KEYS[0],
                "title": "First Papers at the Registry",
                "giver_name": "Maren Halvorsen",
                "turn_in_name": "Ilse Oduya-Brandt",
                "allow_restart": true,
                "reward_summary": "Guided Tour Step 1/7",
                "next_step": "Enroll with Ilse"
            }],
            "available": [{
                "key": GUIDED_TOUR_KEYS[1],
                "title": "Stock the Locker and Tanks",
                "giver_name": "Ilse Oduya-Brandt",
                "turn_in_name": "Ilse Oduya-Brandt",
                "allow_restart": true,
                "reward_summary": "Guided Tour Step 2/7"
            }],
            "completed": completed_json
        }));

        let render_tab = |tab: QuestTab| -> String {
            let mut st = QuestOverlayState::default();
            st.open_tab_index(tab, 0);
            let backend = ratatui::backend::TestBackend::new(100, 28);
            let mut term = ratatui::Terminal::new(backend).expect("test terminal");
            term.draw(|f| render_quest_overlay(f, f.area(), &st, &snap))
                .expect("render overlay");
            term.backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect()
        };

        let active_txt = render_tab(QuestTab::Active);
        assert!(
            active_txt.contains("Optional · Restartable"),
            "{active_txt}"
        );
        let avail_txt = render_tab(QuestTab::Available);
        assert!(avail_txt.contains("Optional · Restartable"), "{avail_txt}");
        let done_txt = render_tab(QuestTab::Completed);
        assert!(
            done_txt.contains("Guided Corridor Tour Complete (7/7) — Corridor Captain"),
            "{done_txt}"
        );
        assert!(done_txt.contains("[Step 1/7]"), "{done_txt}");
        assert!(done_txt.contains("Tour Step 1/7"), "{done_txt}");
    }

    #[test]
    fn active_trade_needs_extracts_both_step2_items_and_advances_hold_readiness() {
        let snap = parse_frame(&json!({
            "schema": 1,
            "active": [{
                "key": GUIDED_TOUR_KEYS[1],
                "title": "Stock the Locker and Tanks",
                "checkpoints": [
                    {
                        "id": "buy_provisions",
                        "title": "Buy at least 10 provisions at Ceres Co-op Chandlery",
                        "instructions": "Buy 10 provisions.",
                        "checked": false,
                        "live_ready": false
                    },
                    {
                        "id": "buy_propellant",
                        "title": "Buy at least 5 tonnes of propellant at Ceres Exchange Hall",
                        "instructions": "Buy 5 propellant.",
                        "checked": false,
                        "live_ready": false
                    }
                ]
            }]
        }));
        let needs = snap.active_trade_needs(None);
        assert_eq!(needs.len(), 2);
        assert_eq!(needs[0].commodity_slug, "provisions");
        assert_eq!(needs[1].commodity_slug, "propellant");

        let (line, ready) = QuestsSnapshot::hold_readiness_line_for(
            &snap.active[0],
            &QuestStowageTelemetry {
                cargo_kg: 0,
                propellant_kg: 0,
                provisions_days: 10,
                cargo_lots: Vec::new(),
            },
        )
        .expect("hold readiness");
        assert!(!ready);
        assert!(line.contains("[HOLD] PROPELLANT: 0/5 t"), "{line}");
        assert!(line.contains("PROVISIONS 10/10d OK"), "{line}");
    }

    #[test]
    fn scene_quest_tracker_renders_live_step2_hold_and_step5_refit_readiness() {
        let snap = parse_frame(&json!({
            "schema": 1,
            "active": [
                {
                    "key": GUIDED_TOUR_KEYS[1],
                    "title": "Stock the Locker and Tanks",
                    "next_step": "Buy 5t propellant at Exchange",
                    "checkpoints": [
                        {
                            "id": "buy_provisions",
                            "title": "Buy 10 provisions",
                            "instructions": "Buy 10 provisions.",
                            "checked": false,
                            "live_ready": false
                        },
                        {
                            "id": "buy_propellant",
                            "title": "Buy 5t propellant",
                            "instructions": "Buy 5 tonnes of propellant.",
                            "checked": false,
                            "live_ready": false
                        }
                    ]
                },
                {
                    "key": GUIDED_TOUR_KEYS[4],
                    "title": "Refit the Dray for Surface Mining",
                    "next_step": "Run +coop/refit prospector",
                    "checkpoints": [
                        {
                            "id": "refit_dray_p",
                            "title": "Refit Dray into Dray-P",
                            "instructions": "Empty hold and run +coop/refit prospector.",
                            "checked": false,
                            "live_ready": false
                        }
                    ]
                }
            ]
        }));
        let stowage = QuestStowageTelemetry {
            cargo_kg: 0,
            propellant_kg: 2000,
            provisions_days: 10,
            cargo_lots: Vec::new(),
        };

        let step2_lines = render_quest_tracker_lines_with_route_and_stowage(
            &snap,
            Some(GUIDED_TOUR_KEYS[1]),
            None,
            Some(&stowage),
        );
        let step2_text: String = step2_lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            step2_text.contains("[HOLD] PROPELLANT: 2/5 t | PROVISIONS 10/10d OK"),
            "{step2_text}"
        );

        let step5_lines = render_quest_tracker_lines_with_route_and_stowage(
            &snap,
            Some(GUIDED_TOUR_KEYS[4]),
            None,
            Some(&stowage),
        );
        let step5_text: String = step5_lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            step5_text.contains("[REFIT CHECK] Cargo empty | Stores 10d | Tank 2t OK"),
            "{step5_text}"
        );
    }

    #[test]
    fn prioritized_room_markers_and_status_bar_quest_badge() {
        let snap = parse_frame(&json!({
            "schema": 1,
            "room_markers": [
                { "npc_id": "#1", "npc_name": "Silver Clerk", "marker": "in_progress", "quest_key": "q1" },
                { "npc_id": "#2", "npc_name": "Maren Halvorsen", "marker": "available", "quest_key": "q2" },
                { "npc_id": "#3", "npc_name": "Ilse Oduya-Brandt", "marker": "turn_in", "quest_key": "q3" }
            ],
            "active": [],
            "available": [{ "key": "q2", "title": "First Papers", "giver_name": "Maren Halvorsen" }],
            "completed": []
        }));
        let ordered: Vec<&str> = snap
            .prioritized_room_markers()
            .iter()
            .map(|m| m.marker.as_str())
            .collect();
        assert_eq!(ordered, vec!["turn_in", "available", "in_progress"]);
        let (badge, color) = snap.status_bar_quest_badge().expect("turn-in badge");
        assert_eq!(badge, "[?] Ilse Oduya-Brandt: Turn-In (F11)");
        assert_eq!(color, Color::Green);

        let offer_only = parse_frame(&json!({
            "schema": 1,
            "room_markers": [
                { "npc_id": "#2", "npc_name": "Maren Halvorsen", "marker": "available", "quest_key": "q2" }
            ],
            "available": [{ "key": "q2", "title": "First Papers", "giver_name": "Maren Halvorsen" }]
        }));
        let (offer_badge, offer_color) = offer_only.status_bar_quest_badge().expect("offer badge");
        assert_eq!(offer_badge, "[!] Maren Halvorsen: Offer (F11)");
        assert_eq!(offer_color, Color::Yellow);
    }

    #[test]
    fn guided_auto_track_priority_and_tracked_scope_ship_routing_isolation() {
        let snap = parse_frame(&json!({
            "schema": 1,
            "active": [
                {
                    "key": "q.side_deimos",
                    "title": "Deliver Water Ice to Deimos",
                    "category": "optional",
                    "next_room_entry": "room.d02_concourse",
                    "checkpoints": [{
                        "id": "deliver_water_ice",
                        "title": "Deliver 3t water ice to Voltaire Anchorage",
                        "instructions": "Run +ship/plan deimos and +market/sell water_ice.",
                        "room_entry": "room.d02_concourse",
                        "checked": false,
                        "live_ready": false
                    }]
                },
                {
                    "key": GUIDED_TOUR_KEYS[5],
                    "title": "Scan, Sound, and Mine a Landing Site",
                    "category": "guided",
                    "next_room_entry": "room.c07_exchange",
                    "checkpoints": [
                        { "id": "scan_landing_site", "title": "Surface scan", "instructions": "+tv/1 scan go", "checked": false, "live_ready": false },
                        { "id": "mine_carbon_ore", "title": "Mine 1t carbon_ore", "instructions": "+tv/1 mine go", "checked": false, "live_ready": false }
                    ]
                }
            ]
        }));

        let mut overlay = QuestOverlayState::default();
        overlay.sync_tracked_for(&snap);
        assert_eq!(overlay.tracked_key.as_deref(), Some(GUIDED_TOUR_KEYS[5]));
        // Tracking Step 6 on settled pad -> Vehicle Bay
        assert_eq!(
            snap.active_ship_target_compartment_for_location(
                overlay.tracked_key.as_deref(),
                None,
                true
            ),
            Some("bay")
        );

        // Tracking side Deimos delivery -> Bridge (never leaks Step 6 bay routing!), Airlock when docked at Deimos
        assert_eq!(
            snap.active_ship_target_compartment_for_location(
                Some("q.side_deimos"),
                Some("ceres"),
                false
            ),
            Some("bridge")
        );
        assert_eq!(
            snap.active_ship_target_compartment_for_location(
                Some("q.side_deimos"),
                Some("deimos"),
                false
            ),
            Some("airlock")
        );
    }

    #[test]
    fn full_seven_step_guided_onboarding_walkthrough_to_corridor_captain() {
        let mut completed: Vec<Value> = Vec::new();
        let mut milestones = QuestMilestoneTracker::default();

        // Step 1: Maren offer -> enroll Co-op -> Ilse turn-in
        let s1_offer = parse_frame(&json!({
            "schema": 1,
            "room_markers": [{ "npc_id": "#11", "npc_name": "Maren Halvorsen", "marker": "available", "quest_key": GUIDED_TOUR_KEYS[0] }],
            "available": [{ "key": GUIDED_TOUR_KEYS[0], "title": "First Papers at the Registry", "giver_name": "Maren Halvorsen", "accept_cmd": "+quest/accept #11 tok1" }],
            "completed": completed
        }));
        let _ = milestones.observe(&s1_offer);
        assert_eq!(
            s1_offer.status_bar_quest_badge().map(|(b, _)| b).as_deref(),
            Some("[!] Maren Halvorsen: Offer (F11)")
        );

        let s1_active = parse_frame(&json!({
            "schema": 1,
            "active": [{
                "key": GUIDED_TOUR_KEYS[0],
                "title": "First Papers at the Registry",
                "category": "guided",
                "next_room_entry": "room.c05_registry",
                "checkpoints": [{ "id": "enroll_coop", "title": "Enroll Co-op", "instructions": "+coop/enroll", "room_entry": "room.c05_registry", "checked": false, "live_ready": false }]
            }],
            "completed": completed
        }));
        assert!(
            milestones
                .observe(&s1_active)
                .iter()
                .any(|m| m.kind == QuestMilestoneKind::Accepted)
        );
        assert_eq!(
            QuestsSnapshot::suggested_action_for(&s1_active.active[0])
                .map(|a| a.command)
                .as_deref(),
            Some("+coop/enroll ")
        );
        completed.push(json!({ "key": GUIDED_TOUR_KEYS[0], "title": "Step 1" }));

        // Between Step 1 and Step 2 -> Next Guided points to Ilse at Registry
        let between12 = parse_frame(&json!({ "schema": 1, "completed": completed }));
        assert_eq!(between12.next_guided_offer().map(|n| n.step), Some(2));

        // Step 2: Buy 10 provisions & 5t propellant
        let s2 = parse_frame(&json!({
            "schema": 1,
            "active": [{
                "key": GUIDED_TOUR_KEYS[1],
                "title": "Stock the Locker and Tanks",
                "checkpoints": [
                    { "id": "buy_provisions", "title": "Buy 10 provisions", "instructions": "Buy 10 provisions.", "checked": false, "live_ready": false },
                    { "id": "buy_propellant", "title": "Buy 5t propellant", "instructions": "Buy 5t propellant.", "checked": false, "live_ready": false }
                ]
            }],
            "completed": completed
        }));
        assert_eq!(s2.active_trade_needs(None).len(), 2);
        completed.push(json!({ "key": GUIDED_TOUR_KEYS[1], "title": "Step 2" }));

        // Step 3: Metals to Stickney Yards
        let s3 = parse_frame(&json!({
            "schema": 1,
            "active": [{
                "key": GUIDED_TOUR_KEYS[2],
                "title": "Metals Down the Corridor",
                "next_room_entry": "room.p07_receiving",
                "checkpoints": [{ "id": "deliver_metals", "title": "Deliver 5t metals to Stickney", "instructions": "Buy 5t metals, +ship/plan stickney, +market/sell.", "room_entry": "room.p07_receiving", "checked": false, "live_ready": false }]
            }],
            "completed": completed
        }));
        assert_eq!(
            s3.active_destination_port(None).map(|d| d.port_id),
            Some("stickney".to_string())
        );
        assert_eq!(
            s3.active_ship_target_compartment_for_location(None, Some("stickney"), false),
            Some("airlock")
        );
        completed.push(json!({ "key": GUIDED_TOUR_KEYS[2], "title": "Step 3" }));

        // Step 4: Return 2t industrial_parts to Ceres
        let s4 = parse_frame(&json!({
            "schema": 1,
            "active": [{
                "key": GUIDED_TOUR_KEYS[3],
                "title": "A Return Load for Ceres",
                "next_room_entry": "room.c08_chandlery",
                "checkpoints": [{ "id": "return_industrial_parts", "title": "Return 2t industrial parts to Ceres", "instructions": "Buy 2t industrial_parts, +ship/plan ceres, +market/sell.", "room_entry": "room.c08_chandlery", "checked": false, "live_ready": false }]
            }],
            "completed": completed
        }));
        assert_eq!(
            s4.active_destination_port(None).map(|d| d.port_id),
            Some("ceres".to_string())
        );
        completed.push(json!({ "key": GUIDED_TOUR_KEYS[3], "title": "Step 4" }));

        // Step 5: Refit Dray to Dray-P prospector
        let s5 = parse_frame(&json!({
            "schema": 1,
            "active": [{
                "key": GUIDED_TOUR_KEYS[4],
                "title": "Refit the Dray for Surface Mining",
                "checkpoints": [{ "id": "refit_dray_p", "title": "Refit Dray-P", "instructions": "+coop/refit prospector", "checked": false, "live_ready": false }]
            }],
            "completed": completed
        }));
        assert_eq!(
            QuestsSnapshot::suggested_action_for(&s5.active[0])
                .map(|a| a.command)
                .as_deref(),
            Some("+coop/refit prospector")
        );
        completed.push(json!({ "key": GUIDED_TOUR_KEYS[4], "title": "Step 5" }));

        // Step 6: Surface scan/sound/mine carbon_ore -> return to Ceres
        let s6 = parse_frame(&json!({
            "schema": 1,
            "active": [{
                "key": GUIDED_TOUR_KEYS[5],
                "title": "Scan, Sound, and Mine a Landing Site",
                "next_room_entry": "room.c07_exchange",
                "checkpoints": [
                    { "id": "scan_landing_site", "title": "Surface scan", "instructions": "+tv/1 scan go", "checked": false, "live_ready": false },
                    { "id": "mine_carbon_ore", "title": "Mine 1t carbon_ore", "instructions": "+tv/1 mine go", "checked": false, "live_ready": false }
                ]
            }],
            "completed": completed
        }));
        assert_eq!(
            s6.surface_mining_status(None).map(|m| m.recommended_family),
            Some("carbon".to_string())
        );
        assert_eq!(
            s6.active_ship_target_compartment_for_location(None, None, true),
            Some("bay")
        );
        completed.push(json!({ "key": GUIDED_TOUR_KEYS[5], "title": "Step 6" }));

        // Step 7: Sell mined carbon_ore at Stickney -> 7/7 Corridor Captain mastery
        let s7 = parse_frame(&json!({
            "schema": 1,
            "active": [{
                "key": GUIDED_TOUR_KEYS[6],
                "title": "Deliver Your Mined Ore to Market",
                "next_room_entry": "room.p11_service",
                "checkpoints": [{ "id": "sell_carbon_ore", "title": "Sell 1t carbon_ore at Stickney", "instructions": "+ship/plan stickney and +market/sell carbon_ore.", "room_entry": "room.p11_service", "checked": false, "live_ready": false }]
            }],
            "completed": completed
        }));
        assert_eq!(
            s7.active_destination_port(None).map(|d| d.port_id),
            Some("stickney".to_string())
        );
        completed.push(json!({ "key": GUIDED_TOUR_KEYS[6], "title": "Step 7" }));

        let mastered = parse_frame(&json!({ "schema": 1, "completed": completed }));
        let prog = mastered
            .guided_tour_progress()
            .expect("guided tour progress");
        assert!(prog.all_done);
        assert_eq!(prog.completed, 7);
        assert!(mastered.next_guided_offer().is_none());
    }

    #[test]
    fn quest_overlay_prioritizes_npc_turn_in_and_confirms_permanent_decline() {
        let snap = parse_frame(&json!({
            "schema": 1,
            "room_name": "Registry & Co-op Desk",
            "room_markers": [
                {
                    "npc_id": "#18",
                    "npc_name": "Ilse Oduya-Brandt",
                    "marker": "available",
                    "symbol": "!",
                    "quest_key": "side.one_shot",
                    "quest_title": "One-Shot Audit",
                    "category": "optional"
                },
                {
                    "npc_id": "#18",
                    "npc_name": "Ilse Oduya-Brandt",
                    "marker": "turn_in",
                    "symbol": "?",
                    "quest_key": GUIDED_TOUR_KEYS[0],
                    "quest_title": "First Papers at the Registry",
                    "category": "guided"
                }
            ],
            "active": [
                {
                    "key": GUIDED_TOUR_KEYS[0],
                    "title": "First Papers at the Registry",
                    "giver_name": "Maren Halvorsen",
                    "turn_in_name": "Ilse Oduya-Brandt",
                    "ready_to_finish": true,
                    "can_finish_now": true,
                    "finish_cmd": "+quest/finish #18 tok1"
                }
            ],
            "available": [
                {
                    "key": GUIDED_TOUR_KEYS[1],
                    "title": "Stock the Locker and Tanks",
                    "giver_name": "Ilse Oduya-Brandt",
                    "turn_in_name": "Ilse Oduya-Brandt",
                    "allow_restart": true,
                    "accept_cmd": "+quest/accept #18 tok_g2",
                    "decline_cmd": "+quest/decline #18 tok_g2"
                },
                {
                    "key": "side.one_shot",
                    "title": "One-Shot Audit",
                    "giver_name": "Ilse Oduya-Brandt",
                    "turn_in_name": "Ilse Oduya-Brandt",
                    "allow_restart": false,
                    "accept_cmd": "+quest/accept #18 tok_side",
                    "decline_cmd": "+quest/decline #18 tok_side"
                }
            ]
        }));

        // 1. Opening for Ilse picks turn-in [?] even when available [!] appears first in room_markers
        let mut st = QuestOverlayState::default();
        st.open_for_npc(&snap, "#18", "Ilse Oduya-Brandt");
        assert_eq!(st.tab, QuestTab::Active);
        assert_eq!(st.selected, 0);

        // 2. Switch to Available tab: 'd' on restartable guided quest does NOT arm decline
        st.handle_key(Key::Tab, &snap);
        assert_eq!(st.tab, QuestTab::Available);
        assert_eq!(
            st.handle_key(Key::Char('d'), &snap),
            QuestOverlayAction::None
        );
        assert!(!st.confirm_decline);

        // 3. Move to non-restartable offer: first 'd' arms confirm_decline, moving cancels it,
        //    then two 'd' presses submit +quest/decline
        st.handle_key(Key::Down, &snap);
        assert_eq!(st.selected, 1);
        assert_eq!(
            st.handle_key(Key::Char('d'), &snap),
            QuestOverlayAction::None
        );
        assert!(st.confirm_decline);

        st.handle_key(Key::Up, &snap);
        assert!(!st.confirm_decline);
        st.handle_key(Key::Down, &snap);
        assert_eq!(
            st.handle_key(Key::Char('d'), &snap),
            QuestOverlayAction::None
        );
        assert!(st.confirm_decline);
        assert_eq!(
            st.handle_key(Key::Char('d'), &snap),
            QuestOverlayAction::Submit("+quest/decline #18 tok_side".to_string())
        );
        assert!(!st.confirm_decline);
    }

    #[test]
    fn tracker_surfaces_secondary_turn_in_guided_offer_priority_and_off_port_sail_hint() {
        let join = |lines: Vec<Line<'static>>| -> String {
            lines
                .into_iter()
                .map(|l| {
                    l.spans
                        .into_iter()
                        .map(|s| s.content.into_owned())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        // 1. Secondary non-tracked active quest ready to turn in here
        let secondary_snap = parse_frame(&json!({
            "schema": 1,
            "active": [
                {
                    "key": GUIDED_TOUR_KEYS[2],
                    "title": "Metals Down the Corridor",
                    "next_step": "Buy 5t metals",
                    "can_finish_now": false
                },
                {
                    "key": GUIDED_TOUR_KEYS[1],
                    "title": "Stock the Locker and Tanks",
                    "next_step": "Turn in to Ilse",
                    "can_finish_now": true
                }
            ]
        }));
        let tracked_txt = join(render_quest_tracker_lines_full(
            &secondary_snap,
            Some(GUIDED_TOUR_KEYS[2]),
            None,
            None,
            Some("ceres"),
        ));
        assert!(
            tracked_txt.contains("[?] Ready here: Stock the Locker and Tanks (F11)"),
            "expected secondary turn-in alert, got:\n{tracked_txt}"
        );

        // 2. Guided offer prioritized ahead of optional offer + (+1 more)
        let offers_snap = parse_frame(&json!({
            "schema": 1,
            "available": [
                {
                    "key": "quest.side_patrol",
                    "title": "Optional Dock Sweep",
                    "category": "optional",
                    "giver_name": "Harbor Clerk"
                },
                {
                    "key": GUIDED_TOUR_KEYS[0],
                    "title": "First Papers at the Registry",
                    "category": "guided",
                    "giver_name": "Maren Halvorsen"
                }
            ]
        }));
        let offers_txt = join(render_quest_tracker_lines_full(
            &offers_snap,
            None,
            None,
            None,
            Some("ceres"),
        ));
        assert!(
            offers_txt.contains("Maren Halvorsen")
                && offers_txt.contains("First Papers at the Registry (+1 more)"),
            "expected guided offer first with (+1 more), got:\n{offers_txt}"
        );
        let sheet_txt = join(render_quest_sheet_lines(&offers_snap, None));
        assert!(
            sheet_txt.contains("First Papers at the Registry (Maren Halvorsen) (+1 more)"),
            "expected guided offer first on sheet, got:\n{sheet_txt}"
        );

        // 3. Between-steps off-port Sail hint at Ceres for Step 4 (Jonas at Stickney Yards)
        let between_snap = parse_frame(&json!({
            "schema": 1,
            "completed": [
                { "key": GUIDED_TOUR_KEYS[0], "title": "Step 1" },
                { "key": GUIDED_TOUR_KEYS[1], "title": "Step 2" },
                { "key": GUIDED_TOUR_KEYS[2], "title": "Step 3" }
            ]
        }));
        let ceres_txt = join(render_quest_tracker_lines_full(
            &between_snap,
            None,
            None,
            None,
            Some("ceres"),
        ));
        assert!(
            ceres_txt.contains("Sail: +ship/plan stickney (Stickney Yards)"),
            "expected off-port Sail hint at Ceres, got:\n{ceres_txt}"
        );
        let stickney_txt = join(render_quest_tracker_lines_full(
            &between_snap,
            None,
            None,
            None,
            Some("stickney"),
        ));
        assert!(
            !stickney_txt.contains("Sail: +ship/plan"),
            "expected no Sail hint when already at Stickney, got:\n{stickney_txt}"
        );
    }

    #[test]
    fn overlay_focuses_ready_turn_in_and_guided_offers_and_renders_step_badges() {
        let render_text = |st: &QuestOverlayState, snap: &QuestsSnapshot| -> String {
            let backend = ratatui::backend::TestBackend::new(100, 26);
            let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
            terminal
                .draw(|f| render_quest_overlay(f, f.area(), st, snap))
                .expect("draw");
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

        // 1. Active tab: ready turn-in Step 2/7 at index 1 auto-focused on open_for
        let active_snap = parse_frame(&json!({
            "schema": 1,
            "active": [
                {
                    "key": "side.delivery",
                    "title": "Side Parcel",
                    "can_finish_now": false
                },
                {
                    "key": GUIDED_TOUR_KEYS[1],
                    "title": "Stock the Locker",
                    "category": "guided",
                    "can_finish_now": true,
                    "ready_to_finish": true
                }
            ],
            "completed": [
                {
                    "key": GUIDED_TOUR_KEYS[0],
                    "title": "First Papers",
                    "turn_in_name": "Ilse"
                }
            ]
        }));
        let mut st = QuestOverlayState::default();
        st.open_for(&active_snap);
        assert_eq!(st.tab, QuestTab::Active);
        assert_eq!(st.selected, 1);
        let active_txt = render_text(&st, &active_snap);
        assert!(
            active_txt.contains("Stock the Locker [Step 2/7]"),
            "expected [Step 2/7] in Active overlay:\n{active_txt}"
        );
        assert!(
            active_txt.contains("[?] Ready to complete here"),
            "expected ready-to-complete cue:\n{active_txt}"
        );

        // Switch to Completed tab -> left list shows [Step 1/7]
        st.tab = QuestTab::Completed;
        st.selected = 0;
        let done_txt = render_text(&st, &active_snap);
        assert!(
            done_txt.contains("[x] First Papers [Step 1/7]"),
            "expected [Step 1/7] in Completed list:\n{done_txt}"
        );

        // 2. Available-only snapshot: guided Step 3/7 at index 1 auto-focused on open_for
        let avail_snap = parse_frame(&json!({
            "schema": 1,
            "available": [
                {
                    "key": "side.audit",
                    "title": "Side Audit",
                    "category": "optional",
                    "giver_name": "Clerk"
                },
                {
                    "key": GUIDED_TOUR_KEYS[2],
                    "title": "Metals Haul",
                    "category": "guided",
                    "giver_name": "Ilse"
                }
            ]
        }));
        let mut st_avail = QuestOverlayState::default();
        st_avail.open_for(&avail_snap);
        assert_eq!(st_avail.tab, QuestTab::Available);
        assert_eq!(st_avail.selected, 1);
        let avail_txt = render_text(&st_avail, &avail_snap);
        assert!(
            avail_txt.contains("[!] Metals Haul [Step 3/7]"),
            "expected [Step 3/7] in Available overlay:\n{avail_txt}"
        );
    }

    #[test]
    fn sheet_renders_step_badges_secondary_turnin_and_room_offer_alerts() {
        let join = |lines: Vec<Line<'static>>| -> String {
            lines
                .into_iter()
                .map(|l| {
                    l.spans
                        .into_iter()
                        .map(|s| s.content.into_owned())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let snap = parse_frame(&json!({
            "schema": 1,
            "completed": [{ "key": GUIDED_TOUR_KEYS[0], "title": "First Papers" }],
            "active": [
                {
                    "key": GUIDED_TOUR_KEYS[1],
                    "title": "Stock the Locker",
                    "next_step": "Buy 10 provisions at Chandlery",
                    "can_finish_now": false,
                    "checkpoints": [{ "id": "buy_prov", "title": "Buy 10 provisions", "checked": false }]
                },
                {
                    "key": "quest.side_ready",
                    "title": "Dockside Ledger Check",
                    "can_finish_now": true,
                    "ready_to_finish": true
                }
            ],
            "available": [
                {
                    "key": "quest.side_offer",
                    "title": "Barge Sweeping",
                    "category": "optional",
                    "giver_name": "Dock Clerk"
                },
                {
                    "key": GUIDED_TOUR_KEYS[2],
                    "title": "Metals Haul",
                    "category": "guided",
                    "giver_name": "Ilse"
                }
            ]
        }));
        let active_txt = join(render_quest_sheet_lines(&snap, Some(GUIDED_TOUR_KEYS[1])));
        assert!(
            active_txt.contains("Stock the Locker [Step 2/7] (0/1)"),
            "expected [Step 2/7] on active sheet row:\n{active_txt}"
        );
        assert!(
            active_txt.contains("[?] Ready here: Dockside Ledger Check (F11)"),
            "expected secondary ready turn-in on sheet:\n{active_txt}"
        );
        assert!(
            active_txt.contains("[!] 2 offers here (F11 -> a)"),
            "expected room offers alert on sheet:\n{active_txt}"
        );

        let idle_snap = parse_frame(&json!({
            "schema": 1,
            "available": [
                {
                    "key": "quest.side_offer",
                    "title": "Barge Sweeping",
                    "category": "optional",
                    "giver_name": "Dock Clerk"
                },
                {
                    "key": GUIDED_TOUR_KEYS[2],
                    "title": "Metals Haul",
                    "category": "guided",
                    "giver_name": "Ilse"
                }
            ]
        }));
        let idle_txt = join(render_quest_sheet_lines(&idle_snap, None));
        assert!(
            idle_txt.contains("Metals Haul (Ilse) (+1 more) [Step 3/7]"),
            "expected prioritized guided offer with [Step 3/7] on idle sheet:\n{idle_txt}"
        );

        // Tracker also renders [Step n/7] on active & idle guided offers plus +tv/bay route label
        let bay_hop = QuestExitBreadcrumb {
            next_room_entry: "bay".into(),
            next_room_name: "Vehicle Bay".into(),
            target_room_name: "Vehicle Bay".into(),
            next_exit_command: "+tv/bay".into(),
            hops: 1,
        };
        let tracker_active = join(render_quest_tracker_lines_full(
            &snap,
            Some(GUIDED_TOUR_KEYS[1]),
            Some(&bay_hop),
            None,
            Some("ceres"),
        ));
        assert!(
            tracker_active.contains("Stock the Locker (0/1) [Step 2/7]"),
            "expected [Step 2/7] on tracker active header:\n{tracker_active}"
        );
        assert!(
            tracker_active.contains("Route: +tv/bay (Vehicle Bay)"),
            "expected +tv/bay route label on tracker:\n{tracker_active}"
        );
        let tracker_idle = join(render_quest_tracker_lines_full(
            &idle_snap,
            None,
            None,
            None,
            Some("ceres"),
        ));
        assert!(
            tracker_idle.contains("Metals Haul (+1 more) [Step 3/7]"),
            "expected [Step 3/7] on idle tracker offer:\n{tracker_idle}"
        );
    }

    #[test]
    fn overlay_empty_active_shows_off_port_sail_shortcut_and_available_tab_hint() {
        let render_full = |st: &QuestOverlayState, snap: &QuestsSnapshot, port: Option<&str>| {
            let backend = ratatui::backend::TestBackend::new(100, 26);
            let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
            terminal
                .draw(|f| render_quest_overlay_full(f, f.area(), st, snap, None, port))
                .expect("draw");
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

        let between = parse_frame(&json!({
            "schema": 1,
            "completed": [
                { "key": GUIDED_TOUR_KEYS[0], "title": "S1" },
                { "key": GUIDED_TOUR_KEYS[1], "title": "S2" },
                { "key": GUIDED_TOUR_KEYS[2], "title": "S3" }
            ],
            "active": [],
            "available": []
        }));
        let mut st = QuestOverlayState {
            open: true,
            tab: QuestTab::Active,
            ..Default::default()
        };
        let ceres_txt = render_full(&st, &between, Some("ceres"));
        assert!(
            ceres_txt.contains("Sail [p]: +ship/plan stickney (Stickney Yards)"),
            "expected off-port Sail [p] hint:\n{ceres_txt}"
        );
        assert_eq!(
            st.handle_key_with_port(Key::Char('p'), &between, Some("ceres")),
            QuestOverlayAction::Prefill("+ship/plan stickney".into())
        );

        let with_offer = parse_frame(&json!({
            "schema": 1,
            "active": [],
            "available": [{
                "key": GUIDED_TOUR_KEYS[0],
                "title": "First Papers",
                "giver_name": "Maren Halvorsen"
            }],
            "completed": []
        }));
        st.open = true;
        st.tab = QuestTab::Active;
        let offer_txt = render_full(&st, &with_offer, Some("ceres"));
        assert!(
            offer_txt.contains("1 quest offer here (Tab -> Available)"),
            "expected Available Here Tab hint:\n{offer_txt}"
        );

        let completed_all: Vec<serde_json::Value> = GUIDED_TOUR_KEYS
            .iter()
            .enumerate()
            .map(|(i, k)| json!({ "key": k, "title": format!("Step {}", i + 1) }))
            .collect();
        let mastered = parse_frame(&json!({
            "schema": 1,
            "active": [],
            "available": [],
            "completed": completed_all
        }));
        st.open = true;
        st.tab = QuestTab::Active;
        let master_txt = render_full(&st, &mastered, Some("stickney"));
        assert!(
            master_txt.contains("* Corridor Captain (7/7) *"),
            "expected Corridor Captain mastery card in empty Active tab:\n{master_txt}"
        );
        assert!(
            master_txt.contains("Jobs [p]: +market/jobs"),
            "expected Jobs [p] shortcut in empty Active tab:\n{master_txt}"
        );
        assert_eq!(
            st.handle_key_with_port(Key::Char('p'), &mastered, Some("stickney")),
            QuestOverlayAction::Prefill("+market/jobs".into())
        );
    }

    #[test]
    fn quest_overlay_shows_step6_ore_hold_and_step7_preflight_checks() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let step6 = parse_frame(&json!({
            "schema": 1,
            "active": [{
                "key": GUIDED_TOUR_KEYS[5],
                "title": "Scan, Sound, and Mine a Landing Site",
                "checkpoints": [
                    { "id": "scan_landing_site", "title": "Scan site", "instructions": "+tv/1 scan go", "checked": true, "live_ready": true },
                    { "id": "mine_carbon_ore", "title": "Mine 1t carbon_ore", "instructions": "+tv/1 mine go", "checked": false, "live_ready": false }
                ]
            }],
            "available": [],
            "completed": []
        }));
        let stow_missing_stores = QuestStowageTelemetry {
            cargo_kg: 2000,
            propellant_kg: 12_000,
            provisions_days: 0,
            cargo_lots: vec![("tacsim:carbon_ore".into(), "Carbon Ore".into(), 2)],
        };
        let (ore_txt, ore_ready) =
            QuestsSnapshot::hold_readiness_line_for(&step6.active[0], &stow_missing_stores)
                .expect("expected Step 6 ore hold readiness");
        assert!(ore_ready);
        assert_eq!(ore_txt, "[ORE HOLD READY] CARBON ORE: 2/1 t");

        let step7 = parse_frame(&json!({
            "schema": 1,
            "active": [{
                "key": GUIDED_TOUR_KEYS[6],
                "title": "Deliver Your Mined Ore to Market",
                "checkpoints": [{
                    "id": "sell_carbon_ore",
                    "title": "Sell 1t carbon_ore at Stickney Yards",
                    "instructions": "Fly to Stickney Yards (+ship/plan stickney) and +market/sell carbon_ore.",
                    "checked": false,
                    "live_ready": false
                }]
            }],
            "available": [],
            "completed": []
        }));
        let st = QuestOverlayState {
            open: true,
            tab: QuestTab::Active,
            ..Default::default()
        };
        let render_with = |snap: &QuestsSnapshot, stow: &QuestStowageTelemetry| -> String {
            let backend = TestBackend::new(120, 28);
            let mut term = Terminal::new(backend).expect("terminal");
            term.draw(|f| render_quest_overlay_with_stowage(f, f.area(), &st, snap, Some(stow)))
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

        let txt_missing = render_with(&step7, &stow_missing_stores);
        assert!(
            txt_missing.contains("[CHECK BEFORE UNDOCK] Fuel 12t OK | Provisions 0d!"),
            "expected Step 7 missing provisions warning in overlay:\n{txt_missing}"
        );
        assert!(
            txt_missing.contains("Stickney Yards"),
            "expected Stickney Yards destination in overlay:\n{txt_missing}"
        );

        let stow_ready = QuestStowageTelemetry {
            provisions_days: 15,
            ..stow_missing_stores
        };
        let txt_ok = render_with(&step7, &stow_ready);
        assert!(
            txt_ok.contains("[PRE-FLIGHT OK] Fuel 12t | Provisions 15d"),
            "expected Step 7 pre-flight OK line in overlay:\n{txt_ok}"
        );
    }

    #[test]
    fn quest_tracker_shows_cross_port_preflight_and_mastery_jobs_hint() {
        let step7 = parse_frame(&json!({
            "schema": 1,
            "active": [{
                "key": GUIDED_TOUR_KEYS[6],
                "title": "Deliver Your Mined Ore to Market",
                "checkpoints": [{
                    "id": "sell_carbon_ore",
                    "title": "Sell 1t carbon_ore at Stickney Yards",
                    "instructions": "Fly to Stickney Yards (+ship/plan stickney) and +market/sell carbon_ore.",
                    "checked": false,
                    "live_ready": false
                }]
            }],
            "available": [],
            "completed": []
        }));
        let stow_missing = QuestStowageTelemetry {
            cargo_kg: 2000,
            propellant_kg: 12_000,
            provisions_days: 0,
            cargo_lots: vec![("tacsim:carbon_ore".into(), "Carbon Ore".into(), 2)],
        };
        let lines =
            render_quest_tracker_lines_full(&step7, None, None, Some(&stow_missing), Some("ceres"));
        let joined = lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            joined.contains("[CHECK BEFORE UNDOCK] Fuel 12t OK | Provisions 0d! -> Stickney Yards"),
            "expected tracker pre-flight warning:\n{joined}"
        );

        let completed_all: Vec<serde_json::Value> = GUIDED_TOUR_KEYS
            .iter()
            .enumerate()
            .map(|(i, k)| json!({ "key": k, "title": format!("Step {}", i + 1) }))
            .collect();
        let mastered = parse_frame(&json!({
            "schema": 1,
            "active": [],
            "available": [],
            "completed": completed_all
        }));
        let m_lines =
            render_quest_tracker_lines_full(&mastered, None, None, None, Some("stickney"));
        let m_joined = m_lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            m_joined.contains("Corridor Captain (7/7)"),
            "expected mastery tracker header:\n{m_joined}"
        );
        assert!(
            m_joined.contains("Jobs: +market/jobs"),
            "expected mastery tracker Jobs hint:\n{m_joined}"
        );
    }

    #[test]
    fn quest_sheet_shows_hold_preflight_and_mastery_jobs_hint() {
        let step7 = parse_frame(&json!({
            "schema": 1,
            "active": [{
                "key": GUIDED_TOUR_KEYS[6],
                "title": "Deliver Your Mined Ore to Market",
                "next_step": "Restock fuel and provisions, then sail to Stickney Yards.",
                "checkpoints": [{
                    "id": "sell_carbon_ore",
                    "title": "Sell 1t carbon_ore at Stickney Yards",
                    "instructions": "Fly to Stickney Yards (+ship/plan stickney) and +market/sell carbon_ore.",
                    "checked": false,
                    "live_ready": false
                }]
            }],
            "available": [],
            "completed": []
        }));
        let stow_missing = QuestStowageTelemetry {
            cargo_kg: 2000,
            propellant_kg: 12_000,
            provisions_days: 0,
            cargo_lots: vec![("tacsim:carbon_ore".into(), "Carbon Ore".into(), 2)],
        };
        let sheet_lines = render_quest_sheet_lines_with_stowage(&step7, None, Some(&stow_missing));
        let joined = sheet_lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            joined.contains("[HOLD READY] CARBON ORE: 2/1 u"),
            "expected hold readiness on sheet:\n{joined}"
        );
        assert!(
            joined.contains("[CHECK BEFORE UNDOCK] Fuel 12t OK | Provisions 0d! -> Stickney Yards"),
            "expected pre-flight readiness on sheet:\n{joined}"
        );

        let completed_all: Vec<serde_json::Value> = GUIDED_TOUR_KEYS
            .iter()
            .enumerate()
            .map(|(i, k)| json!({ "key": k, "title": format!("Step {}", i + 1) }))
            .collect();
        let mastered = parse_frame(&json!({
            "schema": 1,
            "active": [],
            "available": [],
            "completed": completed_all
        }));
        let m_lines = render_quest_sheet_lines_with_stowage(&mastered, None, None);
        let m_joined = m_lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            m_joined.contains("Corridor Captain"),
            "expected Corridor Captain header on sheet:\n{m_joined}"
        );
        assert!(
            m_joined.contains("Guided Tour Complete (7/7)"),
            "expected Guided Tour Complete line on sheet:\n{m_joined}"
        );
        assert!(
            m_joined.contains("Jobs Board: +market/jobs"),
            "expected Jobs Board pointer on sheet:\n{m_joined}"
        );
    }

    #[test]
    fn quest_tracker_shows_step6_prospecting_prep_and_hides_when_ready() {
        let pre_scan = parse_frame(&json!({
            "schema": 1,
            "active": [{
                "key": GUIDED_TOUR_KEYS[5],
                "title": "Scan, Sound, and Mine a Landing Site",
                "checkpoints": [
                    { "id": "scan_landing_site", "title": "Complete a surface scan", "checked": false, "live_ready": false },
                    { "id": "mine_carbon_ore", "title": "Mine 1t carbon ore", "checked": false, "live_ready": false }
                ]
            }],
            "available": [],
            "completed": []
        }));
        let pre_txt = render_quest_tracker_lines_full(&pre_scan, None, None, None, None)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            pre_txt.contains("Prep: +license | +tv/lease | +chart/ground"),
            "expected Step 6 pre-scan prep hint in tracker:\n{pre_txt}"
        );

        let mid_mine = parse_frame(&json!({
            "schema": 1,
            "active": [{
                "key": GUIDED_TOUR_KEYS[5],
                "title": "Scan, Sound, and Mine a Landing Site",
                "checkpoints": [
                    { "id": "scan_landing_site", "title": "Complete a surface scan", "checked": true, "live_ready": true },
                    { "id": "mine_carbon_ore", "title": "Mine 1t carbon ore", "checked": false, "live_ready": false }
                ]
            }],
            "available": [],
            "completed": []
        }));
        let mid_txt = render_quest_tracker_lines_full(&mid_mine, None, None, None, None)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            mid_txt.contains("Prep: +chart/ground | Recover: +tv/1 recover go"),
            "expected Step 6 mid-mining recover hint in tracker:\n{mid_txt}"
        );

        let ready = parse_frame(&json!({
            "schema": 1,
            "active": [{
                "key": GUIDED_TOUR_KEYS[5],
                "title": "Scan, Sound, and Mine a Landing Site",
                "ready_to_finish": true,
                "can_finish_now": true,
                "checkpoints": [
                    { "id": "scan_landing_site", "title": "Complete a surface scan", "checked": true, "live_ready": true },
                    { "id": "mine_carbon_ore", "title": "Mine 1t carbon ore", "checked": true, "live_ready": true }
                ]
            }],
            "available": [],
            "completed": []
        }));
        let ready_txt = render_quest_tracker_lines_full(&ready, None, None, None, None)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !ready_txt.contains("Prep: +license"),
            "prep hint must hide once ready to turn in:\n{ready_txt}"
        );
    }

    #[test]
    fn canonical_guided_onboarding_pack_7step_e2e_walkthrough() {
        let raw_pack: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tacsim/content/quests/guided_onboarding.json"
        ))
        .expect("parse canonical guided_onboarding.json");
        let pack_id = raw_pack["manifest"]["id"].as_str().expect("pack id");
        assert_eq!(pack_id, "tacsim.quests.guided_onboarding");

        let maren = &raw_pack["entries"]["npc.maren_halvorsen"]["def"];
        assert_eq!(maren["name"].as_str(), Some("Maren Halvorsen"));
        assert_eq!(maren["room"].as_str(), Some("room.c03_arrivals"));

        let entry_names = [
            "quest.guide_01_coop_enroll",
            "quest.guide_02_provisions_fuel",
            "quest.guide_03_metals_delivery",
            "quest.guide_04_return_haul",
            "quest.guide_05_refit_prospector",
            "quest.guide_06_surface_mining",
            "quest.guide_07_sell_mined_ore",
        ];

        for (idx, name) in entry_names.iter().enumerate() {
            let full_key = format!("{pack_id}/{name}");
            assert_eq!(GUIDED_TOUR_KEYS[idx], full_key);
            let def = &raw_pack["entries"][name]["def"];
            assert_eq!(def["category"].as_str(), Some("guided"));
            assert_eq!(def["allow_restart"].as_bool(), Some(true));
        }

        let make_active = |name: &str,
                           giver: &str,
                           turn_in: &str,
                           checked_ids: &[&str],
                           ready: bool,
                           can_finish: bool|
         -> serde_json::Value {
            let def = &raw_pack["entries"][name]["def"];
            let cps_raw = def["checkpoints"].as_array().expect("checkpoints");
            let cps: Vec<serde_json::Value> = cps_raw
                .iter()
                .map(|cp| {
                    let id = cp["id"].as_str().unwrap_or("");
                    let done = checked_ids.contains(&id);
                    json!({
                        "id": id,
                        "title": cp["title"].as_str().unwrap_or(""),
                        "instructions": cp["instructions"].as_str().unwrap_or(""),
                        "room_entry": cp["room_entry"].as_str().unwrap_or(""),
                        "checked": done,
                        "live_ready": done
                    })
                })
                .collect();
            let next_room = cps
                .iter()
                .find(|c| !c["checked"].as_bool().unwrap_or(false))
                .or_else(|| cps.last())
                .and_then(|c| c["room_entry"].as_str())
                .unwrap_or("");
            json!({
                "key": format!("{pack_id}/{name}"),
                "title": def["title"].as_str().unwrap_or(""),
                "category": "guided",
                "introduction": def["introduction"].as_str().unwrap_or(""),
                "reward_summary": def["reward_summary"].as_str().unwrap_or(""),
                "giver_name": giver,
                "turn_in_name": turn_in,
                "allow_restart": true,
                "ready_to_finish": ready,
                "can_finish_now": can_finish,
                "turn_in_here": can_finish,
                "next_room_entry": next_room,
                "checkpoints": cps
            })
        };

        let mut milestones = QuestMilestoneTracker::default();
        let mut completed: Vec<serde_json::Value> = Vec::new();

        // 0. Spawn hydration at Ceres Arrivals -> Maren Halvorsen [!] offer, 0 milestones
        let spawn = parse_frame(&json!({
            "schema": 1,
            "room_name": "Port Watch Arrivals",
            "room_markers": [{
                "npc_id": "#42",
                "npc_name": "Maren Halvorsen",
                "marker": "available",
                "symbol": "!",
                "quest_key": GUIDED_TOUR_KEYS[0],
                "quest_title": raw_pack["entries"][entry_names[0]]["def"]["title"]
            }],
            "available": [{
                "key": GUIDED_TOUR_KEYS[0],
                "title": raw_pack["entries"][entry_names[0]]["def"]["title"],
                "category": "guided",
                "giver_name": "Maren Halvorsen",
                "allow_restart": true,
                "accept_cmd": "+quest/accept #42=tok1"
            }],
            "active": [],
            "completed": []
        }));
        assert!(milestones.observe(&spawn).is_empty());
        assert_eq!(
            spawn.guided_tour_progress().map(|p| (p.completed, p.total)),
            Some((0, 7))
        );

        // 1. Step 1: accept -> abandon (opt-out clean, restartable) -> re-accept -> complete
        let s1 = parse_frame(&json!({
            "schema": 1,
            "active": [make_active(entry_names[0], "Maren Halvorsen", "Ilse Oduya-Brandt", &[], false, false)],
            "available": [],
            "completed": completed
        }));
        let m1 = milestones.observe(&s1);
        assert_eq!(m1.len(), 1);
        assert_eq!(m1[0].kind, QuestMilestoneKind::Accepted);
        assert_eq!(
            QuestsSnapshot::suggested_action_for(&s1.active[0]).map(|a| a.command),
            Some("+coop/enroll ".to_string())
        );

        let s1_abandoned = parse_frame(
            &json!({ "schema": 1, "active": [], "available": [], "completed": completed }),
        );
        let m_ab = milestones.observe(&s1_abandoned);
        assert_eq!(m_ab.len(), 1);
        assert_eq!(m_ab[0].kind, QuestMilestoneKind::Abandoned);
        assert!(m_ab[0].restartable);
        assert!(s1_abandoned.next_guided_offer().is_none());

        milestones.observe(&s1);
        completed.push(json!({ "key": GUIDED_TOUR_KEYS[0], "title": s1.active[0].title }));
        let after_s1 = parse_frame(
            &json!({ "schema": 1, "active": [], "available": [], "completed": completed }),
        );
        let m_s1_done = milestones.observe(&after_s1);
        assert_eq!(m_s1_done[0].kind, QuestMilestoneKind::Completed);
        assert!(!m_s1_done[0].tour_all_done);
        assert_eq!(after_s1.next_guided_offer().map(|o| o.step), Some(2));

        // 2. Step 2: provisions (10) & propellant (5t) hold readiness
        let s2 = parse_frame(&json!({
            "schema": 1,
            "active": [make_active(entry_names[1], "Ilse Oduya-Brandt", "Ilse Oduya-Brandt", &[], false, false)],
            "available": [],
            "completed": completed
        }));
        milestones.observe(&s2);
        let s2_slugs: Vec<String> = s2
            .active_trade_needs(None)
            .into_iter()
            .map(|n| n.commodity_slug)
            .collect();
        assert_eq!(s2_slugs, vec!["provisions", "propellant"]);
        completed.push(json!({ "key": GUIDED_TOUR_KEYS[1], "title": s2.active[0].title }));

        // 3. Step 3: deliver_metals MUST extract "metals" (NOT shadowed by "provisions" in instructions!)
        let s3 = parse_frame(&json!({
            "schema": 1,
            "active": [make_active(entry_names[2], "Ilse Oduya-Brandt", "Jonas Achterberg-Nkemelu", &[], false, false)],
            "available": [],
            "completed": completed
        }));
        milestones.observe(&s3);
        let s3_need = s3.active_trade_need(None).expect("step 3 trade need");
        assert_eq!(s3_need.commodity_slug, "metals");
        assert_eq!(s3_need.target_quantity, 5);
        assert!(s3_need.wants_buy && s3_need.wants_sell);
        assert_eq!(
            s3.active_destination_port(None).map(|d| d.port_id),
            Some("stickney".to_string())
        );
        completed.push(json!({ "key": GUIDED_TOUR_KEYS[2], "title": s3.active[0].title }));

        // 4. Step 4: industrial_parts Stickney -> Ceres
        let s4 = parse_frame(&json!({
            "schema": 1,
            "active": [make_active(entry_names[3], "Jonas Achterberg-Nkemelu", "Ilse Oduya-Brandt", &[], false, false)],
            "available": [],
            "completed": completed
        }));
        milestones.observe(&s4);
        assert_eq!(
            s4.active_trade_need(None).map(|n| n.commodity_slug),
            Some("industrial_parts".to_string())
        );
        assert_eq!(
            s4.active_destination_port(None).map(|d| d.port_id),
            Some("ceres".to_string())
        );
        completed.push(json!({ "key": GUIDED_TOUR_KEYS[3], "title": s4.active[0].title }));

        // 5. Step 5: Dray-P refit empty-hold check (propellant OK)
        let s5 = parse_frame(&json!({
            "schema": 1,
            "active": [make_active(entry_names[4], "Ilse Oduya-Brandt", "Rosalind Qureshi-Mbeki", &[], false, false)],
            "available": [],
            "completed": completed
        }));
        milestones.observe(&s5);
        assert_eq!(
            QuestsSnapshot::suggested_action_for(&s5.active[0]).map(|a| a.command),
            Some("+coop/refit prospector".to_string())
        );
        let s5_blocked = QuestStowageTelemetry {
            cargo_kg: 2000,
            propellant_kg: 15_000,
            provisions_days: 8,
            cargo_lots: vec![("tacsim:metals".into(), "Metals".into(), 2)],
        };
        assert_eq!(
            QuestsSnapshot::hold_readiness_line_for(&s5.active[0], &s5_blocked).map(|(_, ok)| ok),
            Some(false)
        );
        let s5_clear = QuestStowageTelemetry {
            cargo_kg: 0,
            propellant_kg: 15_000,
            provisions_days: 0,
            cargo_lots: vec![],
        };
        assert_eq!(
            QuestsSnapshot::hold_readiness_line_for(&s5.active[0], &s5_clear).map(|(_, ok)| ok),
            Some(true)
        );
        completed.push(json!({ "key": GUIDED_TOUR_KEYS[4], "title": s5.active[0].title }));

        // 6. Step 6: Surface mining prep -> bay/bridge routing -> [ORE HOLD READY] -> Ceres return
        let s6_pre = parse_frame(&json!({
            "schema": 1,
            "active": [make_active(entry_names[5], "Rosalind Qureshi-Mbeki", "Rosalind Qureshi-Mbeki", &[], false, false)],
            "available": [],
            "completed": completed
        }));
        milestones.observe(&s6_pre);
        let m6 = s6_pre.surface_mining_status(None).expect("step 6 mining");
        assert_eq!(m6.target_ore_slug, "carbon_ore");
        assert_eq!(m6.recommended_family, "carbon");
        assert_eq!(
            m6.prep_hint_line(),
            "Prep: +license | +tv/lease | +chart/ground"
        );
        assert_eq!(
            s6_pre.active_ship_target_compartment_for_location(None, Some("ceres"), false),
            Some("bridge")
        );
        assert_eq!(
            s6_pre.active_ship_target_compartment_for_location(None, None, true),
            Some("bay")
        );

        let s6_post = parse_frame(&json!({
            "schema": 1,
            "active": [make_active(
                entry_names[5],
                "Rosalind Qureshi-Mbeki",
                "Rosalind Qureshi-Mbeki",
                &["scan_landing_site", "sound_mineral_pocket", "mine_carbon_ore"],
                true,
                false
            )],
            "available": [],
            "completed": completed
        }));
        assert_eq!(
            s6_post.active_ship_target_compartment_for_location(None, None, true),
            Some("bridge")
        );
        assert_eq!(
            s6_post.active_destination_port(None).map(|d| d.port_id),
            Some("ceres".to_string())
        );
        let ore_stow = QuestStowageTelemetry {
            cargo_kg: 1000,
            propellant_kg: 10_000,
            provisions_days: 6,
            cargo_lots: vec![("tacsim:carbon_ore".into(), "Carbon Ore".into(), 1)],
        };
        let (ore_line, ore_ok) =
            QuestsSnapshot::hold_readiness_line_for(&s6_post.active[0], &ore_stow)
                .expect("step 6 ore hold");
        assert!(ore_ok && ore_line.contains("[ORE HOLD READY]"));
        completed.push(json!({ "key": GUIDED_TOUR_KEYS[5], "title": s6_pre.active[0].title }));

        // 7. Step 7: Sell carbon_ore at Stickney Yards -> pre-flight check -> Corridor Captain (7/7)
        let s7 = parse_frame(&json!({
            "schema": 1,
            "active": [make_active(entry_names[6], "Rosalind Qureshi-Mbeki", "Wren Castillo-Haugen", &[], false, false)],
            "available": [],
            "completed": completed
        }));
        milestones.observe(&s7);
        let s7_need = s7.active_trade_need(None).expect("step 7 trade need");
        assert_eq!(s7_need.commodity_slug, "carbon_ore");
        assert!(!s7_need.wants_buy && s7_need.wants_sell);
        assert_eq!(
            s7.active_destination_port(None).map(|d| d.port_id),
            Some("stickney".to_string())
        );
        let s7_dry = QuestStowageTelemetry {
            cargo_kg: 2000,
            propellant_kg: 12_000,
            provisions_days: 0,
            cargo_lots: vec![("tacsim:carbon_ore".into(), "Carbon Ore".into(), 2)],
        };
        assert_eq!(
            QuestsSnapshot::preflight_readiness_line_for(&s7.active[0], &s7_dry).map(|(_, ok)| ok),
            Some(false)
        );
        let s7_ok = QuestStowageTelemetry {
            provisions_days: 15,
            ..s7_dry
        };
        assert_eq!(
            QuestsSnapshot::preflight_readiness_line_for(&s7.active[0], &s7_ok).map(|(_, ok)| ok),
            Some(true)
        );

        // Complete Step 7 -> 7/7 Corridor Captain graduation milestone + Jobs Board pointer
        completed.push(json!({ "key": GUIDED_TOUR_KEYS[6], "title": s7.active[0].title }));
        let done_all = parse_frame(&json!({
            "schema": 1,
            "active": [],
            "available": [],
            "completed": completed
        }));
        let m_final = milestones.observe(&done_all);
        assert_eq!(m_final.len(), 1);
        assert_eq!(m_final[0].kind, QuestMilestoneKind::Completed);
        assert!(m_final[0].tour_all_done);
        assert!(done_all.guided_tour_progress().is_some_and(|p| p.all_done));
        assert!(done_all.next_guided_offer().is_none());
        let sheet_txt = render_quest_sheet_lines_with_stowage(&done_all, None, None)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(sheet_txt.contains("Corridor Captain") && sheet_txt.contains("+market/jobs"));
    }

    #[test]
    fn step5_active_objective_room_with_stowage_routes_exchange_chandlery_then_registry() {
        let snap = parse_frame(&json!({
            "schema": 1,
            "active": [{
                "key": "quest.guide_05_refit_prospector",
                "title": "Refit the Dray-P",
                "category": "onboarding",
                "next_room_entry": "room.c05_registry",
                "next_room_name": "Co-op Registry",
                "next_step": "Empty hold and run +coop/refit prospector",
                "checkpoints": [{
                    "id": "refit",
                    "title": "Refit Dray-P at Registry",
                    "instructions": "Empty cargo and provisions, then run +coop/refit prospector",
                    "room_entry": "room.c05_registry",
                    "checked": false
                }]
            }],
            "available": [],
            "completed": []
        }));

        let with_cargo_and_stores = QuestStowageTelemetry {
            cargo_kg: 2000,
            propellant_kg: 5000,
            provisions_days: 10,
            cargo_lots: vec![("tacsim:metals".into(), "Metals".into(), 2)],
        };
        assert_eq!(
            snap.active_objective_room_with_stowage(None, Some(&with_cargo_and_stores)),
            Some(("room.c07_exchange".to_string(), false))
        );

        let stores_only = QuestStowageTelemetry {
            cargo_kg: 0,
            propellant_kg: 5000,
            provisions_days: 6,
            cargo_lots: vec![],
        };
        assert_eq!(
            snap.active_objective_room_with_stowage(None, Some(&stores_only)),
            Some(("room.c08_chandlery".to_string(), false))
        );

        let cleared_hold = QuestStowageTelemetry {
            cargo_kg: 0,
            propellant_kg: 5000,
            provisions_days: 0,
            cargo_lots: vec![],
        };
        assert_eq!(
            snap.active_objective_room_with_stowage(None, Some(&cleared_hold)),
            Some(("room.c05_registry".to_string(), false))
        );
    }
}
