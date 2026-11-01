//! **Autonomous Space Pilot & Panel-Driven Doctrine (`SF.11`)**
//! (`docs/design/35-ship-free-flight-combat-and-mining.md` §9, §10).
//!
//! Provides a deterministic, panel-feed-driven space pilot ([`SpacePilotDoctrine`])
//! that consumes the three spaceflight panel feeds (`space.holotank`,
//! `space.cockpit`, and `space.docking`) and emits live MUSH commands (`+helm/*`,
//! `+tac/*`, `+mine/*`) as well as 60 fps 6-DOF RCS control inputs
//! ([`DockingControlInput`]) to execute canonical Stories 1 and 2 end-to-end:
//!
//! - **Story 1 (Kirkwood 3:1 Customs Picket & Triad Ambush)**:
//!   Promotes into `kirkwood_picket`, detects the inspecting Republic Customs
//!   corvette on `space.holotank`, brakes to heave-to (`<= 10 m/s`), enables
//!   automatic point-defense (`+tac/pdc auto`) and defensive laser turrets
//!   (`+tac/laser defense`) upon raider ambush, executes a lateral jink burn
//!   (`+helm/burn`) and hull roll (`+helm/roll`) to defeat unguided railgun lead
//!   solutions, and steps the 1:1 wall-clock bubble (`+tac/step`) through clearing.
//! - **Story 2 (Pallas Prospecting, Claim-Jump Defense, 6-DOF Derelict Salvage &
//!   Outer-System Neptune Expedition)**:
//!   Enters `pallas_inclined_belt`, pings an M-type rock (`+mine/ping`), matches
//!   3D velocity (`+helm/match`), launches 4 collector drones (`+mine/drones`),
//!   extracts ore (`+mine/laser`), detects the hostile claim-jumper on
//!   `space.holotank`, disables its drive with offensive lasers (`+tac/beam`) into
//!   an open-epoch derelict, initiates mandatory 6-DOF hard-docking
//!   (`+helm/dock derelict`), steers 6-DOF collar alignment (`space.docking`) to
//!   clamp (`+helm/dock clamp`), attaches a 48-hour Freeport Registry salvage tow
//!   (`+helm/tow`), decouples into Tier-1 System Free Flight (`+helm/decouple`),
//!   fits long-range drop tanks (`+helm/droptanks`), flies an outer-system
//!   expedition to Neptune (`+helm/expedition neptune`), verifies Neptune & Triton
//!   in `space.cockpit`, and recouples autopilot (`+helm/autopilot`).

use mush_core::Audience;
use mush_proto::OutputEvent;
use serde_json::Value;

use super::commands::{SPACE_COCKPIT_FEED, SPACE_DOCKING_FEED, SPACE_HOLOTANK_FEED};
use super::docking::{
    DOCK_GATE_ATTITUDE_MDEG, DOCK_GATE_CLOSURE_MAX_MMPS, DOCK_GATE_CLOSURE_MIN_MMPS,
    DOCK_GATE_LATERAL_MM, DOCK_GATE_ROLL_MDEG, DockingControlInput,
};
use super::picket::{CUSTOMS_SHIP_ID, RAIDER_SHIP_ID};

/// A single contact parsed from a `space.holotank` JSON keyframe.
#[derive(Clone, Debug, PartialEq)]
pub struct HolotankContact {
    /// Ship or object identifier (`"hauler"`, `"customs_corvette"`, `"raider_cutter"`, `"claim_jumper"`).
    pub id: String,
    /// Display callsign.
    pub label: String,
    /// Contact kind (`"ship"` or `"derelict"`).
    pub kind: String,
    /// Diplomatic posture (`"self"`, `"friendly"`, `"inspecting"`, `"neutral"`, `"hostile"`).
    pub relation: String,
    /// Whether this contact is the observer's own hull.
    pub you: bool,
    /// Whether this contact is the observer's locked target.
    pub locked: bool,
    /// Position in kilometres `[x_km, y_km, z_km]`.
    pub pos_km: [f64; 3],
    /// Velocity in kilometres per second `[vx_kms, vy_kms, vz_kms]`.
    pub vel_kms: [f64; 3],
}

/// A guided torpedo track parsed from a `space.holotank` JSON keyframe.
#[derive(Clone, Debug, PartialEq)]
pub struct HolotankTorpedo {
    /// Torpedo ordnance ID.
    pub id: u64,
    /// Current position in kilometres `[x_km, y_km, z_km]`.
    pub pos_km: [f64; 3],
    /// Estimated time-to-impact in seconds.
    pub tti_s: i64,
}

/// Decoded snapshot of a `space.holotank` panel keyframe.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HolotankSnapshot {
    /// Bubble or plot title.
    pub title: String,
    /// Point-defense envelope radius in kilometres.
    pub pdc_range_km: f64,
    /// Tracked contacts in the bubble.
    pub contacts: Vec<HolotankContact>,
    /// In-flight guided torpedoes.
    pub torpedoes: Vec<HolotankTorpedo>,
}

impl HolotankSnapshot {
    /// Parse a `space.holotank` JSON keyframe payload.
    #[must_use]
    pub fn from_feed(value: &Value) -> Self {
        let title = value
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let pdc_range_km = value
            .get("pdc_range_km")
            .and_then(Value::as_f64)
            .unwrap_or(15.0);

        let contacts = value
            .get("contacts")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .map(|c| HolotankContact {
                        id: c
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        label: c
                            .get("label")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        kind: c
                            .get("kind")
                            .and_then(Value::as_str)
                            .unwrap_or("ship")
                            .to_owned(),
                        relation: c
                            .get("relation")
                            .and_then(Value::as_str)
                            .unwrap_or("neutral")
                            .to_owned(),
                        you: c.get("you").and_then(Value::as_bool).unwrap_or(false),
                        locked: c.get("locked").and_then(Value::as_bool).unwrap_or(false),
                        pos_km: [
                            c.get("x_km").and_then(Value::as_f64).unwrap_or(0.0),
                            c.get("y_km").and_then(Value::as_f64).unwrap_or(0.0),
                            c.get("z_km").and_then(Value::as_f64).unwrap_or(0.0),
                        ],
                        vel_kms: [
                            c.get("vx_kms").and_then(Value::as_f64).unwrap_or(0.0),
                            c.get("vy_kms").and_then(Value::as_f64).unwrap_or(0.0),
                            c.get("vz_kms").and_then(Value::as_f64).unwrap_or(0.0),
                        ],
                    })
                    .collect()
            })
            .unwrap_or_default();

        let torpedoes = value
            .get("torpedoes")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .map(|t| HolotankTorpedo {
                        id: t.get("id").and_then(Value::as_u64).unwrap_or(0),
                        pos_km: [
                            t.get("x_km").and_then(Value::as_f64).unwrap_or(0.0),
                            t.get("y_km").and_then(Value::as_f64).unwrap_or(0.0),
                            t.get("z_km").and_then(Value::as_f64).unwrap_or(0.0),
                        ],
                        tti_s: t.get("tti_s").and_then(Value::as_i64).unwrap_or(0),
                    })
                    .collect()
            })
            .unwrap_or_default();

        Self {
            title,
            pdc_range_km,
            contacts,
            torpedoes,
        }
    }

    /// Return the first hostile operational ship contact, if any.
    #[must_use]
    pub fn first_hostile_ship(&self) -> Option<&HolotankContact> {
        self.contacts
            .iter()
            .find(|c| !c.you && c.relation == "hostile" && c.kind == "ship")
    }

    /// Return the first disabled derelict contact in the bubble, if any.
    #[must_use]
    pub fn first_derelict(&self) -> Option<&HolotankContact> {
        self.contacts
            .iter()
            .find(|c| !c.you && c.kind == "derelict")
    }

    /// Return whether an inspecting or allied customs picket is present in the bubble.
    #[must_use]
    pub fn has_inspecting_picket(&self) -> bool {
        self.contacts.iter().any(|c| {
            !c.you
                && (c.id == CUSTOMS_SHIP_ID
                    || c.relation == "inspecting"
                    || (c.label.contains("VIGILANT")
                        && (c.relation == "neutral" || c.relation == "friend")))
        })
    }
}

/// Decoded snapshot of a `space.cockpit` bridge canopy keyframe.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CockpitSnapshot {
    /// Own ship callsign.
    pub callsign: String,
    /// Current attitude `(yaw_deg, pitch_deg, roll_deg)`.
    pub attitude_deg: (i64, i64, i64),
    /// Current speed in metres per second.
    pub speed_mps: i64,
    /// Remaining delta-v budget in metres per second.
    pub delta_v_mps: i64,
    /// Thermal load in permille (`0..=1000`).
    pub heat_permille: u16,
    /// Hull integrity percentage (`0..=100`).
    pub hull_pct: u8,
    /// Point-defense mode (`"AUTO"` or `"MAN"`).
    pub pdc_mode: String,
    /// Emissions control posture (`"ACT"`, `"PAS"`, `"SIL"`).
    pub emcon: String,
    /// Celestial body IDs visible on the bridge canopy (when in Tier-1 System Free Flight).
    pub visible_bodies: Vec<String>,
}

impl CockpitSnapshot {
    /// Parse a `space.cockpit` JSON keyframe payload.
    #[must_use]
    pub fn from_feed(value: &Value) -> Self {
        let callsign = value
            .get("callsign")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let yaw_deg = value.get("yaw_deg").and_then(Value::as_i64).unwrap_or(0);
        let pitch_deg = value.get("pitch_deg").and_then(Value::as_i64).unwrap_or(0);
        let roll_deg = value.get("roll_deg").and_then(Value::as_i64).unwrap_or(0);
        let speed_mps = value.get("speed_mps").and_then(Value::as_i64).unwrap_or(0);
        let delta_v_mps = value
            .get("delta_v_mps")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let heat_permille = value
            .get("heat_permille")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .min(1_000) as u16;
        let hull_pct = value
            .get("hull_pct")
            .and_then(Value::as_u64)
            .unwrap_or(100)
            .min(100) as u8;
        let pdc_mode = value
            .get("pdc_mode")
            .and_then(Value::as_str)
            .unwrap_or("MAN")
            .to_owned();
        let emcon = value
            .get("emcon")
            .and_then(Value::as_str)
            .unwrap_or("ACT")
            .to_owned();
        let visible_bodies = value
            .get("bodies")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|b| {
                        b.get("id")
                            .or_else(|| b.get("name"))
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    })
                    .collect()
            })
            .unwrap_or_default();

        Self {
            callsign,
            attitude_deg: (yaw_deg, pitch_deg, roll_deg),
            speed_mps,
            delta_v_mps,
            heat_permille,
            hull_pct,
            pdc_mode,
            emcon,
            visible_bodies,
        }
    }
}

/// Decoded snapshot of a `space.docking` 6-DOF bow camera keyframe.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DockingSnapshot {
    /// Active docking session ID.
    pub session_id: String,
    /// Target collar label.
    pub target_label: String,
    /// Target kind (`"port"`, `"derelict"`, `"ship"`).
    pub target_kind: String,
    /// Whether manual 6-DOF alignment is mandatory.
    pub mandatory: bool,
    /// Whether the hull is paused from the world tick during the approach.
    pub paused_from_world_tick: bool,
    /// Lateral horizontal offset in millimetres.
    pub dx_mm: i64,
    /// Lateral vertical offset in millimetres.
    pub dy_mm: i64,
    /// Forward axial range to collar plane in millimetres.
    pub dz_mm: i64,
    /// Pitch misalignment in millidegrees.
    pub pitch_mdeg: i64,
    /// Yaw misalignment in millidegrees.
    pub yaw_mdeg: i64,
    /// Roll misalignment in millidegrees.
    pub roll_mdeg: i64,
    /// Lateral horizontal velocity in millimetres per second.
    pub vx_mmps: i64,
    /// Lateral vertical velocity in millimetres per second.
    pub vy_mmps: i64,
    /// Forward closure speed toward the collar in millimetres per second.
    pub vz_mmps: i64,
    /// Whether automatic RCS dampeners are active.
    pub dampeners: bool,
    /// Session status string (`"STANDBY"`, `"APPROACHING"`, `"POSE-ALIGNED"`, `"ALIGNED-READY"`, `"CLAMPED"`, `"COLLAR-SCRAPE"`, `"ABORTED"`).
    pub status: String,
}

impl DockingSnapshot {
    /// Parse a `space.docking` JSON keyframe payload.
    #[must_use]
    pub fn from_feed(value: &Value) -> Self {
        Self {
            session_id: value
                .get("session_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            target_label: value
                .get("target_label")
                .or_else(|| value.get("target_name"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            target_kind: value
                .get("target_kind")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            mandatory: value
                .get("mandatory")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            paused_from_world_tick: value
                .get("paused_from_world_tick")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            dx_mm: value.get("dx_mm").and_then(Value::as_i64).unwrap_or(0),
            dy_mm: value.get("dy_mm").and_then(Value::as_i64).unwrap_or(0),
            dz_mm: value.get("dz_mm").and_then(Value::as_i64).unwrap_or(2_500),
            pitch_mdeg: value.get("pitch_mdeg").and_then(Value::as_i64).unwrap_or(0),
            yaw_mdeg: value.get("yaw_mdeg").and_then(Value::as_i64).unwrap_or(0),
            roll_mdeg: value.get("roll_mdeg").and_then(Value::as_i64).unwrap_or(0),
            vx_mmps: value.get("vx_mmps").and_then(Value::as_i64).unwrap_or(0),
            vy_mmps: value.get("vy_mmps").and_then(Value::as_i64).unwrap_or(0),
            vz_mmps: value.get("vz_mmps").and_then(Value::as_i64).unwrap_or(0),
            dampeners: value
                .get("dampeners")
                .or_else(|| value.get("rcs_dampeners"))
                .and_then(Value::as_bool)
                .unwrap_or(true),
            status: value
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("STANDBY")
                .to_owned(),
        }
    }
}

/// Compute a single 60 fps closed-loop 6-DOF RCS control input ([`DockingControlInput`])
/// from a [`DockingSnapshot`] so an automated pilot trims lateral offset, pitch/yaw/roll
/// tumble, and closure speed into the green clamp envelope (`D-35-14`).
#[must_use]
pub fn compute_6dof_rcs_control(dock: &DockingSnapshot) -> DockingControlInput {
    let trans_x = if dock.dx_mm > DOCK_GATE_LATERAL_MM / 2 {
        -1
    } else if dock.dx_mm < -(DOCK_GATE_LATERAL_MM / 2) {
        1
    } else {
        0
    };

    let trans_y = if dock.dy_mm > DOCK_GATE_LATERAL_MM / 2 {
        -1
    } else if dock.dy_mm < -(DOCK_GATE_LATERAL_MM / 2) {
        1
    } else {
        0
    };

    let pitch_trim = if dock.pitch_mdeg > DOCK_GATE_ATTITUDE_MDEG / 3 {
        -1
    } else if dock.pitch_mdeg < -(DOCK_GATE_ATTITUDE_MDEG / 3) {
        1
    } else {
        0
    };

    let yaw_trim = if dock.yaw_mdeg > DOCK_GATE_ATTITUDE_MDEG / 3 {
        -1
    } else if dock.yaw_mdeg < -(DOCK_GATE_ATTITUDE_MDEG / 3) {
        1
    } else {
        0
    };

    let roll_trim = if dock.roll_mdeg > DOCK_GATE_ROLL_MDEG / 3 {
        -1
    } else if dock.roll_mdeg < -(DOCK_GATE_ROLL_MDEG / 3) {
        1
    } else {
        0
    };

    let target_vz_mmps = (DOCK_GATE_CLOSURE_MIN_MMPS + DOCK_GATE_CLOSURE_MAX_MMPS) / 2;
    let closure_pulse = if dock.vz_mmps < DOCK_GATE_CLOSURE_MIN_MMPS + 80 {
        1
    } else if dock.vz_mmps > DOCK_GATE_CLOSURE_MAX_MMPS - 80 || dock.vz_mmps > target_vz_mmps + 120
    {
        -1
    } else {
        0
    };

    DockingControlInput {
        trans_x,
        trans_y,
        closure_pulse,
        pitch_trim,
        yaw_trim,
        roll_trim,
        toggle_dampeners: !dock.dampeners,
        abort: false,
    }
}

/// High-level mission goal assigned to [`SpacePilotDoctrine`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpaceMissionGoal {
    /// **Story 1**: Transit through the Kirkwood 3:1 Customs Picket, heave-to for
    /// customs inspection, and survive the Belt Raider torpedo/railgun ambush.
    KirkwoodPicketTransit,
    /// **Story 2 (Part A)**: Enter an orbital asteroid cluster, prospect and mine a
    /// target rock, disable any claim-jumping attacker into a derelict, hard-dock
    /// via 6-DOF bow camera, and attach a 48-hour Freeport Registry salvage tow.
    ClusterProspectAndSalvage {
        /// Canonical orbital asteroid cluster ID (`"pallas_inclined_belt"`).
        cluster_id: String,
        /// Target asteroid pocket ID (`"rock.m_309"`).
        target_rock_id: String,
    },
    /// **Story 2 (Part B)**: Decouple from a transit leg into Tier-1 System Free
    /// Flight, fit external drop tanks, execute an outer-system expedition burn to
    /// `expedition_body` (`"neptune"`), and recouple to the port autopilot.
    OuterSystemExpedition {
        /// Origin port/body (`"ceres"`).
        from_body: String,
        /// Destination port/body (`"mars"`).
        to_body: String,
        /// Outer-system flyby target (`"neptune"`).
        expedition_body: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PicketStage {
    NotEntered,
    EnteredNeedBrake,
    BrakedNeedStepAndDefense,
    AmbushDefenseComplete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClusterStage {
    NotEntered,
    EnteredNeedProspectAndMine,
    MinedNeedDisableClaimJumper,
    DerelictReadyNeedDock,
    DockingActiveNeedClamp,
    ClampedNeedTow,
    SalvageTowComplete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExpeditionStage {
    NotDecoupled,
    DecoupledNeedDropTanksAndFlyby,
    AtOuterBodyNeedRecouple,
    RecoupledComplete,
}

/// Deterministic autonomous pilot that consumes `space.holotank`, `space.cockpit`,
/// and `space.docking` panel feeds and produces MUSH commands (`+helm/*`, `+tac/*`,
/// `+mine/*`) to complete [`SpaceMissionGoal`]s.
#[derive(Clone, Debug)]
pub struct SpacePilotDoctrine {
    goal: SpaceMissionGoal,
    picket_stage: PicketStage,
    cluster_stage: ClusterStage,
    expedition_stage: ExpeditionStage,
    /// Latest decoded `space.holotank` keyframe observed by the pilot.
    pub latest_holotank: Option<HolotankSnapshot>,
    /// Latest decoded `space.cockpit` keyframe observed by the pilot.
    pub latest_cockpit: Option<CockpitSnapshot>,
    /// Latest decoded `space.docking` keyframe observed by the pilot.
    pub latest_docking: Option<DockingSnapshot>,
    /// Total panel keyframes ingested across the mission.
    pub keyframes_ingested: usize,
    /// Complete ordered transcript of MUSH commands issued by the pilot.
    pub issued_commands: Vec<String>,
}

impl SpacePilotDoctrine {
    /// Construct a new autonomous space pilot for `goal`.
    #[must_use]
    pub fn new(goal: SpaceMissionGoal) -> Self {
        Self {
            goal,
            picket_stage: PicketStage::NotEntered,
            cluster_stage: ClusterStage::NotEntered,
            expedition_stage: ExpeditionStage::NotDecoupled,
            latest_holotank: None,
            latest_cockpit: None,
            latest_docking: None,
            keyframes_ingested: 0,
            issued_commands: Vec::new(),
        }
    }

    /// Switch the pilot to a new [`SpaceMissionGoal`] while preserving its ingested feed history.
    pub fn set_goal(&mut self, goal: SpaceMissionGoal) {
        self.goal = goal;
        self.picket_stage = PicketStage::NotEntered;
        self.cluster_stage = ClusterStage::NotEntered;
        self.expedition_stage = ExpeditionStage::NotDecoupled;
    }

    /// Ingest a slice of `(Audience, OutputEvent)` pairs returned by `TestWorld::run`
    /// (or the live gateway), updating [`Self::latest_holotank`], [`Self::latest_cockpit`],
    /// and [`Self::latest_docking`].
    pub fn observe_events(&mut self, events: &[(Audience, OutputEvent)]) {
        for (_, ev) in events {
            if let OutputEvent::Panel { feed, data, .. } = ev {
                match feed.as_str() {
                    SPACE_HOLOTANK_FEED => {
                        self.latest_holotank = Some(HolotankSnapshot::from_feed(data));
                        self.keyframes_ingested += 1;
                    }
                    SPACE_COCKPIT_FEED => {
                        self.latest_cockpit = Some(CockpitSnapshot::from_feed(data));
                        self.keyframes_ingested += 1;
                    }
                    SPACE_DOCKING_FEED => {
                        self.latest_docking = Some(DockingSnapshot::from_feed(data));
                        self.keyframes_ingested += 1;
                    }
                    _ => {}
                }
            }
        }
    }

    /// Return whether the current [`SpaceMissionGoal`] has reached its terminal completion state.
    #[must_use]
    pub fn is_goal_complete(&self) -> bool {
        match &self.goal {
            SpaceMissionGoal::KirkwoodPicketTransit => {
                self.picket_stage == PicketStage::AmbushDefenseComplete
            }
            SpaceMissionGoal::ClusterProspectAndSalvage { .. } => {
                self.cluster_stage == ClusterStage::SalvageTowComplete
            }
            SpaceMissionGoal::OuterSystemExpedition { .. } => {
                self.expedition_stage == ExpeditionStage::RecoupledComplete
            }
        }
    }

    /// Decide the next batch of MUSH commands (`+helm/*`, `+tac/*`, `+mine/*`) based on the
    /// latest panel feeds and current mission stage.
    pub fn decide_next_commands(&mut self) -> Vec<String> {
        let cmds = match &self.goal {
            SpaceMissionGoal::KirkwoodPicketTransit => self.decide_picket_commands(),
            SpaceMissionGoal::ClusterProspectAndSalvage {
                cluster_id,
                target_rock_id,
            } => {
                let cid = cluster_id.clone();
                let rid = target_rock_id.clone();
                self.decide_cluster_commands(&cid, &rid)
            }
            SpaceMissionGoal::OuterSystemExpedition {
                from_body,
                to_body,
                expedition_body,
            } => {
                let fb = from_body.clone();
                let tb = to_body.clone();
                let eb = expedition_body.clone();
                self.decide_expedition_commands(&fb, &tb, &eb)
            }
        };
        self.issued_commands.extend(cmds.iter().cloned());
        cmds
    }

    fn decide_picket_commands(&mut self) -> Vec<String> {
        match self.picket_stage {
            PicketStage::NotEntered => {
                self.picket_stage = PicketStage::EnteredNeedBrake;
                vec!["+helm/enter kirkwood_picket".to_owned()]
            }
            PicketStage::EnteredNeedBrake => {
                let has_picket = self
                    .latest_holotank
                    .as_ref()
                    .is_some_and(HolotankSnapshot::has_inspecting_picket);
                let speed = self.latest_cockpit.as_ref().map_or(20, |c| c.speed_mps);
                self.picket_stage = PicketStage::BrakedNeedStepAndDefense;
                if has_picket && speed > 10 {
                    vec!["+helm/burn -15 0 0".to_owned(), "+tac/step 1000".to_owned()]
                } else {
                    vec!["+tac/step 1000".to_owned()]
                }
            }
            PicketStage::BrakedNeedStepAndDefense => {
                let hostile_id = self
                    .latest_holotank
                    .as_ref()
                    .and_then(HolotankSnapshot::first_hostile_ship)
                    .map(|c| c.id.clone())
                    .unwrap_or_else(|| RAIDER_SHIP_ID.to_owned());
                self.picket_stage = PicketStage::AmbushDefenseComplete;
                vec![
                    "+tac/pdc auto".to_owned(),
                    "+tac/laser defense".to_owned(),
                    "+helm/burn 0 12 0".to_owned(),
                    "+helm/roll 0 0 90".to_owned(),
                    format!("+tac/rail {hostile_id}"),
                    "+tac/step 1000".to_owned(),
                ]
            }
            PicketStage::AmbushDefenseComplete => Vec::new(),
        }
    }

    fn decide_cluster_commands(&mut self, cluster_id: &str, rock_id: &str) -> Vec<String> {
        match self.cluster_stage {
            ClusterStage::NotEntered => {
                self.cluster_stage = ClusterStage::EnteredNeedProspectAndMine;
                vec![format!("+helm/enter {cluster_id}")]
            }
            ClusterStage::EnteredNeedProspectAndMine => {
                self.cluster_stage = ClusterStage::MinedNeedDisableClaimJumper;
                vec![
                    format!("+mine/ping {rock_id}"),
                    format!("+helm/match {rock_id}"),
                    format!("+mine/drones {rock_id} 4"),
                    format!("+mine/laser {rock_id} 4000"),
                ]
            }
            ClusterStage::MinedNeedDisableClaimJumper => {
                if let Some(derelict) = self
                    .latest_holotank
                    .as_ref()
                    .and_then(HolotankSnapshot::first_derelict)
                {
                    let did = derelict.id.clone();
                    self.cluster_stage = ClusterStage::DockingActiveNeedClamp;
                    return vec![format!("+helm/dock derelict {did}")];
                }
                let hostile_id = self
                    .latest_holotank
                    .as_ref()
                    .and_then(HolotankSnapshot::first_hostile_ship)
                    .map(|c| c.id.clone())
                    .unwrap_or_else(|| "claim_jumper".to_owned());
                self.cluster_stage = ClusterStage::DerelictReadyNeedDock;
                vec![
                    "+tac/pdc auto".to_owned(),
                    format!("+tac/beam {hostile_id}"),
                ]
            }
            ClusterStage::DerelictReadyNeedDock => {
                let derelict_id = self
                    .latest_holotank
                    .as_ref()
                    .and_then(HolotankSnapshot::first_derelict)
                    .map(|c| c.id.clone())
                    .unwrap_or_else(|| "claim_jumper".to_owned());
                self.cluster_stage = ClusterStage::DockingActiveNeedClamp;
                vec![format!("+helm/dock derelict {derelict_id}")]
            }
            ClusterStage::DockingActiveNeedClamp => {
                self.cluster_stage = ClusterStage::ClampedNeedTow;
                vec!["+helm/dock clamp".to_owned()]
            }
            ClusterStage::ClampedNeedTow => {
                self.cluster_stage = ClusterStage::SalvageTowComplete;
                vec!["+helm/tow".to_owned()]
            }
            ClusterStage::SalvageTowComplete => Vec::new(),
        }
    }

    fn decide_expedition_commands(
        &mut self,
        from_body: &str,
        to_body: &str,
        expedition_body: &str,
    ) -> Vec<String> {
        match self.expedition_stage {
            ExpeditionStage::NotDecoupled => {
                self.expedition_stage = ExpeditionStage::DecoupledNeedDropTanksAndFlyby;
                vec![format!("+helm/decouple {from_body} {to_body}")]
            }
            ExpeditionStage::DecoupledNeedDropTanksAndFlyby => {
                self.expedition_stage = ExpeditionStage::AtOuterBodyNeedRecouple;
                vec![
                    "+helm/droptanks".to_owned(),
                    format!("+helm/expedition {expedition_body}"),
                ]
            }
            ExpeditionStage::AtOuterBodyNeedRecouple => {
                self.expedition_stage = ExpeditionStage::RecoupledComplete;
                vec![format!("+helm/autopilot {from_body}")]
            }
            ExpeditionStage::RecoupledComplete => Vec::new(),
        }
    }
}
