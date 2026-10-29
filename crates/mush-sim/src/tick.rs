//! # The minimal deterministic pulse driver — E7 cut 8 (10b §3.8)
//!
//! One promoted domain, one `ClockRef::DomainTick` scalar, one frozen
//! `ActionWindowRecord` per tick, and **one deterministic domain-tick commit
//! per domain per tick** with recorded RNG.
//!
//! ## The retired combat lane (16b cut T, D-16-20)
//!
//! The `tactical_intent` submit action, its document, its provider and the
//! placeholder resolution that consumed staged intents were **retired with cut
//! T**: combat is the arena's (`crates/tacsim/src/arena/`), which is the single
//! writer for its own facts while a match runs. What is left here is the
//! world-side pulse the non-combat domains use — journeys, placement, the
//! windows and the passive driver — so nothing submits into the tick window
//! today and the commit records an empty resolution.
//!
//! ## What this module builds, and what it only drives
//!
//! Cut 8 **defines**: [`DomainClock::Pulse`]
//! (in `data.rs`), the production [`SimDomainClockProvider`], the
//! [`StandingOrder`] family, the [`DomainTickCommit`] family, the `domain_tick`
//! resolution action, the `domaintick` schedule source, and the boundary facts
//! that leave the bundle through one `mush-tactics`-owned durable outbox
//! adapter.
//!
//! Cut 8 **drives, and never re-implements**: K's `ClockProviderRegistry`,
//! `Registrar::domain_clock_provider`, `ClockRef::DomainTick`,
//! `ActionWindowRecord::{checked_submit, resolution_order, checked_close_due,
//! checked_consume}`, `PrepareCtx::start_resolution_at`, `ResolutionRecord`,
//! `scheduled_action_ingress`, `ScheduleActionAdapter`, `DurableScheduleDriver`,
//! `DurableOutboxIntent` and the K3 bundle caps.
//!
//! ## The one binding constraint (08i §9 answers 4/G-A; 10b D-E7-37)
//!
//! **The `ClockProviderRegistry` is composition-frozen and cut 8 never mutates
//! it at runtime.** Exactly one owner-scoped provider is registered for
//! `mush-sim` at composition through `Registrar::domain_clock_provider`; every
//! runtime domain is answered *by id* out of that one registration. Promotion
//! (cut 6's `@domain/promote`, which writes a `DomainActivation` **request**)
//! and cut 8's applying authority both leave the registry byte-identical — a
//! probe snapshots its fingerprint before and after.
//!
//! ## Why the tick is two halves
//!
//! A `schedule_source`-fired action runs inside `prepare_authoritative` with
//! **no `&World`, no `Ctx` and no `Outbox`**, and writes only typed
//! `state_family` records through an `ActionTxn` (10b §4.0d's finding, which
//! cuts 6 and 7 both met). So the *deterministic* half — the frozen window, the
//! recorded RNG, the ordered deltas, the state hash, the boundary facts — is the
//! prepared authoritative commit built here, and every legacy-lane consequence
//! (rendering, projection, paging) reads the committed record afterwards. That
//! is the same split RESOLUTION A gave cuts 6 and 7, for the same kernel reason.
//!
//! ## The one tick function
//!
//! [`DomainClock::tick_at`](crate::data::DomainClock::tick_at) is the *only*
//! place a domain-tick scalar is computed. The composition-registered clock
//! provider calls it; so does every command that needs "which tick is this
//! submission in?". A reader and the kernel therefore cannot disagree about
//! which window an intent belongs to, which is the whole determinism argument.

use std::collections::BTreeMap;

use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionPresentation, ActionProvider,
    ActionStateError, ActionTxn, ActionWindow, ActionWindowDraft, ActionWindowGuardValidator,
    ActionWindowId, ActionWindowRecord, ActionWindowStateCodec, AuthorityGuardKind, CanonicalHash,
    CanonicalId, ClockProviderContext, ClockProviderFailure, ClockRef, ClockStamp,
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, DurableSchedule,
    DurableScheduleDraft, DurableScheduleStateCodec, Engine, LegalActionQuery, ModError, ObjId,
    OwnerId, PerceivedRef, ProjectedParameter, ProjectedParameterKind, ProjectedParameterSchema,
    ProjectedTargetSchema, ProviderOfferDraft, Registrar, RequestUid, ResolvedActionInvocation,
    RevisionSet, RulesValue, ScheduleActionAdapter, ScheduleActionDraft, ScheduleAdvance,
    ScheduleCatchUpOrder, ScheduleEvaluation, ScheduleId, ScheduleRestartPolicy,
    StateFamilyDescriptor, StateFamilyHandle, StateFamilyId, SubjectKey, WindowMode,
    WindowRestartPolicy, WindowTimeoutPolicy, action_window_state_family_descriptor,
    canonical_json_bytes_with_limit, durable_schedule_state_family_descriptor,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::NS;
use crate::data::{DomainClock, DomainId, DomainRecord, DomainStatus, SpatialState};

// ---- E7c8: constants ----

/// The schedule-source id. **Disjoint** from `control_handoff` (cut 5),
/// `production` (cut 4), `npc` (H-series) and cut 7's six `sensor_*` sources —
/// one source per family per `OwnerId` (10b §3.8 OPEN 1).
pub const SCHEDULE_SOURCE: &str = "domaintick";
/// The resolution action fired by the schedule source.
pub const TICK_ACTION_ID: &str = "domain_tick";
/// The action that arms or disarms one domain's durable pulse cursor.
pub const ARM_ACTION_ID: &str = "domain_pulse_arm";
/// The arm action's outcome definition.
pub const ARM_OUTCOME_ID: &str = "domain_pulse_armed";
/// The world-side maintenance tick's tag.
pub const PULSE_TICK_TAG: &str = "sim.domain_pulse";
/// How often the world-side maintenance tick fires, in seconds.
pub const PULSE_TICK_SECS: u64 = 1;
/// Domains one maintenance fire may visit (D-LBK-5: bounded on records
/// **visited**, and `MAX_ACTIVE_DOMAINS` is 8, so this never binds in practice).
pub const MAX_DOMAINS_PER_PULSE_FIRE: usize = 32;
/// The tick's outcome definition.
pub const TICK_OUTCOME_ID: &str = "domain_ticked";

/// The per-domain intent window family (K's `ActionWindowRecord`).
pub const WINDOW_FAMILY: &str = "tick_windows";
/// The per-domain durable pulse cursor family.
pub const SCHEDULE_FAMILY: &str = "domaintick_schedules";
/// The per-`(domain, tick)` deterministic commit family.
pub const COMMIT_FAMILY: &str = "domain_tick_commits";
/// The per-`(unit, channel)` standing-order family.
pub const ORDER_FAMILY: &str = "standing_orders";
/// Commit-coupled progress through the three bounded retention scans.
pub const REAP_FAMILY: &str = "tick_reap_progress";

/// **The measured unit cap (B5, §9 q6, §7).**
///
/// Not a guess and not a derived ceiling: [`measure_tick_bundle`] encodes a
/// worst-case tick for N units — one `StandingOrder` post-image per unit, the
/// window consume, the commit record with its ordered deltas and recorded draws,
/// and the boundary facts — and reports the encoded size against all three K8
/// caps. `e7c8_b5_*` pins this constant at **under 60 %** of the nearest one and
/// fails when a change pushes past it. A domain over the cap **refuses
/// promotion and pages with the measured size**; it is never truncated.
pub const MAX_UNITS_PER_DOMAIN: usize = 64;

/// `MAX_INTENTS_PER_TICK` **is** K's `MAX_WINDOW_CAPACITY` (§7): cut 8 defines
/// no second constant, it names K's.
pub const MAX_INTENTS_PER_TICK: u32 = mush_core::MAX_WINDOW_CAPACITY;

/// Commits retained per domain. `@domain/replay` is bounded by this, and the
/// tick drops the record that falls out of the ring **in its own bundle**, so a
/// domain pulsing every two seconds cannot grow an unbounded family.
pub const MAX_RETAINED_TICK_COMMITS: u64 = 64;
/// Boundary facts one tick may emit (each becomes one outbox intent).
pub const MAX_BOUNDARY_FACTS_PER_TICK: usize = 64;
/// The canonical encoding cap on one commit document.
pub const MAX_TICK_COMMIT_BYTES: usize = 256 * 1024;
/// The canonical encoding cap on one standing-order document.
pub const MAX_STANDING_ORDER_BYTES: usize = 4 * 1024;
/// Deltas one commit may carry (one per accepted intent, at the window cap).
pub const MAX_TICK_DELTAS: usize = MAX_INTENTS_PER_TICK as usize;

/// **The per-fire reap cap** (E7c8 fold D5(a)). Records the adapter may name for
/// removal in one tick, per family. Bounded on records **visited** (D-LBK-5) and
/// **resumable**: whatever is left over is named again by the next fire, two
/// seconds later, so the ring converges rather than truncating.
pub const MAX_REAP_PER_FIRE: usize = 64;
/// Records one reap scan visits per family per fire.
pub const MAX_REAP_SCAN_RECORDS: usize = 256;
/// The behavior that marks a domain host as a **space** domain, so its pulse
/// takes `[tacsim].space_pulse_ms` rather than the ground cadence (fold B9).
pub const SPACE_DOMAIN_BEHAVIOR: &str = "sim.space";

// ----------------------------------------------------------------------
// B3 — standing orders (the continuous half of S2A)
// ----------------------------------------------------------------------

/// The continuous control channels an order persists on (S2A: heading, thrust,
/// speed and sensor posture **persist until changed**; fire, launch, jump, dock,
/// eject and damage control are bounded discrete intents and are never stored
/// here).
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderChannel {
    /// A heading vector.
    Heading,
    /// A thrust setting.
    Thrust,
    /// A speed setting.
    Speed,
    /// An emission-control posture.
    Posture,
}

impl OrderChannel {
    /// Every channel, in canonical order.
    #[must_use]
    pub const fn all() -> [OrderChannel; 4] {
        [
            OrderChannel::Heading,
            OrderChannel::Thrust,
            OrderChannel::Speed,
            OrderChannel::Posture,
        ]
    }

    /// The staff/player-facing token.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            OrderChannel::Heading => "heading",
            OrderChannel::Thrust => "thrust",
            OrderChannel::Speed => "speed",
            OrderChannel::Posture => "posture",
        }
    }

    /// Parse a token; `None` refuses rather than guessing a channel.
    #[must_use]
    pub fn parse(token: &str) -> Option<OrderChannel> {
        OrderChannel::all()
            .into_iter()
            .find(|channel| channel.token() == token.trim().to_ascii_lowercase())
    }
}

/// The lowest and highest scalar an order may carry. A stored value outside the
/// band is refused **on load**, never clamped.
pub const MIN_ORDER_SCALAR: i32 = -1_000;
/// See [`MIN_ORDER_SCALAR`].
pub const MAX_ORDER_SCALAR: i32 = 1_000;

/// Why a tick record refused to be constructed or to re-enter.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TickRecordError {
    /// A scalar outside `MIN_ORDER_SCALAR..=MAX_ORDER_SCALAR`.
    #[error("an order scalar must be {MIN_ORDER_SCALAR}..={MAX_ORDER_SCALAR}")]
    ScalarOutOfRange,
    /// The value does not belong on the channel it was filed under.
    #[error("that order value does not belong on that control channel")]
    ChannelValueMismatch,
    /// A revision counter of zero can never have been written.
    #[error("a tick record revision must be non-zero")]
    ZeroRevision,
    /// A stamp on the wrong clock family.
    #[error("a tick record must be stamped on ClockRef::DomainTick")]
    NotDomainTick,
    /// More deltas than the window can ever have admitted.
    #[error("a domain tick commit carries at most {MAX_TICK_DELTAS} deltas")]
    TooManyDeltas,
    /// More boundary facts than one tick may emit.
    #[error("a domain tick commit carries at most {MAX_BOUNDARY_FACTS_PER_TICK} facts")]
    TooManyFacts,
    /// The document will not encode inside its cap.
    #[error("a tick record exceeds its canonical encoding limit")]
    TooLarge,
    /// The stored hash disagrees with the one its own contents derive.
    #[error("a domain tick commit's state hash does not match its contents")]
    HashMismatch,
}

/// One order value. Typed rather than free text, so the wire re-runs a real
/// invariant rather than a length check.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OrderValue {
    /// A heading vector (`Heading` only).
    Vector {
        /// x component.
        x: i32,
        /// y component.
        y: i32,
        /// z component.
        z: i32,
    },
    /// A scalar setting (`Thrust`/`Speed` only).
    Scalar {
        /// The setting.
        value: i32,
    },
    /// An emission-control level 0..=3 (`Posture` only).
    Emcon {
        /// The level.
        level: u8,
    },
}

impl OrderValue {
    /// Whether this value belongs on `channel`.
    #[must_use]
    pub const fn fits(&self, channel: OrderChannel) -> bool {
        matches!(
            (self, channel),
            (OrderValue::Vector { .. }, OrderChannel::Heading)
                | (
                    OrderValue::Scalar { .. },
                    OrderChannel::Thrust | OrderChannel::Speed
                )
                | (OrderValue::Emcon { .. }, OrderChannel::Posture)
        )
    }

    fn validate(&self) -> Result<(), TickRecordError> {
        let band = MIN_ORDER_SCALAR..=MAX_ORDER_SCALAR;
        match self {
            OrderValue::Vector { x, y, z } => {
                if band.contains(x) && band.contains(y) && band.contains(z) {
                    Ok(())
                } else {
                    Err(TickRecordError::ScalarOutOfRange)
                }
            }
            OrderValue::Scalar { value } => {
                if band.contains(value) {
                    Ok(())
                } else {
                    Err(TickRecordError::ScalarOutOfRange)
                }
            }
            OrderValue::Emcon { level } => {
                if *level <= 3 {
                    Ok(())
                } else {
                    Err(TickRecordError::ScalarOutOfRange)
                }
            }
        }
    }

    /// A short rendering for a player-facing line.
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            OrderValue::Vector { x, y, z } => format!("{x},{y},{z}"),
            OrderValue::Scalar { value } => value.to_string(),
            OrderValue::Emcon { level } => format!("emcon {level}"),
        }
    }
}

/// The `(unit, channel)` key one standing order occupies. Latest order wins:
/// a second order on the same key **replaces** the first.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StandingOrderKey {
    /// The commanded unit.
    pub unit: ObjId,
    /// The control channel.
    pub channel: OrderChannel,
}

impl StandingOrderKey {
    /// The canonical encoded key.
    #[must_use]
    pub fn encoded(&self) -> String {
        format!("{}:{}", self.unit.0, self.channel.token())
    }
}

/// A continuous order that persists until changed (S2A; 10b §3.8's
/// `StandingOrder { unit, channel, value, set_at, by }`).
///
/// This is the tick's **per-unit authoritative post-image**: a domain of N units
/// with an order each writes N of these in one bundle, which is exactly the
/// shape [`measure_tick_bundle`] encodes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(into = "StandingOrderWire")]
pub struct StandingOrder {
    unit: ObjId,
    channel: OrderChannel,
    value: OrderValue,
    set_at: ClockStamp,
    by: ObjId,
    revision: u64,
}

/// The strict wire form of [`StandingOrder`].
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StandingOrderWire {
    /// The commanded unit.
    pub unit: ObjId,
    /// The control channel.
    pub channel: OrderChannel,
    /// The value.
    pub value: OrderValue,
    /// The domain-tick stamp it was set at.
    pub set_at: ClockStamp,
    /// The actor that set it.
    pub by: ObjId,
    /// Bumped on every change.
    pub revision: u64,
}

impl StandingOrder {
    /// The one constructor.
    ///
    /// # Errors
    ///
    /// [`TickRecordError`] for a value off its band, a value on the wrong
    /// channel, a revision of zero, or a stamp that is not a domain tick.
    pub fn new(
        unit: ObjId,
        channel: OrderChannel,
        value: OrderValue,
        set_at: ClockStamp,
        by: ObjId,
        revision: u64,
    ) -> Result<StandingOrder, TickRecordError> {
        value.validate()?;
        if !value.fits(channel) {
            return Err(TickRecordError::ChannelValueMismatch);
        }
        if revision == 0 {
            return Err(TickRecordError::ZeroRevision);
        }
        if !matches!(set_at.clock, ClockRef::DomainTick { .. }) {
            return Err(TickRecordError::NotDomainTick);
        }
        Ok(StandingOrder {
            unit,
            channel,
            value,
            set_at,
            by,
            revision,
        })
    }

    /// The commanded unit.
    #[must_use]
    pub const fn unit(&self) -> ObjId {
        self.unit
    }
    /// The channel.
    #[must_use]
    pub const fn channel(&self) -> OrderChannel {
        self.channel
    }
    /// The value.
    #[must_use]
    pub const fn value(&self) -> &OrderValue {
        &self.value
    }
    /// When it was set.
    #[must_use]
    pub const fn set_at(&self) -> &ClockStamp {
        &self.set_at
    }
    /// Who set it.
    #[must_use]
    pub const fn by(&self) -> ObjId {
        self.by
    }
    /// Its revision.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }
    /// Its key.
    #[must_use]
    pub const fn key(&self) -> StandingOrderKey {
        StandingOrderKey {
            unit: self.unit,
            channel: self.channel,
        }
    }

    fn validate_document(&self) -> Result<(), TickRecordError> {
        let value = serde_json::to_value(self).map_err(|_| TickRecordError::TooLarge)?;
        canonical_json_bytes_with_limit(&value, MAX_STANDING_ORDER_BYTES)
            .map(|_| ())
            .map_err(|_| TickRecordError::TooLarge)
    }
}

impl From<StandingOrder> for StandingOrderWire {
    fn from(value: StandingOrder) -> StandingOrderWire {
        StandingOrderWire {
            unit: value.unit,
            channel: value.channel,
            value: value.value,
            set_at: value.set_at,
            by: value.by,
            revision: value.revision,
        }
    }
}

impl TryFrom<StandingOrderWire> for StandingOrder {
    type Error = TickRecordError;

    fn try_from(value: StandingOrderWire) -> Result<StandingOrder, TickRecordError> {
        let order = StandingOrder::new(
            value.unit,
            value.channel,
            value.value,
            value.set_at,
            value.by,
            value.revision,
        )?;
        order.validate_document()?;
        Ok(order)
    }
}

impl<'de> Deserialize<'de> for StandingOrder {
    fn deserialize<D>(deserializer: D) -> Result<StandingOrder, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = StandingOrderWire::deserialize(deserializer)?;
        StandingOrder::try_from(wire).map_err(serde::de::Error::custom)
    }
}

// ----------------------------------------------------------------------
// B3 — the intent vocabulary
// ----------------------------------------------------------------------

/// Every intent kind a tactical actor may submit. An **unknown kind refuses; it
/// is never skipped** — a tick that silently drops an intent it cannot classify
/// is the fail-open shape this cut exists to avoid (10b §3.8, review addition).
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentKind {
    /// Set a heading (continuous).
    Heading,
    /// Set thrust (continuous).
    Thrust,
    /// Set speed (continuous).
    Speed,
    /// Set emission-control posture (continuous).
    Posture,
    /// Fire at a held contact (discrete).
    Fire,
    /// Launch from a bay at a held contact (discrete).
    Launch,
    /// Damage control on a section (discrete).
    DamageControl,
    /// Dock with a held contact (discrete).
    Dock,
    /// Eject (discrete).
    Eject,
}

impl IntentKind {
    /// Every kind, in canonical order.
    #[must_use]
    pub const fn all() -> [IntentKind; 9] {
        [
            IntentKind::Heading,
            IntentKind::Thrust,
            IntentKind::Speed,
            IntentKind::Posture,
            IntentKind::Fire,
            IntentKind::Launch,
            IntentKind::DamageControl,
            IntentKind::Dock,
            IntentKind::Eject,
        ]
    }

    /// The wire/staff token.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            IntentKind::Heading => "heading",
            IntentKind::Thrust => "thrust",
            IntentKind::Speed => "speed",
            IntentKind::Posture => "posture",
            IntentKind::Fire => "fire",
            IntentKind::Launch => "launch",
            IntentKind::DamageControl => "dc",
            IntentKind::Dock => "dock",
            IntentKind::Eject => "eject",
        }
    }

    /// Parse a token; `None` refuses.
    #[must_use]
    pub fn parse(token: &str) -> Option<IntentKind> {
        let token = token.trim().to_ascii_lowercase();
        IntentKind::all().into_iter().find(|k| k.token() == token)
    }

    /// **The control channel this kind rides** (E7c8 fold A6/D2(a)).
    ///
    /// K5a publishes a channel set per station; before this fold nothing mapped
    /// an [`IntentKind`] onto one, so a gunner steered and a helmsman fired —
    /// whichever single channel the station happened to grant was stamped on the
    /// document and witnessed against itself. The map is total and canonical: a
    /// station may issue a kind exactly when its grant carries this channel.
    #[must_use]
    pub const fn required_channel(self) -> &'static str {
        match self {
            IntentKind::Heading
            | IntentKind::Thrust
            | IntentKind::Speed
            | IntentKind::Dock
            | IntentKind::Eject => "movement",
            IntentKind::Posture => "sensors",
            IntentKind::Fire | IntentKind::Launch => "fire",
            IntentKind::DamageControl => "damage_control",
        }
    }

    /// Every channel some kind requires, in canonical order — the vocabulary a
    /// station's grant is measured against.
    #[must_use]
    pub fn required_channels() -> Vec<&'static str> {
        let mut channels: Vec<&'static str> = IntentKind::all()
            .into_iter()
            .map(IntentKind::required_channel)
            .collect();
        channels.sort_unstable();
        channels.dedup();
        channels
    }

    /// The continuous channel this kind persists on, if any.
    #[must_use]
    pub const fn channel(self) -> Option<OrderChannel> {
        match self {
            IntentKind::Heading => Some(OrderChannel::Heading),
            IntentKind::Thrust => Some(OrderChannel::Thrust),
            IntentKind::Speed => Some(OrderChannel::Speed),
            IntentKind::Posture => Some(OrderChannel::Posture),
            _ => None,
        }
    }

    /// The stable primary sort position inside one window's resolution order.
    /// Continuous orders settle before discrete effects resolve, so a heading
    /// set this tick is what a shot fired this tick is measured against.
    #[must_use]
    pub const fn order_rank(self) -> i64 {
        match self {
            IntentKind::Heading => 0,
            IntentKind::Thrust => 1,
            IntentKind::Speed => 2,
            IntentKind::Posture => 3,
            IntentKind::DamageControl => 4,
            IntentKind::Dock => 5,
            IntentKind::Eject => 6,
            IntentKind::Launch => 7,
            IntentKind::Fire => 8,
        }
    }
}

// ----------------------------------------------------------------------
// B4 — the commit record
// ----------------------------------------------------------------------

/// One accepted intent, as the commit records it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntentRef {
    /// The submitting actor.
    pub actor: ObjId,
    /// The commanded unit.
    pub unit: ObjId,
    /// What was intended.
    pub kind: IntentKind,
    /// The submission's stable request identity.
    pub request: RequestUid,
}

/// What resolution did with one accepted intent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum DeltaOutcome {
    /// A continuous order was written; latest order wins.
    ///
    /// **The post-image travels with the delta** (fold T7). Deviation (c) makes
    /// the [`StandingOrder`] the tick's only per-unit authoritative post-image,
    /// so a delta that named only its channel left the entire mutable result of
    /// the tick outside `state_hash`: a replay that wrote heading `999,0,0`
    /// where the first wrote `1,0,0` reproduced the stored hash exactly.
    OrderSet {
        /// The channel it settled on.
        channel: OrderChannel,
        /// The value that settled there.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<OrderValue>,
        /// The revision it settled at.
        #[serde(default)]
        revision: u64,
    },
    /// A discrete effect resolved, with its recorded draw.
    Resolved {
        /// The recorded draw this effect used.
        draw: u32,
        /// Whether the effect landed.
        effective: bool,
    },
    /// The intent was refused with a stated reason. **Refused, never skipped.**
    Refused {
        /// Why.
        reason: String,
    },
}

/// One ordered delta the tick produced.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitDelta {
    /// The unit it applies to.
    pub unit: ObjId,
    /// What was intended — `None` for an intent this build cannot classify
    /// (correction 17: the refusal used to be recorded under a borrowed
    /// `IntentKind::Eject`, so two unclassifiable intents from one actor
    /// produced byte-identical deltas).
    #[serde(default)]
    pub kind: Option<IntentKind>,
    /// What happened.
    pub outcome: DeltaOutcome,
}

/// A typed boundary fact. **T2: nothing economic happens inside a pulse** — the
/// tick emits these and the economy consumes them afterwards (D-E7-17).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundaryFact {
    /// The stable fact kind (`tactics.sortie_started`, `tactics.unit_disabled`,
    /// `tactics.docked`, …).
    pub kind: String,
    /// The domain it happened in.
    pub domain: u64,
    /// The domain clock generation it happened on (fold B8: without it a
    /// demote → re-promote, which restarts the tick counter, re-mints ids an
    /// idempotent consumer has already recorded).
    #[serde(default)]
    pub generation: u64,
    /// The tick it happened on.
    pub tick: i64,
    /// The unit it happened to.
    pub unit: ObjId,
    /// Its position in this tick's fact list (fold B8: two crew can both
    /// resolve the same kind on one unit in one tick, and before this the two
    /// facts shared an id, so the second was silently dropped as a duplicate).
    #[serde(default)]
    pub ordinal: u16,
    /// **Provisional** (fold D6(b)): cut 8's resolution is a `draw % 2`
    /// placeholder with no unit post-image behind it, so a consumer must opt in
    /// rather than settle off it. The flag is removed when a damage model lands.
    #[serde(default)]
    pub provisional: bool,
}

impl BoundaryFact {
    /// The stable id cut 9 indexes on:
    /// `{kind}/{domain}/{generation}/{tick}/{unit}/{ordinal}` — unique inside a
    /// tick and never replayed across a re-promotion (fold B8).
    #[must_use]
    pub fn stable_id(&self) -> String {
        format!(
            "{}/{}/{}/{}/{}/{}",
            self.kind, self.domain, self.generation, self.tick, self.unit.0, self.ordinal
        )
    }
}

/// The `(domain, generation, tick)` key one commit occupies.
///
/// **The generation is part of the identity** (fold D5(a)/T4/B3): `apply_pulse`
/// re-anchors `started_at` on every promotion, so a re-promoted domain restarts
/// its tick counter at 0 and walked straight back over the previous generation's
/// key space — silently overwriting its low-numbered commits and stranding its
/// high-numbered ones.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TickCommitKey {
    /// The domain.
    pub domain: DomainId,
    /// The domain clock generation.
    #[serde(default)]
    pub generation: u64,
    /// The tick.
    pub tick: i64,
}

/// Encode one `i64` as a fixed-width, order-preserving decimal.
///
/// **Total over `i64`** (fold B3): the previous encoding clamped with
/// `tick.max(0)`, so every negative tick shared tick 0's family key — at tick 1
/// the ring's expiry key was `-63`, whose encoding **was** tick 0's, and the
/// deterministic first commit of a fresh domain was deleted 63 ticks early.
#[must_use]
pub fn encode_tick(tick: i64) -> String {
    let shifted = i128::from(tick) - i128::from(i64::MIN);
    format!("{shifted:020}")
}

impl TickCommitKey {
    /// The canonical encoded key. Zero-padded so the family's canonical scan
    /// order is `(domain, generation, tick)` order.
    #[must_use]
    pub fn encoded(&self) -> String {
        format!(
            "{:020}:{:020}:{}",
            self.domain.0,
            self.generation,
            encode_tick(self.tick)
        )
    }
}

/// **One deterministic domain-tick commit per domain per tick** (10b §3.8).
///
/// `state_hash` covers the accepted-intent set, the RNG root and the ordered
/// deltas, so a replay that omits an intent **fails the hash** rather than
/// reproducing it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(into = "DomainTickCommitWire")]
pub struct DomainTickCommit {
    domain: DomainId,
    generation: u64,
    tick: i64,
    clock: ClockRef,
    window: ActionWindowId,
    accepted: Vec<IntentRef>,
    deltas: Vec<UnitDelta>,
    facts: Vec<BoundaryFact>,
    rng_inputs: CanonicalHash,
    draws: Vec<u32>,
    resolution_secret_version: Option<u32>,
    state_hash: CanonicalHash,
}

/// The strict wire form of [`DomainTickCommit`].
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DomainTickCommitWire {
    /// The domain.
    pub domain: DomainId,
    /// The tick.
    pub tick: i64,
    /// The exact domain clock generation the tick ran on.
    pub clock: ClockRef,
    /// The window this commit consumed.
    pub window: ActionWindowId,
    /// The frozen accepted set, in resolution order.
    pub accepted: Vec<IntentRef>,
    /// The ordered deltas.
    pub deltas: Vec<UnitDelta>,
    /// The boundary facts the economy consumes afterwards.
    pub facts: Vec<BoundaryFact>,
    /// The RNG stream's canonical input hash.
    pub rng_inputs: CanonicalHash,
    /// The recorded draws, in draw order.
    pub draws: Vec<u32>,
    /// The root-secret version the draws were taken under (fold T11), so a
    /// replay can be checked against the exact version rather than whichever
    /// one happens to be live.
    #[serde(default)]
    pub resolution_secret_version: Option<u32>,
    /// The hash over the accepted set, the RNG root, the ordered deltas **with
    /// their post-images** and the emitted facts.
    pub state_hash: CanonicalHash,
}

impl DomainTickCommit {
    /// The hash domain the state hash is taken under.
    pub const HASH_DOMAIN: &'static str = "sim.domain_tick_commit.v1";

    /// Derive the state hash for a candidate commit's contents.
    ///
    /// # Errors
    ///
    /// [`TickRecordError::TooLarge`] when the contents will not encode.
    /// **What the hash covers** (fold T7/T12): the accepted set, the RNG root,
    /// the ordered deltas **with their post-images**, the emitted **facts** and
    /// the resolution secret version the draws were taken under. Before this
    /// fold the deltas carried only a channel name and the facts were outside
    /// the hash entirely, so a replay that wrote a different order value, or a
    /// stored commit whose `facts` array had been edited, both reproduced the
    /// stored hash exactly — the claim was true for **omission** and false for
    /// **divergence**.
    #[allow(clippy::too_many_arguments)]
    pub fn derive_state_hash(
        domain: DomainId,
        tick: i64,
        clock: &ClockRef,
        window: ActionWindowId,
        accepted: &[IntentRef],
        deltas: &[UnitDelta],
        facts: &[BoundaryFact],
        rng_inputs: CanonicalHash,
        draws: &[u32],
        resolution_secret_version: Option<u32>,
    ) -> Result<CanonicalHash, TickRecordError> {
        CanonicalHash::for_canonical_value(
            Self::HASH_DOMAIN,
            1,
            &json!({
                "domain": domain,
                "tick": tick,
                "clock": clock,
                "window": window,
                "accepted": accepted,
                "deltas": deltas,
                "facts": facts,
                "rng_inputs": rng_inputs,
                "draws": draws,
                "resolution_secret_version": resolution_secret_version,
            }),
            MAX_TICK_COMMIT_BYTES,
        )
        .map_err(|_| TickRecordError::TooLarge)
    }

    /// The one constructor. It **derives** the state hash rather than accepting
    /// one, so a commit whose hash disagrees with its contents is unconstructible
    /// and therefore unstorable.
    ///
    /// # Errors
    ///
    /// [`TickRecordError`] for a non-domain-tick clock, too many deltas or
    /// facts, or a document that will not encode inside its cap.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        domain: DomainId,
        tick: i64,
        clock: ClockRef,
        window: ActionWindowId,
        accepted: Vec<IntentRef>,
        deltas: Vec<UnitDelta>,
        facts: Vec<BoundaryFact>,
        rng_inputs: CanonicalHash,
        draws: Vec<u32>,
        resolution_secret_version: Option<u32>,
    ) -> Result<DomainTickCommit, TickRecordError> {
        let ClockRef::DomainTick { generation, .. } = &clock else {
            return Err(TickRecordError::NotDomainTick);
        };
        let generation = *generation;
        if deltas.len() > MAX_TICK_DELTAS || accepted.len() > MAX_TICK_DELTAS {
            return Err(TickRecordError::TooManyDeltas);
        }
        if facts.len() > MAX_BOUNDARY_FACTS_PER_TICK {
            return Err(TickRecordError::TooManyFacts);
        }
        let state_hash = Self::derive_state_hash(
            domain,
            tick,
            &clock,
            window,
            &accepted,
            &deltas,
            &facts,
            rng_inputs,
            &draws,
            resolution_secret_version,
        )?;
        let commit = DomainTickCommit {
            domain,
            generation,
            tick,
            clock,
            window,
            accepted,
            deltas,
            facts,
            rng_inputs,
            draws,
            resolution_secret_version,
            state_hash,
        };
        let value = serde_json::to_value(&commit).map_err(|_| TickRecordError::TooLarge)?;
        canonical_json_bytes_with_limit(&value, MAX_TICK_COMMIT_BYTES)
            .map_err(|_| TickRecordError::TooLarge)?;
        Ok(commit)
    }

    /// The domain.
    #[must_use]
    pub const fn domain(&self) -> DomainId {
        self.domain
    }
    /// The tick.
    #[must_use]
    pub const fn tick(&self) -> i64 {
        self.tick
    }
    /// The clock generation it ran on.
    #[must_use]
    pub const fn clock(&self) -> &ClockRef {
        &self.clock
    }
    /// The consumed window.
    #[must_use]
    pub const fn window(&self) -> ActionWindowId {
        self.window
    }
    /// The frozen accepted set.
    #[must_use]
    pub fn accepted(&self) -> &[IntentRef] {
        &self.accepted
    }
    /// The ordered deltas.
    #[must_use]
    pub fn deltas(&self) -> &[UnitDelta] {
        &self.deltas
    }
    /// The boundary facts.
    #[must_use]
    pub fn facts(&self) -> &[BoundaryFact] {
        &self.facts
    }
    /// The recorded draws.
    #[must_use]
    pub fn draws(&self) -> &[u32] {
        &self.draws
    }
    /// The root-secret version the draws were taken under.
    #[must_use]
    pub const fn resolution_secret_version(&self) -> Option<u32> {
        self.resolution_secret_version
    }
    /// The clock generation.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }
    /// The state hash.
    #[must_use]
    pub const fn state_hash(&self) -> CanonicalHash {
        self.state_hash
    }
    /// Its key.
    #[must_use]
    pub const fn key(&self) -> TickCommitKey {
        TickCommitKey {
            domain: self.domain,
            generation: self.generation,
            tick: self.tick,
        }
    }
}

impl From<DomainTickCommit> for DomainTickCommitWire {
    fn from(value: DomainTickCommit) -> DomainTickCommitWire {
        DomainTickCommitWire {
            domain: value.domain,
            tick: value.tick,
            clock: value.clock,
            window: value.window,
            accepted: value.accepted,
            deltas: value.deltas,
            facts: value.facts,
            rng_inputs: value.rng_inputs,
            draws: value.draws,
            resolution_secret_version: value.resolution_secret_version,
            state_hash: value.state_hash,
        }
    }
}

impl TryFrom<DomainTickCommitWire> for DomainTickCommit {
    type Error = TickRecordError;

    fn try_from(value: DomainTickCommitWire) -> Result<DomainTickCommit, TickRecordError> {
        let stored = value.state_hash;
        let commit = DomainTickCommit::new(
            value.domain,
            value.tick,
            value.clock,
            value.window,
            value.accepted,
            value.deltas,
            value.facts,
            value.rng_inputs,
            value.draws,
            value.resolution_secret_version,
        )?;
        if commit.state_hash != stored {
            return Err(TickRecordError::HashMismatch);
        }
        Ok(commit)
    }
}

impl<'de> Deserialize<'de> for DomainTickCommit {
    fn deserialize<D>(deserializer: D) -> Result<DomainTickCommit, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = DomainTickCommitWire::deserialize(deserializer)?;
        DomainTickCommit::try_from(wire).map_err(serde::de::Error::custom)
    }
}

// ----------------------------------------------------------------------
// B1 — identities
// ----------------------------------------------------------------------

/// This module's `OwnerId`.
///
/// # Errors
///
/// Never, in practice: `NS` is a static canonical id.
pub fn owner() -> Result<OwnerId, ModError> {
    OwnerId::new(NS).map_err(|error| ModError::msg(error.to_string()))
}

/// The owner-local canonical id of one runtime domain.
///
/// This is the `DomainId(u64)` ↔ `CanonicalId` mapping the landed
/// `DomainClockProvider::now(.., domain: &CanonicalId, ..)` signature needs
/// (10b §3.8 OPEN 2 / contradiction (i)) — the arena fixture models it with
/// `domain.as_str()`, and this is its production form.
#[must_use]
pub fn domain_canonical_id(domain: DomainId) -> Option<CanonicalId> {
    CanonicalId::new(format!("d{}", domain.0)).ok()
}

/// The inverse map: a `CanonicalId` back to its `DomainId`.
#[must_use]
pub fn domain_id_of(canonical: &CanonicalId) -> Option<DomainId> {
    canonical
        .as_str()
        .strip_prefix('d')?
        .parse()
        .ok()
        .map(DomainId)
}

/// The exact `ClockRef` a pulsing domain reads on.
#[must_use]
pub fn pulse_clock(owner: &OwnerId, domain: DomainId, generation: u64) -> Option<ClockRef> {
    Some(ClockRef::DomainTick {
        owner: owner.clone(),
        id: domain_canonical_id(domain)?,
        generation,
    })
}

/// The stable window identity for one `(domain, generation, tick)`.
///
/// **The generation is part of the identity** (fold D5(a)/T4): without it a
/// demote → re-promote — which re-anchors `started_at` and restarts the tick
/// counter at 0 — walked back over the previous generation's leftover windows,
/// so every legitimate order for the reused tick number refused
/// `IntentWindowMismatch` and the tick's own `checked_close_due` failed
/// `ClockMismatch` across two `ClockRef::DomainTick` generations.
#[must_use]
pub fn tick_window_id(
    owner: &OwnerId,
    domain: DomainId,
    generation: u64,
    tick: i64,
) -> Option<ActionWindowId> {
    let hash = CanonicalHash::for_canonical_value(
        "sim.domain_tick_window_local.v1",
        1,
        &json!({ "domain": domain, "generation": generation, "tick": tick }),
        1024,
    )
    .ok()?;
    let local = CanonicalId::parse_canonical(hash.to_hex()).ok()?;
    ActionWindowId::derive(owner, &local).ok()
}

/// The stable pulse-cursor identity for one domain. **The retryable entry's key
/// shape is `domaintick/{domain}`** and the deterministic idempotency key the
/// kernel mints over it is `(schedule, generation, ordinal)` — which is
/// `(domain, tick)` (§4 rule 3).
#[must_use]
pub fn tick_schedule_id(owner: &OwnerId, domain: DomainId) -> Option<ScheduleId> {
    let hash = CanonicalHash::for_canonical_value(
        "sim.domain_tick_schedule_local.v1",
        1,
        &json!({ "domain": domain }),
        1024,
    )
    .ok()?;
    let local = CanonicalId::parse_canonical(hash.to_hex()).ok()?;
    ScheduleId::derive(owner, &local).ok()
}

// ----------------------------------------------------------------------
// B1 — the production DomainClockProvider
// ----------------------------------------------------------------------

/// **The one owner-scoped production `DomainClockProvider`** (08i answer 4/G-A).
///
/// Registered once, at composition, through `Registrar::domain_clock_provider`.
/// It holds nothing: every answer is read from the runtime `domain` id and the
/// live `SpatialState`, so promoting a second domain adds no registration and
/// leaves the frozen `ClockProviderRegistry` byte-identical.
#[derive(Clone, Copy, Debug, Default)]
pub struct SimDomainClockProvider;

impl mush_core::DomainClockProvider for SimDomainClockProvider {
    fn now(
        &self,
        context: &ClockProviderContext<'_>,
        domain: &CanonicalId,
        generation: u64,
    ) -> Result<i64, ClockProviderFailure> {
        let Some(id) = domain_id_of(domain) else {
            return Err(ClockProviderFailure::Unavailable);
        };
        let Some(state) = context.module_state::<SpatialState>() else {
            return Err(ClockProviderFailure::Unavailable);
        };
        let Some(record) = state.domains.get(&id) else {
            return Err(ClockProviderFailure::Unavailable);
        };
        let Some(current) = record.clock.generation() else {
            // A passive domain has no tick to answer. It is not an integrity
            // failure — it is simply not on this clock.
            return Err(ClockProviderFailure::Unavailable);
        };
        if current != generation {
            return Err(ClockProviderFailure::StaleGeneration {
                requested: generation,
                current,
            });
        }
        record
            .clock
            .tick_at(&context.world_ic_now())
            .ok_or(ClockProviderFailure::Integrity)
    }
}

// ----------------------------------------------------------------------
// families
// ----------------------------------------------------------------------

/// Every typed family the pulse owns, cloned into each provider and adapter.
#[derive(Clone)]
pub(crate) struct TickFamilies {
    windows: StateFamilyHandle<ActionWindowId, ActionWindowRecord>,
    schedules: StateFamilyHandle<ScheduleId, DurableSchedule>,
    commits: StateFamilyHandle<TickCommitKey, DomainTickCommit>,
    orders: StateFamilyHandle<StandingOrderKey, StandingOrder>,
    reaping: StateFamilyHandle<DomainId, TickReapProgress>,
}

/// One cursor per family and active domain. Aborted ticks never advance these
/// cursors; restart resumes the exact committed scan, not the family prefix.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TickReapProgress {
    commits: Option<String>,
    windows: Option<String>,
    orders: Option<String>,
}

impl TickReapProgress {
    fn validate(&self) -> Result<(), String> {
        if [&self.commits, &self.windows, &self.orders]
            .into_iter()
            .flatten()
            .any(|key| key.is_empty() || key.len() > 64)
        {
            return Err("invalid tick retention cursor".to_owned());
        }
        Ok(())
    }
}

fn descriptor(kind: &str, id: &str, payload: Value) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).map_err(|error| ModError::msg(error.to_string()))?,
        DefinitionId::new(id).map_err(|error| ModError::msg(error.to_string()))?,
        1,
        payload,
    )
    .map_err(|error| ModError::msg(error.to_string()))
}

/// The tick outcome descriptor. Counts only: an outcome naming the accepted
/// actors would put one station's picture on another's receipt.
pub(crate) fn tick_outcome_descriptor() -> Result<DefinitionDescriptor, ModError> {
    Ok(descriptor(
        "outcome",
        TICK_OUTCOME_ID,
        json!({
            "fields": {
                "tick": "i64",
                "accepted": "usize",
                "facts": "usize",
                "facts_refused": "usize",
                "reaped": "usize"
            },
            "contains_truth_subject": false
        }),
    )?
    .as_current())
}

/// The tick action descriptor.
pub(crate) fn tick_action_descriptor(
    outcome: &DefinitionRef,
) -> Result<DefinitionDescriptor, ModError> {
    Ok(descriptor(
        "action",
        TICK_ACTION_ID,
        json!({
            "internal": true,
            "subject": "self_actor",
            "targets": 0,
            "state_family": COMMIT_FAMILY,
            "writes_economy": false
        }),
    )?
    .with_references(vec![outcome.clone()])
    .as_current())
}

/// The arm outcome descriptor.
pub(crate) fn arm_outcome_descriptor() -> Result<DefinitionDescriptor, ModError> {
    Ok(descriptor(
        "outcome",
        ARM_OUTCOME_ID,
        json!({
            "fields": { "armed": "bool", "generation": "u64" },
            "contains_truth_subject": false
        }),
    )?
    .as_current())
}

/// The arm action descriptor.
pub(crate) fn arm_action_descriptor(
    outcome: &DefinitionRef,
) -> Result<DefinitionDescriptor, ModError> {
    Ok(descriptor(
        "action",
        ARM_ACTION_ID,
        json!({
            "internal": false,
            "subject": "self_actor",
            "targets": 0,
            "state_family": SCHEDULE_FAMILY,
            "writes_economy": false
        }),
    )?
    .with_references(vec![outcome.clone()])
    .as_current())
}

/// One refusal on the tick lane, **classified honestly** (fold T5/D4(a)).
///
/// Before this fold every refusal the pulse raised was `retryable: false`, so a
/// per-domain bound refusal, a cross-generation window and any typed-state
/// hiccup all satisfied neither `scheduled_prepare_stale` nor
/// `scheduled_prepare_retryable` and fell through to `apply_integrity_freeze` —
/// one domain's problem stopped the world. Now `Stale` is anything a later fire
/// recovers from, `retryable` is a genuine transient, and `Integrity` is
/// reserved for what should really stop the world.
pub(crate) fn err(code: ActionErrorCode, message: &'static str) -> ActionError {
    let retryable = matches!(
        code,
        ActionErrorCode::Busy | ActionErrorCode::Unavailable | ActionErrorCode::Conflict
    );
    ActionError::new(code, retryable, message).expect("static tick error is bounded")
}

/// A refusal a **later fire recovers from on its own**: a window that has
/// already closed, a candidate whose domain moved on, a stamp from a generation
/// that is gone. The driver discards it as stale and drives on.
fn stale(message: &'static str) -> ActionError {
    err(ActionErrorCode::Stale, message)
}

/// A refusal that should genuinely stop the world: the tick's own records
/// disagree with themselves in a way no later fire can mend.
fn integrity(message: &'static str) -> ActionError {
    err(ActionErrorCode::Integrity, message)
}

/// The typed-state lane is **transient**, not an integrity failure: a store that
/// cannot answer this instant answers the next poll (fold T5).
fn state_error(_error: &ActionStateError) -> ActionError {
    err(
        ActionErrorCode::Unavailable,
        "the tactical tick's own state is unavailable",
    )
}

/// The recorded reason a world-side pulse gives an intent it cannot resolve.
///
/// With the combat lane retired (16b cut T, D-16-20) no submit action writes
/// into a tick window at all, so this is unreachable in the shipped tree. It
/// exists because the alternative — dropping an intent the window froze — is the
/// fail-open shape the pulse was built to refuse.
pub const NO_RESOLVER: &str = "no resolver for that intent on this lane";

/// One tick's collection interval: `[tick, tick + 1)` on the domain's own clock.
///
/// `secret_version` **pins** the root-secret version the eventual resolution
/// will draw under (fold T11). The kernel already runs the other half of that
/// contract — `ActionState::unresolved_window_resolution_secret_versions` feeds
/// `Engine::resolution_key_rotation_references`, which exists so a rotation can
/// be REFUSED when it would prune material an unresolved window still needs.
/// Leaving it `None`, as cut 8 did, made every tick window invisible to that
/// guard. `None` remains valid where no resolution key ring is installed.
fn open_window(
    id: ActionWindowId,
    clock: &ClockRef,
    tick: i64,
    generation: u64,
    secret_version: Option<u32>,
) -> Result<ActionWindow, mush_core::ActionWindowError> {
    ActionWindow::new(ActionWindowDraft {
        id,
        clock: clock.clone(),
        opens: ClockStamp {
            clock: clock.clone(),
            at: tick,
        },
        closes: ClockStamp {
            clock: clock.clone(),
            at: tick.saturating_add(1),
        },
        generation,
        mode: WindowMode::CollectSimultaneous,
        capacity: MAX_INTENTS_PER_TICK,
        restart: WindowRestartPolicy::CloseIfElapsed,
        timeout: WindowTimeoutPolicy::Close,
        max_catch_up: 1,
        resolution_secret_version: secret_version,
    })
}

// ----------------------------------------------------------------------
// B4 — the tick provider
// ----------------------------------------------------------------------

/// The pulse's resolution half: **one deterministic commit per domain per tick**.
#[derive(Clone)]
struct DomainTickProvider {
    outcome: DefinitionRef,
    families: TickFamilies,
}

/// The tick's decoded schedule parameters.
struct TickPlan {
    domain: DomainId,
    tick: i64,
    observed_at: ClockStamp,
    window: ActionWindowId,
    /// Commit keys that have fallen out of the retention ring, **named by a
    /// bounded scan** rather than guessed arithmetically (fold D5(a)/B3). Also
    /// carries every commit of a superseded generation.
    reap_commits: Vec<TickCommitKey>,
    /// Orphan window records: a generation that is gone, or a tick older than
    /// the one this fire retains. The consumed window itself is kept one tick.
    reap_windows: Vec<ActionWindowId>,
    /// Standing orders whose unit is no longer in any domain (fold A9): the
    /// book had no lifecycle at all and no `txn.remove` anywhere in the tree.
    reap_orders: Vec<StandingOrderKey>,
    reap_progress: TickReapProgress,
}

impl DomainTickProvider {
    fn plan(invocation: &ActionInvocation) -> Result<TickPlan, ActionError> {
        let parameters = invocation.parameters();
        let domain = parameters
            .get("domain")
            .and_then(Value::as_u64)
            .map(DomainId)
            .ok_or_else(|| integrity("a domain tick names its domain"))?;
        let tick = parameters
            .get("tick")
            .and_then(Value::as_i64)
            .ok_or_else(|| integrity("a domain tick names its tick"))?;
        let observed_at: ClockStamp = parameters
            .get("observed_at")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .ok_or_else(|| integrity("a domain tick names its observed stamp"))?;
        let window: ActionWindowId = parameters
            .get("window")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .ok_or_else(|| integrity("a domain tick names its window"))?;
        let reap_commits: Vec<TickCommitKey> = parameters
            .get("reap_commits")
            .cloned()
            .map_or_else(
                || Some(Vec::new()),
                |value| serde_json::from_value(value).ok(),
            )
            .ok_or_else(|| integrity("a domain tick names its commit reap set"))?;
        let reap_windows: Vec<ActionWindowId> = parameters
            .get("reap_windows")
            .cloned()
            .map_or_else(
                || Some(Vec::new()),
                |value| serde_json::from_value(value).ok(),
            )
            .ok_or_else(|| integrity("a domain tick names its window reap set"))?;
        let reap_orders: Vec<StandingOrderKey> = parameters
            .get("reap_orders")
            .cloned()
            .map_or_else(
                || Some(Vec::new()),
                |value| serde_json::from_value(value).ok(),
            )
            .ok_or_else(|| integrity("a domain tick names its order reap set"))?;
        if reap_commits.len() > MAX_REAP_PER_FIRE
            || reap_windows.len() > MAX_REAP_PER_FIRE
            || reap_orders.len() > MAX_REAP_PER_FIRE
        {
            return Err(err(
                ActionErrorCode::BoundsExceeded,
                "a domain tick reaps at most MAX_REAP_PER_FIRE records per family",
            ));
        }
        let reap_progress: TickReapProgress = parameters
            .get("reap_progress")
            .cloned()
            .map_or_else(|| Ok(TickReapProgress::default()), serde_json::from_value)
            .map_err(|_| integrity("a domain tick names valid retention progress"))?;
        reap_progress
            .validate()
            .map_err(|_| integrity("invalid retention cursor"))?;
        Ok(TickPlan {
            domain,
            tick,
            observed_at,
            window,
            reap_commits,
            reap_windows,
            reap_orders,
            reap_progress,
        })
    }

    /// A distinct, replay-stable operation id per window transition, so the
    /// close and the consume do not collide on one `RequestUid` receipt.
    fn operation(request: RequestUid, phase: &str) -> Result<RequestUid, ActionError> {
        let hash = CanonicalHash::for_canonical_value(
            "sim.domain_tick_operation.v1",
            1,
            &json!({ "request": request, "phase": phase }),
            512,
        )
        .map_err(|_| integrity("a tick operation id is unconstructible"))?;
        let mut bytes = [0_u8; 16];
        bytes.copy_from_slice(&hash.as_bytes()[..16]);
        Ok(RequestUid::from_bytes(bytes))
    }
}

impl ActionProvider for DomainTickProvider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        DomainTickProvider::plan(invocation).map(|_| ())
    }

    /// **The `"internal": true` flag is decorative; THIS is the fence** (fold
    /// A14). Nothing in `mush-core` reads that key, so before this the tick was
    /// unreachable to a client only because the trait default returned
    /// `Ok(None)` — real protection, but accidental. An explicit override says
    /// why, so a later maintainer who adds a projection for any reason has to
    /// delete this line and read the comment first: `prepare_authoritative_with_context`
    /// takes its `current` witness list as an action **parameter**, i.e. an
    /// attacker-declared "these intents passed the station check".
    fn project_offer(
        &self,
        _action: &DefinitionRef,
        _query: &LegalActionQuery,
        _engine: &Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        Ok(None)
    }

    fn prepare_invocation(
        &self,
        _invocation: &ActionInvocation,
        _txn: &mut ActionTxn<'_>,
    ) -> Result<(), ActionError> {
        Err(err(
            ActionErrorCode::Unavailable,
            "a domain tick requires the authoritative action coordinator",
        ))
    }

    fn prepare_authoritative(
        &self,
        _invocation: &ActionInvocation,
        _txn: &mut ActionTxn<'_>,
    ) -> Result<mush_core::AuthoritativeActionOutput, ActionError> {
        // The tick always resolves with recorded RNG, so it takes the
        // context-bearing entry point and never this one.
        Err(integrity("a domain tick resolves with recorded randomness"))
    }

    fn prepare_authoritative_with_context(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        context: &mut mush_core::PrepareCtx<'_>,
    ) -> Result<mush_core::AuthoritativeActionOutput, ActionError> {
        let plan = DomainTickProvider::plan(invocation)?;
        let commit_token = context.commit_token();
        let owner = self.families.windows.owner().clone();

        // ---- the frozen set ----
        let record = txn
            .get(&self.families.windows, &plan.window)
            .map_err(|error| state_error(&error))?
            .cloned();
        // A record left over from an EARLIER generation is treated as absent
        // and reaped rather than closed (fold T4): `checked_close_due` compares
        // stamps across two `ClockRef::DomainTick` generations and fails
        // `ClockMismatch`, which used to take the whole world down with it.
        let record = record.filter(|record| record.window().clock() == &plan.observed_at.clock);
        let (unresolved, consumed) = match record {
            Some(record) => {
                let closed = if record.phase() == mush_core::ActionWindowPhase::Open {
                    record
                        .checked_close_due(
                            record.revision(),
                            Self::operation(invocation.request(), "close")?,
                            &plan.observed_at,
                        )
                        .map_err(|_| stale("this tick's window would not close"))?
                        .into_record()
                } else {
                    record
                };
                let ordered = closed
                    .resolution_order()
                    .map_err(|_| stale("this tick's window would not order"))?;
                let unresolved: Vec<ObjId> = ordered
                    .iter()
                    .map(|intent| intent.invocation().actor())
                    .collect();
                let consumed = closed
                    .checked_consume(
                        closed.revision(),
                        Self::operation(invocation.request(), "consume")?,
                        commit_token,
                        &plan.observed_at,
                    )
                    .map_err(|_| stale("this tick's window would not consume"))?
                    .into_record();
                (unresolved, Some(consumed))
            }
            None => (Vec::new(), None),
        };

        // ---- recorded RNG (K3.1a): one stream, never re-drawn on replay ----
        let rng_domain = CanonicalId::new("sim.domain_tick")
            .map_err(|_| integrity("the tick's rng domain is invalid"))?;
        // **The RNG root identifies the tick's INPUTS** (fold T7). With the
        // combat lane retired (16b cut T, D-16-20) the only input left is the
        // window's own identity and whatever it froze, so the root is exactly
        // that — never a constant, because two ticks of one domain must not
        // share a stream.
        let inputs_hash = CanonicalHash::for_canonical_value(
            "sim.domain_tick_inputs.v1",
            1,
            &json!({
                "domain": plan.domain,
                "tick": plan.tick,
                "window": plan.window,
                "unresolved": unresolved,
            }),
            MAX_TICK_COMMIT_BYTES,
        )
        .map_err(|_| integrity("the tick's rng inputs will not encode"))?;
        // **T11:** draw under the version the window PINNED, so a replay after a
        // key rotation reproduces the same stream. Falling back to the live
        // version keeps a pre-fold window (pinned `None`) resolvable.
        let secret_version = consumed
            .as_ref()
            .and_then(|record| record.window().resolution_secret_version())
            .or_else(|| context.current_resolution_secret_version().ok());
        let rng = match secret_version {
            Some(version) => context.start_resolution_at(version, &rng_domain, inputs_hash),
            None => context.start_resolution(&rng_domain, inputs_hash),
        }
        .map_err(|_| {
            err(
                ActionErrorCode::Unavailable,
                "the tick's resolution stream is unavailable",
            )
        })?;

        // ---- resolution ----
        //
        // **The placeholder resolution is retired** (16b cut T, T0/D-16-20).
        // Combat is the arena's (`crates/tacsim/src/arena/`), which is the
        // single writer for its own facts while a match runs; this pulse is the
        // world-side one the non-combat domains share, and nothing submits into
        // its window today. So the commit records an empty resolution — and an
        // intent that nevertheless reaches the window is **refused with its
        // reason**, never silently dropped, which is the one invariant the
        // retired arm carried (10b §3.8, review addition).
        let accepted: Vec<IntentRef> = Vec::new();
        let facts: Vec<BoundaryFact> = Vec::new();
        let draws: Vec<u32> = Vec::new();
        let facts_refused = 0_usize;
        let generation = match &plan.observed_at.clock {
            ClockRef::DomainTick { generation, .. } => *generation,
            _ => return Err(integrity("a domain tick fires on a domain tick clock")),
        };
        let deltas: Vec<UnitDelta> = unresolved
            .into_iter()
            .map(|actor| UnitDelta {
                unit: actor,
                kind: None,
                outcome: DeltaOutcome::Refused {
                    reason: NO_RESOLVER.to_owned(),
                },
            })
            .collect();

        // ---- the one commit ----
        let clock = plan.observed_at.clock.clone();
        let commit = DomainTickCommit::new(
            plan.domain,
            plan.tick,
            clock,
            plan.window,
            accepted.clone(),
            deltas,
            facts.clone(),
            inputs_hash,
            draws,
            secret_version,
        )
        .map_err(|_| {
            err(
                ActionErrorCode::BoundsExceeded,
                "this tick's commit exceeds its record bound",
            )
        })?;
        let key = commit.key();
        let accepted_count = accepted.len();

        if let Some(consumed) = consumed {
            // **T8/D5(a):** the spent window is RETAINED for one tick, not
            // deleted. Deleting it destroyed the kernel's own duplicate fence:
            // `prepare_authoritative` treats absence as "open a fresh one", so
            // an intent that raced the 250 ms driver against a 2 s pulse was
            // accepted into a spent window, acknowledged to the player and never
            // consumed. Retained, `checked_submit`'s `require_open` refuses it
            // with the player-actionable `Stale` the provider already maps. It
            // is reaped by the NEXT fire through `plan.reap_windows`.
            txn.replace(&self.families.windows, &plan.window, consumed)
                .map_err(|error| state_error(&error))?;
        }
        if txn
            .get(&self.families.commits, &key)
            .map_err(|error| state_error(&error))?
            .is_some()
        {
            txn.replace(&self.families.commits, &key, commit)
        } else {
            txn.insert(&self.families.commits, &key, commit)
        }
        .map_err(|error| state_error(&error))?;
        // **The retention ring, the orphan sweep and the order reaper** (fold
        // D5(a)/B3/A9). Every key here was named by a BOUNDED SCAN of the real
        // family in `DomainTickScheduleAdapter::plan` — not by
        // `plan.tick - 64`, which named a tick that never committed after any
        // gap in a `LatestFirst`/`max_catch_up: 1` cursor and, below tick 64,
        // named tick 0's own key. The per-fire cap is `MAX_REAP_PER_FIRE`; what
        // is left over is named again two seconds later.
        let mut reaped = 0_usize;
        for expired in &plan.reap_commits {
            if txn
                .get(&self.families.commits, expired)
                .map_err(|error| state_error(&error))?
                .is_some()
            {
                txn.remove(&self.families.commits, expired)
                    .map_err(|error| state_error(&error))?;
                reaped = reaped.saturating_add(1);
            }
        }
        for orphan in &plan.reap_windows {
            if *orphan == plan.window {
                continue;
            }
            if txn
                .get(&self.families.windows, orphan)
                .map_err(|error| state_error(&error))?
                .is_some()
            {
                txn.remove(&self.families.windows, orphan)
                    .map_err(|error| state_error(&error))?;
                reaped = reaped.saturating_add(1);
            }
        }
        for order_key in &plan.reap_orders {
            if txn
                .get(&self.families.orders, order_key)
                .map_err(|error| state_error(&error))?
                .is_some()
            {
                txn.remove(&self.families.orders, order_key)
                    .map_err(|error| state_error(&error))?;
                reaped = reaped.saturating_add(1);
            }
        }
        if txn
            .get(&self.families.reaping, &plan.domain)
            .map_err(|error| state_error(&error))?
            .is_some()
        {
            txn.replace(
                &self.families.reaping,
                &plan.domain,
                plan.reap_progress.clone(),
            )
        } else {
            txn.insert(
                &self.families.reaping,
                &plan.domain,
                plan.reap_progress.clone(),
            )
        }
        .map_err(|error| state_error(&error))?;

        // The next collection interval, so the following tick's submissions have
        // somewhere to go without a second authority opening it.
        let next_tick = plan.tick.saturating_add(1);
        if let Some(next_id) = tick_window_id(&owner, plan.domain, generation, next_tick)
            && txn
                .get(&self.families.windows, &next_id)
                .map_err(|error| state_error(&error))?
                .is_none()
        {
            let window = open_window(
                next_id,
                &plan.observed_at.clock,
                next_tick,
                generation,
                context.current_resolution_secret_version().ok(),
            )
            .map_err(|_| integrity("the next tick's window is unconstructible"))?;
            txn.insert(
                &self.families.windows,
                &next_id,
                ActionWindowRecord::open(window),
            )
            .map_err(|error| state_error(&error))?;
        }

        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({
                "tick": plan.tick,
                "accepted": accepted_count,
                "facts": facts.len(),
                "facts_refused": facts_refused,
                "reaped": reaped,
            }),
        )
        .map_err(|_| integrity("the tick outcome is invalid"))?;
        // **The prepared resolution's outcome must BE the action's outcome.**
        // `prepare_registered_authoritative_action` refuses the bundle when the
        // two differ, and cut 8 passed an empty `json!({})` here — which no test
        // could see, because no cut-8 test ever drove the tick provider at all.
        // The E7c8 fold's D7(a) harness (`world_task.rs`'s `e7c8_pulse`) is what
        // surfaced it on its first driven tick.
        let prepared = rng
            .into_prepared(
                Vec::new(),
                RulesValue::new(self.outcome.clone(), json!({}))
                    .map_err(|_| integrity("the tick modifiers are invalid"))?,
                outcome.clone(),
            )
            .map_err(|_| integrity("the tick's resolution will not close"))?;
        Ok(mush_core::AuthoritativeActionOutput::new(outcome).with_resolution(prepared))
    }

    fn prepare_resolved_authoritative_with_context(
        &self,
        resolved: &ResolvedActionInvocation,
        txn: &mut ActionTxn<'_>,
        context: &mut mush_core::PrepareCtx<'_>,
    ) -> Result<mush_core::AuthoritativeActionOutput, ActionError> {
        self.prepare_authoritative_with_context(resolved.invocation(), txn, context)
    }
}

/// The boundary fact one discrete intent kind emits (D-E7-17's typed set).
#[must_use]
pub const fn boundary_kind(kind: IntentKind) -> &'static str {
    match kind {
        IntentKind::Fire => "tactics.fired",
        IntentKind::Launch => "tactics.sortie_started",
        IntentKind::DamageControl => "tactics.damage_controlled",
        IntentKind::Dock => "tactics.docked",
        IntentKind::Eject => "tactics.unit_abandoned",
        _ => "tactics.order_set",
    }
}

// ----------------------------------------------------------------------
// B1 — the arm provider (the only kernel way to create a DurableSchedule)
// ----------------------------------------------------------------------

/// Arms or disarms one domain's durable pulse cursor.
///
/// A `DurableSchedule` can be written **only** inside an `ActionTxn` in
/// `prepare_authoritative`, so the applying authority needs an action of its
/// own; this is it. Staff-gated at projection and re-checked structurally in
/// `prepare_authoritative` through the fixed `armed_by` offer parameter — cut
/// 7's O2/K16 lesson, that a gate which lives only in `project_offer` is a
/// compositional convention rather than a fence.
#[derive(Clone)]
struct PulseArmProvider {
    action: DefinitionRef,
    outcome: DefinitionRef,
    families: TickFamilies,
}

/// The arm action's decoded parameters.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArmDoc {
    domain: u64,
    generation: u64,
    /// The domain-tick scalar the cursor starts from.
    from_tick: i64,
    /// Arm (`true`) or disarm (`false`).
    arm: bool,
    /// On a **disarm**, the units whose standing orders leave with the domain
    /// (fold A9). Bounded by `MAX_UNITS_PER_DOMAIN`; the world-side pass reads
    /// the census, because a prepared provider has no `&World`.
    #[serde(default)]
    clear_units: Vec<ObjId>,
}

impl PulseArmProvider {
    fn document(invocation: &ActionInvocation) -> Result<ArmDoc, ActionError> {
        let raw = invocation
            .parameters()
            .get("arm")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                err(
                    ActionErrorCode::InvalidRequest,
                    "a pulse arm carries one `arm` document",
                )
            })?;
        serde_json::from_str(raw).map_err(|_| {
            err(
                ActionErrorCode::InvalidRequest,
                "that pulse arm will not decode",
            )
        })
    }
}

impl ActionProvider for PulseArmProvider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        if invocation.subject() != &PerceivedRef::SelfActor || !invocation.targets().is_empty() {
            return Err(err(
                ActionErrorCode::InvalidRequest,
                "a pulse arm acts on self with no targets",
            ));
        }
        Self::document(invocation).map(|_| ())
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &LegalActionQuery,
        engine: &Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        let SubjectKey::Object { id: subject } = query.truth_subject() else {
            return Ok(None);
        };
        if action != &self.action
            || query.actor() != *subject
            || query.projected_subject() != &PerceivedRef::SelfActor
            || !pulse_arm_is_permitted(&engine.world, query.actor())
        {
            return Ok(None);
        }
        let definitions = engine
            .registry
            .definitions()
            .ok_or_else(|| integrity("the pulse definition registry is unavailable"))?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|_| integrity("the pulse guard registry is unavailable"))?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| integrity("the pulse definition guard is unavailable"))?;
        Ok(Some(ProviderOfferDraft {
            fixed_parameters: json!({ "armed_by": query.actor() }),
            parameters: ProjectedParameterSchema::new(vec![ProjectedParameter {
                id: CanonicalId::new("arm")
                    .map_err(|_| integrity("the pulse parameter id is invalid"))?,
                required: true,
                kind: ProjectedParameterKind::Text {
                    maximum_bytes: mush_core::MAX_PROJECTED_ACTION_TEXT_BYTES,
                },
            }])
            .map_err(|_| integrity("the pulse parameter schema is invalid"))?,
            targets: ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: ActionPresentation::new(
                "domain_pulse_arm",
                "arm a domain pulse",
                "arm or disarm one domain's durable tick cursor",
            )
            .map_err(|_| integrity("the pulse presentation is invalid"))?,
            guards: guards
                .finish()
                .map_err(|_| integrity("the pulse offer guards are invalid"))?,
        }))
    }

    fn prepare_invocation(
        &self,
        _invocation: &ActionInvocation,
        _txn: &mut ActionTxn<'_>,
    ) -> Result<(), ActionError> {
        Err(err(
            ActionErrorCode::Unavailable,
            "arming a domain pulse requires the authoritative action coordinator",
        ))
    }

    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<mush_core::AuthoritativeActionOutput, ActionError> {
        self.arm_pulse(invocation, txn, None)
    }

    fn prepare_authoritative_with_context(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        context: &mut mush_core::PrepareCtx<'_>,
    ) -> Result<mush_core::AuthoritativeActionOutput, ActionError> {
        let version = context.current_resolution_secret_version().ok();
        self.arm_pulse(invocation, txn, version)
    }
}

impl PulseArmProvider {
    fn arm_pulse(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        secret_version: Option<u32>,
    ) -> Result<mush_core::AuthoritativeActionOutput, ActionError> {
        self.validate_invocation(invocation)?;
        // The staff gate is STRUCTURAL, not compositional (cut 7 fold K16):
        // `armed_by` is a fixed offer parameter, so its presence and its
        // agreement with the acting object prove the offer was projected for
        // this actor and the gate ran.
        if invocation.parameters().get("armed_by") != Some(&json!(invocation.actor())) {
            return Err(err(
                ActionErrorCode::Unauthorized,
                "a pulse arm must come through a projected offer for the acting object",
            ));
        }
        let doc = Self::document(invocation)?;
        if doc.clear_units.len() > MAX_UNITS_PER_DOMAIN {
            return Err(err(
                ActionErrorCode::BoundsExceeded,
                "a pulse disarm clears at most MAX_UNITS_PER_DOMAIN units' orders",
            ));
        }
        let owner = self.families.schedules.owner().clone();
        let domain = DomainId(doc.domain);
        let id = tick_schedule_id(&owner, domain)
            .ok_or_else(|| integrity("that domain's pulse cursor identity is unconstructible"))?;
        let prior = txn
            .get(&self.families.schedules, &id)
            .map_err(|error| state_error(&error))?
            .cloned();
        if !doc.arm {
            if prior.is_some() {
                txn.remove(&self.families.schedules, &id)
                    .map_err(|error| state_error(&error))?;
            }
            if txn
                .get(&self.families.reaping, &domain)
                .map_err(|error| state_error(&error))?
                .is_some()
            {
                txn.remove(&self.families.reaping, &domain)
                    .map_err(|error| state_error(&error))?;
            }
            // **A9:** the standing-order book leaves with the domain. Before
            // this fold there was no `txn.remove` on the order family anywhere
            // in the tree, so a captured unit booted with the previous
            // commander's heading, thrust and emcon posture still authoritative.
            for unit in &doc.clear_units {
                for channel in OrderChannel::all() {
                    let key = StandingOrderKey {
                        unit: *unit,
                        channel,
                    };
                    if txn
                        .get(&self.families.orders, &key)
                        .map_err(|error| state_error(&error))?
                        .is_some()
                    {
                        txn.remove(&self.families.orders, &key)
                            .map_err(|error| state_error(&error))?;
                    }
                }
            }
            let outcome = RulesValue::new(
                self.outcome.clone(),
                json!({ "armed": false, "generation": doc.generation }),
            )
            .map_err(|_| integrity("the pulse outcome is invalid"))?;
            return Ok(mush_core::AuthoritativeActionOutput::new(outcome));
        }
        let clock = pulse_clock(&owner, domain, doc.generation)
            .ok_or_else(|| integrity("that domain's pulse clock is unconstructible"))?;
        // **The authority is idempotent in TRUTH, not only in its key** (fold
        // T1/D3). Re-arming a live cursor used to `txn.replace` it with a fresh
        // `pulse_cursor(.., generation: 1, next_ordinal: 0)` at revision 1 —
        // and because `DurableSchedule::advance_key` hashes
        // `{schedule, owner, generation, next_ordinal, revision}` and NOT
        // `next_due`, the reset cursor minted the byte-identical ingress
        // identity the previous tick had already finalized: `Finalized` →
        // `drive_one` `Err` → `apply_integrity_freeze`. A promoted domain
        // resolved exactly one tick and then wedged the server.
        if let Some(existing) = &prior
            && existing.clock() == &clock
        {
            // Write the record back **byte-identically**: the cursor's
            // generation, ordinal, revision and `next_due` are all unchanged, so
            // `advance_key` is unchanged and no ingress identity repeats. (A
            // wholly empty transaction is refused by the kernel, so the no-op
            // has to be an identical post-image rather than nothing at all.)
            txn.replace(&self.families.schedules, &id, existing.clone())
                .map_err(|error| state_error(&error))?;
            let outcome = RulesValue::new(
                self.outcome.clone(),
                json!({ "armed": true, "generation": doc.generation }),
            )
            .map_err(|_| integrity("the pulse outcome is invalid"))?;
            return Ok(mush_core::AuthoritativeActionOutput::new(outcome));
        }
        let schedule = pulse_cursor(&owner, domain, &clock, doc.from_tick)
            .map_err(|_| integrity("that domain's pulse cursor is unconstructible"))?;
        if prior.is_some() {
            txn.replace(&self.families.schedules, &id, schedule)
        } else {
            txn.insert(&self.families.schedules, &id, schedule)
        }
        .map_err(|error| state_error(&error))?;
        // The first collection interval, so an intent submitted before the first
        // occurrence has a window to join.
        let window_id = tick_window_id(&owner, domain, doc.generation, doc.from_tick)
            .ok_or_else(|| integrity("that tick's window identity is unconstructible"))?;
        if txn
            .get(&self.families.windows, &window_id)
            .map_err(|error| state_error(&error))?
            .is_none()
        {
            let window = open_window(
                window_id,
                &clock,
                doc.from_tick,
                doc.generation,
                secret_version,
            )
            .map_err(|_| integrity("that tick's window is unconstructible"))?;
            txn.insert(
                &self.families.windows,
                &window_id,
                ActionWindowRecord::open(window),
            )
            .map_err(|error| state_error(&error))?;
        }
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({ "armed": true, "generation": doc.generation }),
        )
        .map_err(|_| integrity("the pulse outcome is invalid"))?;
        Ok(mush_core::AuthoritativeActionOutput::new(outcome))
    }
}

/// Who may arm a domain pulse: the same gate cut 7 put on the sensor ingress.
#[must_use]
pub fn pulse_arm_is_permitted(world: &mush_core::World, actor: ObjId) -> bool {
    mush_core::perm::has_power(world, actor, mush_core::Powers::SIM_ADMIN)
        || mush_core::perm::has_role(world, actor, mush_core::Role::Royalty)
}

// ----------------------------------------------------------------------
// B4 — the schedule source
// ----------------------------------------------------------------------

/// The firing half. `plan` re-derives the candidate from stored state and
/// **refuses a stale one rather than acting on it** (`handoff.rs`'s rule), then
/// names the retention sets by a bounded scan of the real families — the one
/// place in the tick lane that has an `&Engine` at all.
///
/// Cut 8's per-intent station re-check went with the combat lane (16b cut T,
/// D-16-20): no submit action writes into a tick window any more, so there is no
/// witness to re-run here.
#[derive(Clone)]
struct DomainTickScheduleAdapter {
    families: TickFamilies,
}

impl ScheduleActionAdapter for DomainTickScheduleAdapter {
    fn plan(
        &self,
        action: &DefinitionRef,
        advance: &ScheduleAdvance,
        engine: &Engine,
    ) -> Result<ScheduleActionDraft, ActionError> {
        let current = engine
            .action_state()
            .get(&self.families.schedules, &advance.schedule())
            .map_err(|_| integrity("the domain pulse cursor is unavailable"))?;
        let candidate_is_current = current.is_some_and(|schedule| {
            matches!(
                schedule.plan_due(schedule.revision(), advance.observed_at(), advance.cause()),
                Ok(ScheduleEvaluation::Advance(planned)) if planned.as_ref() == advance
            )
        });
        if !candidate_is_current {
            return Err(err(
                ActionErrorCode::Stale,
                "the domain pulse candidate is no longer current",
            ));
        }
        let ClockRef::DomainTick { id, generation, .. } = advance.observed_at().clock.clone()
        else {
            return Err(integrity("a domain pulse fires on a domain tick"));
        };
        let domain =
            domain_id_of(&id).ok_or_else(|| integrity("that pulse names no runtime domain"))?;
        let occurrence = advance
            .occurrences()
            .first()
            .ok_or_else(|| integrity("a domain pulse advance carries one occurrence"))?;
        // The occurrence due at T resolves the window that collected `[T-1, T)`.
        let tick = occurrence.due().at.saturating_sub(1);
        let owner = self.families.windows.owner().clone();
        let window = tick_window_id(&owner, domain, generation, tick)
            .ok_or_else(|| integrity("that tick's window identity is unconstructible"))?;
        let state = engine
            .world
            .module_state::<SpatialState>()
            .ok_or_else(|| err(ActionErrorCode::Stale, "that domain is gone"))?;
        let host = state
            .domains
            .get(&domain)
            .map(|record| record.host)
            .ok_or_else(|| err(ActionErrorCode::Stale, "that domain is gone"))?;

        // **The bounded reap sets** (fold D5(a)/B3/A9). The tick provider has an
        // `ActionTxn` and no scan, so the one place on this lane that CAN read
        // the families names what falls out of the ring — from the real records,
        // never from `plan.tick - 64` arithmetic against a non-consecutive
        // cursor. Each set is capped at `MAX_REAP_PER_FIRE` and resumes on the
        // next fire.
        let (reap_commits, reap_windows, reap_orders, reap_progress) =
            self.reap_sets(engine, state, domain, generation, tick)?;

        let definitions = engine
            .registry
            .definitions()
            .ok_or_else(|| integrity("the tick definition registry is unavailable"))?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|_| integrity("the tick guard registry is unavailable"))?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| integrity("the tick definition guard is unavailable"))?;
        Ok(ScheduleActionDraft {
            actor: host,
            subject: PerceivedRef::SelfActor,
            targets: Vec::new(),
            parameters: json!({
                "domain": domain.0,
                "tick": tick,
                "observed_at": advance.observed_at(),
                "window": window,
                "reap_commits": reap_commits,
                "reap_windows": reap_windows,
                "reap_orders": reap_orders,
                "reap_progress": reap_progress,
            }),
            guards: guards
                .finish()
                .map_err(|_| integrity("the tick guards are invalid"))?,
        })
    }
}

/// Visit at most one page, stopping at the removal cap WITHOUT skipping the
/// unvisited suffix. An exhausted page wraps only when the family is exhausted.
fn reap_page<K, V, R>(
    engine: &Engine,
    family: &StateFamilyHandle<K, V>,
    after: Option<&str>,
    select: impl Fn(&V) -> Option<R>,
) -> Result<(Vec<R>, Option<String>), ActionError>
where
    K: Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    let page = engine
        .action_state()
        .scan_family(family, after, MAX_REAP_SCAN_RECORDS)
        .map_err(|error| state_error(&error))?;
    let mut selected = Vec::new();
    for (index, entry) in page.entries().iter().enumerate() {
        if let Some(key) = select(entry.value()) {
            selected.push(key);
        }
        if selected.len() == MAX_REAP_PER_FIRE && index + 1 < page.entries().len() {
            return Ok((selected, Some(entry.encoded_key().to_owned())));
        }
    }
    Ok((selected, page.next_after().map(ToString::to_string)))
}

impl DomainTickScheduleAdapter {
    /// Read-only planning; progress is installed beside the removals in the
    /// same authoritative tick. Other domains' live records cannot hide this
    /// domain's expired records, and refusal/abort cannot skip a page.
    #[allow(clippy::type_complexity)]
    fn reap_sets(
        &self,
        engine: &Engine,
        state: &SpatialState,
        domain: DomainId,
        generation: u64,
        tick: i64,
    ) -> Result<
        (
            Vec<TickCommitKey>,
            Vec<ActionWindowId>,
            Vec<StandingOrderKey>,
            TickReapProgress,
        ),
        ActionError,
    > {
        let progress = engine
            .action_state()
            .get(&self.families.reaping, &domain)
            .map_err(|error| state_error(&error))?
            .cloned()
            .unwrap_or_default();
        let oldest_retained = tick.saturating_sub(MAX_RETAINED_TICK_COMMITS as i64);
        let (commits, commit_cursor) = reap_page(
            engine,
            &self.families.commits,
            progress.commits.as_deref(),
            |commit| {
                (commit.domain() == domain
                    && (commit.generation() != generation || commit.tick() < oldest_retained))
                    .then(|| commit.key())
            },
        )?;
        let (windows, window_cursor) = reap_page(
            engine,
            &self.families.windows,
            progress.windows.as_deref(),
            |record| {
                let ClockRef::DomainTick {
                    id,
                    generation: old_generation,
                    ..
                } = record.window().clock()
                else {
                    return None;
                };
                (domain_id_of(id) == Some(domain)
                    && (*old_generation != generation || record.window().opens().at < tick))
                    .then(|| record.window().id())
            },
        )?;
        let (orders, order_cursor) = reap_page(
            engine,
            &self.families.orders,
            progress.orders.as_deref(),
            |order| {
                crate::graph::effective_domain(state, order.unit())
                    .is_none()
                    .then(|| order.key())
            },
        )?;
        Ok((
            commits,
            windows,
            orders,
            TickReapProgress {
                commits: commit_cursor,
                windows: window_cursor,
                orders: order_cursor,
            },
        ))
    }
}

// ----------------------------------------------------------------------
// B1/B4 — the applying authority
// ----------------------------------------------------------------------

/// Apply cut 6's `DomainActivation` **request** to a domain record: set
/// [`DomainClock::Pulse`] and bump the revision.
///
/// **The `ClockProviderRegistry` is untouched, always.** This writes one field
/// of one `sim` document; the provider that answers the resulting
/// `ClockRef::DomainTick` was registered once, at composition.
///
/// # Errors
///
/// Returns the reason as a string when the domain is not active, is already
/// pulsing, or the requested period is out of band.
pub fn apply_pulse(
    record: &mut DomainRecord,
    period_ms: u32,
    now: &ClockStamp,
) -> Result<u64, String> {
    if record.status != DomainStatus::Active {
        return Err(format!(
            "Domain #{} is not active; only an active domain pulses.",
            record.id
        ));
    }
    let revision = record
        .revision
        .checked_add(1)
        .ok_or_else(|| format!("Domain #{} has exhausted its revisions.", record.id))?;
    // **The generation IS the record's new revision.** A domain revision is
    // strictly increasing and never reused, so a generation is never reused
    // either — and a promote → demote → promote round trip therefore cannot
    // resurrect the first pulse's generation, which would have made an intent
    // pinned to it readable again after a stand-down. (Deriving it from the
    // *previous* generation cannot do this: `Passive` carries none, so the
    // second promotion would start at 1 again.)
    let generation = revision;
    let clock = DomainClock::pulse(period_ms, generation, now.clone())
        .map_err(|error| error.to_string())?;
    record.clock = clock;
    record.revision = revision;
    Ok(generation)
}

/// Stand a pulsing domain back down to `Passive`, leaving every other field
/// alone. A demoted domain keeps its state; only the cadence changes (Q15A).
///
/// # Errors
///
/// Returns the reason when the domain is not pulsing or cannot bump.
pub fn clear_pulse(record: &mut DomainRecord) -> Result<(), String> {
    if !record.clock.is_pulsing() {
        return Err(format!("Domain #{} is not pulsing.", record.id));
    }
    let revision = record
        .revision
        .checked_add(1)
        .ok_or_else(|| format!("Domain #{} has exhausted its revisions.", record.id))?;
    record.clock = DomainClock::Passive;
    record.revision = revision;
    Ok(())
}

// ----------------------------------------------------------------------
// B1/B6 — the world-side applying authority
// ----------------------------------------------------------------------

/// Look up this module's registered `domain_pulse_arm` action by its identity.
///
/// A **string lookup, not an edge**: the definition registry is keyed by
/// `(owner, kind, id)`, so a tick or a command names the action without holding
/// a handle to it (cut 7's `sensor_arm_action` device).
#[must_use]
pub fn arm_action_ref(reg: &mush_core::Registry) -> Option<DefinitionRef> {
    reg.definitions()?
        .current(&mush_core::DefinitionKey {
            owner: OwnerId::new(NS).ok()?,
            kind: DefinitionKind::new("action").ok()?,
            id: DefinitionId::new(ARM_ACTION_ID).ok()?,
        })
        .cloned()
}

/// What one maintenance pass decided, so a command can single-step it and a
/// test can assert on it without reading a log.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PulseMaintenance {
    /// Domains whose live activation request was applied this pass.
    pub applied: Vec<DomainId>,
    /// Domains whose cursor **needs** arming this pass — a domain with no
    /// cursor, or one whose cursor is pinned to a superseded generation.
    ///
    /// **Not "every live pulsing domain"** (fold T1/T3/D3): the old plan pushed
    /// every one on every 1 s fire, so the queue never drained, the single
    /// submission per fire pinned it to the lowest live `DomainId` for ever, a
    /// second promoted domain never got a cursor, and a demoted one beside a
    /// live one was never disarmed.
    pub armed: Vec<DomainId>,
    /// Domains that were stood back down to `Passive` this pass.
    pub stood_down: Vec<DomainId>,
    /// Domains holding a cursor that no longer belongs to a pulsing domain —
    /// the disarm queue, which takes **priority** over arming because a disarm
    /// is a correctness obligation and an arm is not (fold D3(a)).
    pub disarm: Vec<DomainId>,
    /// The cadence each applied domain takes, ground or space (fold B9).
    pub periods: BTreeMap<DomainId, u32>,
    /// Domains refused with the measured reason (never truncated).
    pub refused: Vec<String>,
    /// Whether the visit bound bound. **Paged, never logged** (E4c V6).
    pub bound_hit: bool,
}

/// Read handles on the pulse's own typed families, carried in world state so a
/// staff command can page them.
///
/// `Ctx::scan_action_state` is **owner-fenced**: only `mush-sim` can read these,
/// and it can only read — every write still goes through an `ActionTxn` inside
/// `prepare_authoritative`.
#[derive(Clone)]
pub struct TickHandles {
    /// The `(domain, tick)` commit family `@domain/replay` reads.
    pub commits: StateFamilyHandle<TickCommitKey, DomainTickCommit>,
    /// The `(unit, channel)` standing-order family `@domain/status` renders.
    pub orders: StateFamilyHandle<StandingOrderKey, StandingOrder>,
    /// The per-tick intent window.
    pub windows: StateFamilyHandle<ActionWindowId, ActionWindowRecord>,
    /// The per-domain durable pulse cursor family. The maintenance pass reads
    /// it so it can arm only what is **unarmed** (fold D3(a)); every write is
    /// still `PulseArmProvider`'s, inside an `ActionTxn`.
    pub schedules: StateFamilyHandle<ScheduleId, DurableSchedule>,
    /// Committed progress of the bounded retention sweep.
    pub reaping: StateFamilyHandle<DomainId, TickReapProgress>,
}

impl std::fmt::Debug for TickHandles {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TickHandles")
            .finish_non_exhaustive()
    }
}

/// The live pulse cadence, carried in world state so the composition root can
/// upgrade it from `[tacsim].ground_pulse_ms` / `space_pulse_ms` after
/// composition — the `AccountOracle` device cut 5 used, which keeps
/// `server_modules_with`'s eleven call sites untouched. §7 lists both keys as
/// "parsed and inert until cut 8"; this is the cut that reads them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PulseConfig {
    /// Ground/atmosphere cadence, milliseconds.
    pub ground_ms: u32,
    /// Space cadence, milliseconds.
    pub space_ms: u32,
}

impl Default for PulseConfig {
    fn default() -> PulseConfig {
        PulseConfig {
            ground_ms: crate::data::GROUND_PULSE_MS,
            space_ms: crate::data::SPACE_PULSE_MS,
        }
    }
}

impl PulseConfig {
    /// The cadence for one domain, clamped into the constructible band rather
    /// than refused: a misconfigured host still pulses, at the nearest legal
    /// rate, and the clamp is visible in `@domain/status`.
    #[must_use]
    pub fn period_ms(&self, space: bool) -> u32 {
        let raw = if space { self.space_ms } else { self.ground_ms };
        raw.clamp(
            crate::data::MIN_PULSE_PERIOD_MS,
            crate::data::MAX_PULSE_PERIOD_MS,
        )
    }
}

/// Run one maintenance pass over the promoted domains: apply live activation
/// requests, arm the cursor of a domain that has none, and stand a domain that
/// is no longer eligible back down.
///
/// **The `ClockProviderRegistry` is untouched** — the only writes are one `sim`
/// domain document per applied domain and, at most once per fire, one
/// `Ctx::submit_authoritative` of this module's own `domain_pulse_arm` action
/// (cut 2's fold C4: exactly one submission per command or tick).
pub fn run_maintenance(ctx: &mut mush_core::Ctx<'_>, only: Option<DomainId>) -> PulseMaintenance {
    let now = ctx.w.world_ic_now();
    let config = ctx
        .w
        .module_state::<PulseConfig>()
        .copied()
        .unwrap_or_default();
    // **D3(a):** which domains already hold a cursor, and at which generation.
    // `Ctx::scan_action_state` is owner-fenced and read-only, so consulting the
    // family here costs no authority and is what makes the arm queue finite.
    let cursors: BTreeMap<DomainId, u64> = ctx
        .w
        .module_state::<TickHandles>()
        .cloned()
        .and_then(|handles| {
            ctx.scan_action_state(&handles.schedules, None, MAX_REAP_SCAN_RECORDS)
                .ok()
                .map(|page| {
                    page.entries()
                        .iter()
                        .filter_map(|entry| {
                            let ClockRef::DomainTick { id, generation, .. } = entry.value().clock()
                            else {
                                return None;
                            };
                            domain_id_of(id).map(|domain| (domain, *generation))
                        })
                        .collect()
                })
        })
        .unwrap_or_default();
    // **B9:** which domains take the space cadence. A domain host carrying the
    // `sim.space` behavior pulses at `[tacsim].space_pulse_ms`; every other one
    // takes the ground cadence. Before this fold `period_ms(false)` was the sole
    // production call site and `space_ms` had no reader at all.
    let space: std::collections::BTreeSet<DomainId> = ctx
        .w
        .module_state::<SpatialState>()
        .map(|state| {
            state
                .domains
                .iter()
                .filter(|(_, record)| domain_is_space(ctx.w, record.host))
                .map(|(id, _)| *id)
                .collect()
        })
        .unwrap_or_default();
    let plan = {
        let Some(state) = ctx.w.module_state::<SpatialState>() else {
            return PulseMaintenance::default();
        };
        let book = ctx
            .w
            .module_state::<crate::domain::ActivationBook>()
            .cloned()
            .unwrap_or_default();
        plan_maintenance(&MaintenanceInputs {
            state,
            book: &book,
            now: &now,
            config,
            space: &space,
            cursors: &cursors,
            only,
        })
    };
    // ---- the applying authority: one `sim` document per applied domain ----
    let mut applied_records = Vec::new();
    let mut stood_down_records = Vec::new();
    if let Some(state) = ctx.w.module_state_mut::<SpatialState>() {
        for id in &plan.applied {
            let period = plan
                .periods
                .get(id)
                .copied()
                .unwrap_or_else(|| config.period_ms(false));
            if let Some(record) = state.domains.get_mut(id)
                && apply_pulse(record, period, &now).is_ok()
            {
                applied_records.push(record.clone());
            }
        }
        for id in &plan.stood_down {
            if let Some(record) = state.domains.get_mut(id)
                && clear_pulse(record).is_ok()
            {
                stood_down_records.push(record.clone());
            }
        }
    }
    for record in applied_records.iter().chain(&stood_down_records) {
        crate::data::persist_domain(ctx.out, record);
    }
    // **D9(a):** the journey book follows the domain, both ways. `Promoted` had
    // no writer at all before this fold, so a cruising carrier that entered a
    // live domain was inexpressible; and without the symmetric restore a
    // demoted domain would strand every journey `Promoted` for ever.
    write_journey_promotion(ctx, &plan);
    // ---- one arm/disarm per fire (cut 2's fold C4, unchanged) ----
    let Some(action) = arm_action_ref(ctx.reg) else {
        return plan;
    };
    // **The disarm goes FIRST.** A cursor left on a domain that is no longer
    // pulsing is the standing trigger for the scan pause; an arm that waits one
    // more second costs nothing.
    let mut submitted = None;
    if let Some(id) = plan.disarm.first() {
        let generation = cursors.get(id).copied().unwrap_or(1);
        let units: Vec<ObjId> = ctx
            .w
            .module_state::<SpatialState>()
            .and_then(|state| state.derived.local_objects.get(id).cloned())
            .map(|set| set.into_iter().take(MAX_UNITS_PER_DOMAIN).collect())
            .unwrap_or_default();
        submitted = Some((*id, generation, 0, false, units));
    }
    if submitted.is_none() {
        for id in plan.applied.iter().chain(&plan.armed) {
            let Some(record) = ctx
                .w
                .module_state::<SpatialState>()
                .and_then(|state| state.domains.get(id))
            else {
                continue;
            };
            let Some(generation) = record.clock.generation() else {
                continue;
            };
            let from_tick = record.clock.tick_at(&now).unwrap_or(0);
            submitted = Some((*id, generation, from_tick, true, Vec::new()));
            break;
        }
    }
    let Some((id, generation, from_tick, arm, clear_units)) = submitted else {
        return plan;
    };
    let doc = ArmDoc {
        domain: id.0,
        generation,
        from_tick,
        arm,
        clear_units,
    };
    // **The key drops `from_tick`** (fold T1/D3): it advanced every pulse
    // period, so the submission was never a HIT and every domain tick reset the
    // live cursor to `(generation 1, ordinal 0, revision 1)` — minting the
    // ingress identity the previous tick had already finalized.
    let (Ok(document), Ok(key)) = (
        serde_json::to_string(&doc),
        mush_core::IdempotencyKeyText::parse_canonical(format!(
            "sim.pulse.{}.{}.{}",
            id.0,
            generation,
            u8::from(arm)
        )),
    ) else {
        return plan;
    };
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        Vec::new(),
        json!({ "arm": document }),
        key,
    );
    plan
}

/// Whether one domain host declares the space cadence (fold B9).
#[must_use]
pub fn domain_is_space(world: &mush_core::World, host: ObjId) -> bool {
    mush_core::BehaviorId::new(SPACE_DOMAIN_BEHAVIOR)
        .ok()
        .is_some_and(|behavior| world.has_behavior(host, &behavior))
}

/// **D9(a): the `JourneyState::Promoted` writer, both directions.**
///
/// A `Cruising` carrier whose current leg resolves into a newly pulsing domain
/// becomes `Promoted { domain }`; a carrier `Promoted` into a domain that has
/// just stood down goes back to `Cruising`. `run_maintenance` already holds
/// `&mut Ctx`, `SpatialState` and the `JourneyBook`, so this needed no new seam
/// — §3.8's deviation (g) claimed a dependency on (f) that was a design
/// preference, not a build constraint.
fn write_journey_promotion(ctx: &mut mush_core::Ctx<'_>, plan: &PulseMaintenance) {
    use crate::journey::{JourneyBook, JourneyState};
    if plan.applied.is_empty() && plan.stood_down.is_empty() {
        return;
    }
    let changes: Vec<(ObjId, JourneyState)> = {
        let (Some(state), Some(book)) = (
            ctx.w.module_state::<SpatialState>(),
            ctx.w.module_state::<JourneyBook>(),
        ) else {
            return;
        };
        let mut changes = Vec::new();
        for (root, journey) in &book.journeys {
            match journey.state() {
                JourneyState::Cruising => {
                    if let Some(domain) = journey_domain(state, journey)
                        && plan.applied.contains(&domain)
                    {
                        changes.push((*root, JourneyState::Promoted { domain }));
                    }
                }
                JourneyState::Promoted { domain } if plan.stood_down.contains(domain) => {
                    changes.push((*root, JourneyState::Cruising));
                }
                _ => {}
            }
        }
        changes
    };
    if changes.is_empty() {
        return;
    }
    let mut written = Vec::new();
    if let Some(book) = ctx.w.module_state_mut::<JourneyBook>() {
        for (root, state) in changes {
            if let Some(journey) = book.journeys.remove(&root) {
                let updated = journey.with_state(state);
                written.push(updated.clone());
                book.journeys.insert(root, updated);
            }
        }
    }
    for journey in &written {
        crate::journey::persist_journey(ctx.out, journey);
    }
}

/// The domain a carrier journey is currently in: where its root actually is, or
/// — while it is under way and therefore deliberately unplaced — the domain its
/// destination resolves to.
fn journey_domain(
    state: &SpatialState,
    journey: &crate::journey::CarrierJourney,
) -> Option<DomainId> {
    if let Some(domain) = crate::graph::effective_domain(state, journey.root()) {
        return Some(domain);
    }
    match journey.destination() {
        crate::data::SpatialParent::Direct { domain, .. } => Some(*domain),
        _ => None,
    }
}

/// The world-side maintenance fire.
pub(crate) fn maintenance_tick(
    ctx: &mut mush_core::Ctx<'_>,
    _tag: &str,
    _scene: Option<mush_core::SceneId>,
) {
    let plan = run_maintenance(ctx, None);
    if plan.bound_hit {
        tracing::warn!(
            "sim: the domain pulse maintenance pass stopped at its bound of {} domains",
            MAX_DOMAINS_PER_PULSE_FIRE
        );
    }
    for refusal in &plan.refused {
        tracing::error!(detail = %refusal, "sim: a domain refused promotion to tactical cadence");
    }
}

/// Everything one maintenance decision reads.
///
/// Grouped rather than passed loose because the pass grew two real inputs in
/// this fold — the live cursor set (D3) and the space-cadence set (B9) — and a
/// seven-argument free function is where a caller starts guessing.
pub struct MaintenanceInputs<'a> {
    /// The live spatial state.
    pub state: &'a SpatialState,
    /// Cut 6's activation requests.
    pub book: &'a crate::domain::ActivationBook,
    /// The world IC instant.
    pub now: &'a ClockStamp,
    /// The configured cadences.
    pub config: PulseConfig,
    /// Domains that take the space cadence.
    pub space: &'a std::collections::BTreeSet<DomainId>,
    /// Domain → the generation of its live durable cursor.
    pub cursors: &'a BTreeMap<DomainId, u64>,
    /// Restrict the pass to one domain (`@domain/tick`).
    pub only: Option<DomainId>,
}

/// Decide what the maintenance pass should do, without mutating anything.
///
/// Pure over its inputs, so the world-side tick and `@domain/tick` share one
/// decision and cannot disagree. Bounded on domains **visited** (D-LBK-5).
#[must_use]
pub fn plan_maintenance(input: &MaintenanceInputs<'_>) -> PulseMaintenance {
    let MaintenanceInputs {
        state,
        book,
        now,
        config,
        space,
        cursors,
        only,
    } = input;
    let mut plan = PulseMaintenance::default();
    let mut visited = 0_usize;
    for (id, record) in &state.domains {
        if only.is_some_and(|wanted| wanted != *id) {
            continue;
        }
        visited = visited.saturating_add(1);
        if visited > MAX_DOMAINS_PER_PULSE_FIRE {
            plan.bound_hit = true;
            break;
        }
        let period_ms = config.period_ms(space.contains(id));
        let live = book.is_promoted(*id);
        let units = state
            .derived
            .local_objects
            .get(id)
            .map_or(0, std::collections::BTreeSet::len);
        match (&record.clock, live, record.status == DomainStatus::Active) {
            // A live request on an active, passive domain: apply it.
            (DomainClock::Passive, true, true) => match check_domain_capacity(units) {
                Ok(_) => {
                    let mut candidate = record.clone();
                    match apply_pulse(&mut candidate, period_ms, now) {
                        Ok(_) => {
                            plan.applied.push(*id);
                            plan.periods.insert(*id, period_ms);
                        }
                        Err(reason) => plan.refused.push(reason),
                    }
                }
                Err(reason) => plan.refused.push(reason),
            },
            // Pulsing but no longer eligible: stand it back down. State is
            // preserved both ways; only the cadence changes (Q15A).
            (DomainClock::Pulse { .. }, false, _) | (DomainClock::Pulse { .. }, _, false) => {
                plan.stood_down.push(*id);
            }
            // Pulsing and eligible: arm it **only when it needs arming** — no
            // cursor at all, or a cursor pinned to a superseded generation
            // (fold T1/T3/D3(a)). Re-arming a live cursor is not "a HIT that
            // changes nothing": it reset the cursor and froze the world on the
            // domain's second tick.
            (DomainClock::Pulse { generation, .. }, true, true) => {
                if cursors.get(id) != Some(generation) {
                    plan.armed.push(*id);
                }
                // **B5:** the cap is re-measured on the PULSING arm too, and a
                // domain that has outgrown it is PAGED rather than stood down
                // mid-combat. Before this fold `check_domain_capacity` ran on
                // exactly one arm — the promote transition — so a 65th unit
                // entering an already-pulsing domain was neither refused nor
                // reported anywhere.
                if let Err(reason) = check_domain_capacity(units) {
                    plan.refused.push(format!(
                        "Domain #{id} is already pulsing and has outgrown its cap; it keeps \
                         pulsing and is NOT stood down mid-combat. {reason}"
                    ));
                }
            }
            _ => {}
        }
    }
    // **The disarm queue** (fold T3/D3(a)): a cursor that belongs to no pulsing
    // domain. It covers the domain stood down by this very pass, one whose
    // record has gone entirely, and a leftover from an earlier crash — each of
    // which otherwise leaves a `SimDomainClockProvider::now` that answers
    // `Unavailable` on every 250 ms poll for ever.
    for id in cursors.keys() {
        if only.is_some_and(|wanted| wanted != *id) {
            continue;
        }
        let pulsing = state
            .domains
            .get(id)
            .is_some_and(|record| record.clock.is_pulsing());
        if !pulsing || plan.stood_down.contains(id) {
            plan.disarm.push(*id);
        }
    }
    plan
}

// ----------------------------------------------------------------------
// B5 — the measurement
// ----------------------------------------------------------------------

/// One measured worst-case tick (§9 q6, 08i answer 15).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TickBundleMeasurement {
    /// How many units the encoded tick covered.
    pub units: usize,
    /// Exact replay writes the bundle carries.
    pub replay_writes: usize,
    /// Canonical bytes of the whole encoded bundle.
    pub bundle_bytes: usize,
    /// Canonical bytes of the largest single outbox payload.
    pub outbox_payload_bytes: usize,
}

impl TickBundleMeasurement {
    /// The fraction of `MAX_AUTHORITATIVE_REPLAY_WRITES` this tick uses, in
    /// percent.
    #[must_use]
    pub const fn replay_write_percent(&self) -> usize {
        self.replay_writes * 100 / mush_core::MAX_AUTHORITATIVE_REPLAY_WRITES
    }
    /// The fraction of `MAX_AUTHORITATIVE_BUNDLE_BYTES`, in percent.
    #[must_use]
    pub const fn bundle_percent(&self) -> usize {
        self.bundle_bytes * 100 / mush_core::MAX_AUTHORITATIVE_BUNDLE_BYTES
    }
    /// The fraction of `MAX_DURABLE_OUTBOX_PAYLOAD_BYTES`, in percent.
    #[must_use]
    pub const fn outbox_percent(&self) -> usize {
        self.outbox_payload_bytes * 100 / mush_core::MAX_DURABLE_OUTBOX_PAYLOAD_BYTES
    }
    /// The highest of the three fractions — the cap that binds first.
    #[must_use]
    pub const fn nearest_percent(&self) -> usize {
        let mut worst = self.replay_write_percent();
        if self.bundle_percent() > worst {
            worst = self.bundle_percent();
        }
        if self.outbox_percent() > worst {
            worst = self.outbox_percent();
        }
        worst
    }
    /// Which of the three caps is nearest.
    #[must_use]
    pub fn nearest_cap(&self) -> &'static str {
        let worst = self.nearest_percent();
        if worst == self.replay_write_percent() {
            "MAX_AUTHORITATIVE_REPLAY_WRITES"
        } else if worst == self.outbox_percent() {
            "MAX_DURABLE_OUTBOX_PAYLOAD_BYTES"
        } else {
            "MAX_AUTHORITATIVE_BUNDLE_BYTES"
        }
    }
}

/// **Encode a worst-case tick for `units` units and measure it** — never assume
/// (08i answer 15).
///
/// **The write count is COUNTED, not modelled** (fold B2). Cut 8 hard-coded
/// `units + 1 + 1 + 2` and modelled one `StandingOrder` per unit, but the tick
/// writes one per `StandingOrderKey { unit, channel }` over **four** continuous
/// channels, and K's window slot admits `MAX_INTENTS_PER_TICK` 256 of them — so
/// a 64-unit domain reaches 256 order post-images, not 64. Everything the
/// bundle contains is built here as a real record and the writes are counted
/// off that list, so a future cut that adds a write class moves the number
/// instead of leaving a literal behind.
///
/// The encoded shape is exactly what `DomainTickProvider` writes at saturation:
/// `min(MAX_INTENTS_PER_TICK, units × OrderChannel::all().len())` `StandingOrder`
/// post-images, the consumed window's one-tick retention, the
/// `DomainTickCommit` with one delta per intent, its recorded draws and its
/// `ResolutionRecord`, the next window's post-image, the kernel schedule,
/// retention progress and three full removal sets, and one boundary fact
/// (hence one outbox payload) per intent up to `MAX_BOUNDARY_FACTS_PER_TICK`.
///
/// # Panics
///
/// Never: every fallible step returns `None` instead.
#[must_use]
pub fn measure_tick_bundle(units: usize) -> Option<TickBundleMeasurement> {
    let owner = OwnerId::new(NS).ok()?;
    let domain = DomainId(7);
    let generation = 1_u64;
    let tick = 4_096_i64;
    let clock = pulse_clock(&owner, domain, generation)?;
    let stamp = ClockStamp {
        clock: clock.clone(),
        at: tick,
    };
    let window = tick_window_id(&owner, domain, generation, tick)?;
    // One intent per `(unit, channel)` slot, capped by the window's own
    // capacity — the reachable worst case, not one per unit.
    let channels = OrderChannel::all();
    let intents = units
        .saturating_mul(channels.len())
        .min(MAX_INTENTS_PER_TICK as usize);
    let mut accepted = Vec::with_capacity(intents);
    let mut deltas = Vec::with_capacity(intents);
    let mut facts = Vec::new();
    let mut draws = Vec::with_capacity(intents);
    let mut orders = Vec::with_capacity(intents);
    for index in 0..intents {
        let unit = ObjId(1_000_000 + u32::try_from(index / channels.len()).ok()?);
        let actor = ObjId(2_000_000 + u32::try_from(index).ok()?);
        let channel = channels[index % channels.len()];
        let value = match channel {
            OrderChannel::Heading => OrderValue::Vector {
                x: -999,
                y: 999,
                z: -999,
            },
            OrderChannel::Thrust | OrderChannel::Speed => OrderValue::Scalar { value: -999 },
            OrderChannel::Posture => OrderValue::Emcon { level: 3 },
        };
        let kind = match channel {
            OrderChannel::Heading => IntentKind::Heading,
            OrderChannel::Thrust => IntentKind::Thrust,
            OrderChannel::Speed => IntentKind::Speed,
            OrderChannel::Posture => IntentKind::Posture,
        };
        let request = RequestUid::from_bytes([(index % 251) as u8; 16]);
        accepted.push(IntentRef {
            actor,
            unit,
            kind,
            request,
        });
        deltas.push(UnitDelta {
            unit,
            kind: Some(kind),
            outcome: DeltaOutcome::OrderSet {
                channel,
                value: Some(value.clone()),
                revision: u64::from(u32::MAX),
            },
        });
        draws.push(0xDEAD_BEEF_u32.wrapping_add(index as u32));
        orders.push(
            StandingOrder::new(
                unit,
                channel,
                value,
                stamp.clone(),
                actor,
                u64::from(u32::MAX),
            )
            .ok()?,
        );
        if facts.len() < MAX_BOUNDARY_FACTS_PER_TICK {
            facts.push(BoundaryFact {
                kind: "tactics.sortie_started".to_owned(),
                domain: domain.0,
                generation,
                tick,
                unit,
                ordinal: u16::try_from(facts.len()).ok()?,
                provisional: true,
            });
        }
    }
    let inputs = CanonicalHash::for_canonical_value(
        "sim.domain_tick_inputs.v1",
        1,
        &json!({ "domain": domain, "tick": tick }),
        4096,
    )
    .ok()?;
    let commit = DomainTickCommit::new(
        domain,
        tick,
        clock.clone(),
        window,
        accepted,
        deltas,
        facts.clone(),
        inputs,
        draws.clone(),
        Some(1),
    )
    .ok()?;
    let next_window = ActionWindowRecord::open(
        open_window(
            tick_window_id(&owner, domain, generation, tick + 1)?,
            &clock,
            tick + 1,
            generation,
            Some(1),
        )
        .ok()?,
    );
    // The consumed window is RETAINED (T8), so it is a post-image rather than a
    // removal, and the previous tick's window is reaped beside it.
    let consumed_window =
        ActionWindowRecord::open(open_window(window, &clock, tick, generation, Some(1)).ok()?);
    // The `ResolutionRecord`'s own draws are a fourth relevant cap
    // (`MAX_RESOLUTION_DRAWS`); they were never in the encoded bundle before.
    let resolution = json!({ "domain": "sim.domain_tick", "draws": draws });
    let mut payloads = Vec::with_capacity(facts.len());
    for fact in &facts {
        payloads.push(json!({
            "id": fact.stable_id(),
            "kind": fact.kind,
            "domain": fact.domain,
            "generation": fact.generation,
            "tick": fact.tick,
            "unit": fact.unit,
            "ordinal": fact.ordinal,
            "provisional": fact.provisional,
        }));
    }
    let outbox_payload_bytes = payloads
        .iter()
        .filter_map(|payload| {
            canonical_json_bytes_with_limit(payload, mush_core::MAX_DURABLE_OUTBOX_PAYLOAD_BYTES)
                .ok()
                .map(|bytes| bytes.len())
        })
        .max()
        .unwrap_or(0);
    let reap_progress = TickReapProgress {
        commits: Some("9".repeat(64)),
        windows: Some("f".repeat(64)),
        orders: Some("9".repeat(64)),
    };
    let reap_commits: Vec<_> = (0..MAX_REAP_PER_FIRE)
        .map(|index| TickCommitKey {
            domain,
            generation,
            tick: tick - MAX_RETAINED_TICK_COMMITS as i64 - index as i64 - 1,
        })
        .collect();
    let reap_windows: Vec<_> = (0..MAX_REAP_PER_FIRE)
        .map(|index| tick_window_id(&owner, domain, generation, tick - index as i64 - 1))
        .collect::<Option<Vec<_>>>()?;
    let reap_orders: Vec<_> = (0..MAX_REAP_PER_FIRE)
        .map(|index| StandingOrderKey {
            unit: ObjId(u32::MAX - index as u32),
            channel: OrderChannel::Heading,
        })
        .collect();
    let schedule = pulse_cursor(&owner, domain, &clock, tick + 1).ok()?;
    let bundle = json!({
        "orders": orders,
        "commit": commit,
        "consumed_window": consumed_window,
        "next_window": next_window,
        "resolution": resolution,
        "outbox": payloads,
        "reap_progress": reap_progress,
        "reap_commits": reap_commits,
        "reap_windows": reap_windows,
        "reap_orders": reap_orders,
        "schedule": schedule,
    });
    let bundle_bytes =
        canonical_json_bytes_with_limit(&bundle, mush_core::MAX_AUTHORITATIVE_BUNDLE_BYTES)
            .ok()?
            .len();
    // **Counted off the encoded bundle**, one write per record the tick names:
    // every `StandingOrder` post-image, the commit, the retained consumed
    // window, the next window, sweep progress, the kernel-seeded schedule,
    // and all three bounded removal sets (not just one illustrative deletion).
    let replay_writes = orders
        .len()
        .checked_add(5)
        .and_then(|writes| {
            writes.checked_add(reap_commits.len() + reap_windows.len() + reap_orders.len())
        })
        .filter(|writes| *writes <= mush_core::MAX_AUTHORITATIVE_REPLAY_WRITES)?;
    Some(TickBundleMeasurement {
        units,
        replay_writes,
        bundle_bytes,
        outbox_payload_bytes,
    })
}

// ----------------------------------------------------------------------
// registration
// ----------------------------------------------------------------------

/// The definition descriptors the pulse contributes.
///
/// `mush-sim` may register **one** `definition_validator` per kind, and
/// `handoff.rs` already owns the `"action"`/`"outcome"` ones — so the pulse
/// contributes its descriptors there rather than registering a second validator,
/// exactly as cut 7's sensor ingress does inside `mush-tactics`.
pub(crate) struct TickDescriptors {
    pub(crate) tick_outcome: DefinitionDescriptor,
    pub(crate) arm_outcome: DefinitionDescriptor,
    pub(crate) tick_action: DefinitionDescriptor,
    pub(crate) arm_action: DefinitionDescriptor,
}

/// The registered form of [`TickDescriptors`].
pub(crate) struct TickDefinitions {
    pub(crate) tick_outcome: DefinitionRef,
    pub(crate) arm_outcome: DefinitionRef,
    pub(crate) tick_action: DefinitionRef,
    pub(crate) arm_action: DefinitionRef,
}

/// Build the pulse's descriptors, in registration order.
pub(crate) fn definition_descriptors() -> Result<TickDescriptors, ModError> {
    let tick_outcome = tick_outcome_descriptor()?;
    let arm_outcome = arm_outcome_descriptor()?;
    let tick_action = tick_action_descriptor(&definition_ref_of(&tick_outcome)?)?;
    let arm_action = arm_action_descriptor(&definition_ref_of(&arm_outcome)?)?;
    Ok(TickDescriptors {
        tick_outcome,
        arm_outcome,
        tick_action,
        arm_action,
    })
}

/// The `DefinitionRef` a descriptor carries once registered under this owner.
///
/// The registrar stamps identity from the descriptor itself, so deriving the
/// reference through a throwaway builder is exact rather than a guess — the same
/// device `handoff.rs`'s own tests use to name its refs before registration.
fn definition_ref_of(descriptor: &DefinitionDescriptor) -> Result<DefinitionRef, ModError> {
    let mut definitions = mush_core::DefinitionRegistryBuilder::new();
    definitions
        .register(owner()?, descriptor.clone())
        .map_err(|error| ModError::msg(error.to_string()))
}

/// Register the pulse: five typed families, the window guard validator, the one
/// domain clock provider, two action providers and **one** schedule source.
///
/// The `tactical_intent` submit provider left with the combat lane (16b cut T,
/// D-16-20); what remains is the arm and the tick.
///
/// Called from `SimulationModule::register`, under **this** owner, because the
/// kernel owner fence requires the source's schedule family and fired action to
/// belong to the registering module (cut 7's three fences).
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    definitions: TickDefinitions,
) -> Result<(), ModError> {
    let windows = reg.state_family(
        action_window_state_family_descriptor(
            StateFamilyId::new(WINDOW_FAMILY).map_err(|error| ModError::msg(error.to_string()))?,
        )
        .map_err(|error| ModError::msg(error.to_string()))?,
        ActionWindowStateCodec,
        ActionWindowStateCodec,
        ActionWindowStateCodec,
        ActionWindowStateCodec,
    )?;
    reg.authority_guard_validator(
        AuthorityGuardKind::Window,
        ActionWindowGuardValidator::new(windows.clone()),
    )?;
    let schedules = reg.state_family(
        durable_schedule_state_family_descriptor(
            StateFamilyId::new(SCHEDULE_FAMILY)
                .map_err(|error| ModError::msg(error.to_string()))?,
        )
        .map_err(|error| ModError::msg(error.to_string()))?,
        DurableScheduleStateCodec,
        DurableScheduleStateCodec,
        DurableScheduleStateCodec,
        DurableScheduleStateCodec,
    )?;
    let commits = reg.state_family(
        StateFamilyDescriptor::new(
            StateFamilyId::new(COMMIT_FAMILY).map_err(|error| ModError::msg(error.to_string()))?,
            CanonicalId::new("sim.tick_commit_key.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
            CanonicalId::new("sim.domain_tick_commit.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
            1,
            CanonicalId::new("sim.domain_tick_commit.decode.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
            CanonicalId::new("sim.domain_tick_commit.validate.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
            CanonicalId::new("sim.domain_tick_commit.json.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
        )
        .map_err(|error| ModError::msg(error.to_string()))?
        .with_limits(64, MAX_TICK_COMMIT_BYTES)
        .map_err(|error| ModError::msg(error.to_string()))?,
        |key: &TickCommitKey| Ok(key.encoded()),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|error| error.to_string()),
        |value: &DomainTickCommit| {
            // Re-run the whole wire on validate, so a value that could never
            // have been written is refused rather than stored.
            let encoded = serde_json::to_value(value).map_err(|error| error.to_string())?;
            serde_json::from_value::<DomainTickCommit>(encoded)
                .map(|_| ())
                .map_err(|error| error.to_string())
        },
        |value: &DomainTickCommit| serde_json::to_value(value).map_err(|error| error.to_string()),
    )?;
    let orders = reg.state_family(
        StateFamilyDescriptor::new(
            StateFamilyId::new(ORDER_FAMILY).map_err(|error| ModError::msg(error.to_string()))?,
            CanonicalId::new("sim.standing_order_key.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
            CanonicalId::new("sim.standing_order.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
            1,
            CanonicalId::new("sim.standing_order.decode.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
            CanonicalId::new("sim.standing_order.validate.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
            CanonicalId::new("sim.standing_order.json.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
        )
        .map_err(|error| ModError::msg(error.to_string()))?
        .with_limits(64, MAX_STANDING_ORDER_BYTES)
        .map_err(|error| ModError::msg(error.to_string()))?,
        |key: &StandingOrderKey| Ok(key.encoded()),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|error| error.to_string()),
        |value: &StandingOrder| value.validate_document().map_err(|error| error.to_string()),
        |value: &StandingOrder| serde_json::to_value(value).map_err(|error| error.to_string()),
    )?;
    let reaping = reg.state_family(
        StateFamilyDescriptor::new(
            StateFamilyId::new(REAP_FAMILY).map_err(|error| ModError::msg(error.to_string()))?,
            CanonicalId::new("sim.domain_id.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
            CanonicalId::new("sim.tick_reap_progress.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
            1,
            CanonicalId::new("sim.tick_reap_progress.decode.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
            CanonicalId::new("sim.tick_reap_progress.validate.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
            CanonicalId::new("sim.tick_reap_progress.json.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
        )
        .map_err(|error| ModError::msg(error.to_string()))?
        .with_limits(20, 1024)
        .map_err(|error| ModError::msg(error.to_string()))?,
        |key: &DomainId| Ok(format!("{:020}", key.0)),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|error| error.to_string()),
        TickReapProgress::validate,
        |value: &TickReapProgress| serde_json::to_value(value).map_err(|error| error.to_string()),
    )?;
    let families = TickFamilies {
        windows,
        schedules,
        commits,
        orders,
        reaping,
    };

    // **B1 — the one owner-scoped production domain clock provider.**
    reg.domain_clock_provider(SimDomainClockProvider)?;
    // The READ handles a staff command pages. Owner-fenced by the kernel, so
    // carrying them in world state grants nothing another module could use.
    reg.state(TickHandles {
        commits: families.commits.clone(),
        orders: families.orders.clone(),
        windows: families.windows.clone(),
        schedules: families.schedules.clone(),
        reaping: families.reaping.clone(),
    });

    reg.action_provider(
        definitions.arm_action.clone(),
        PulseArmProvider {
            action: definitions.arm_action,
            outcome: definitions.arm_outcome,
            families: families.clone(),
        },
    )?;
    reg.action_provider(
        definitions.tick_action.clone(),
        DomainTickProvider {
            outcome: definitions.tick_outcome,
            families: families.clone(),
        },
    )?;
    reg.schedule_source(
        CanonicalId::new(SCHEDULE_SOURCE).map_err(|error| ModError::msg(error.to_string()))?,
        definitions.tick_action,
        families.schedules.clone(),
        DomainTickScheduleAdapter { families },
    )?;
    Ok(())
}

/// Build the durable pulse cursor for a domain that has just started pulsing.
///
/// Cadence one tick, `LatestFirst` with a catch-up of one: a tactical pulse that
/// has been asleep resolves **now** rather than replaying an hour of empty ticks,
/// and the forgiven gap is reported by the kernel rather than silently dropped.
///
/// # Errors
///
/// When the identity or the stamp will not carry a cursor.
pub fn pulse_cursor(
    owner: &OwnerId,
    domain: DomainId,
    clock: &ClockRef,
    from_tick: i64,
) -> Result<DurableSchedule, String> {
    let id = tick_schedule_id(owner, domain)
        .ok_or_else(|| "that domain's pulse cursor identity is unconstructible".to_owned())?;
    DurableSchedule::new(DurableScheduleDraft {
        id,
        owner: owner.clone(),
        clock: clock.clone(),
        next_due: ClockStamp {
            clock: clock.clone(),
            at: from_tick.saturating_add(1),
        },
        cadence: Some(1),
        generation: 1,
        next_ordinal: 0,
        restart: ScheduleRestartPolicy::Resume,
        catch_up_order: ScheduleCatchUpOrder::LatestFirst,
        max_catch_up: 1,
    })
    .map_err(|error| error.to_string())
}

/// **The promotion capacity gate (hazard axis 2).** A domain of `units` units
/// either commits inside every K8 cap with the mandated headroom, or it
/// **refuses promotion and pages with the measured size** — it is never
/// truncated and never split silently (§9 q6; 10b §3.8).
///
/// # Errors
///
/// A string carrying the measured figures and the finishing command, ready to
/// go straight into a staff page (08c §4.6: page only for a genuine decision,
/// with the command filled in, and stop only the subject).
pub fn check_domain_capacity(units: usize) -> Result<TickBundleMeasurement, String> {
    let measured = measure_tick_bundle(units).ok_or_else(|| {
        format!(
            "A domain of {units} units will not encode one tick at all; it cannot be promoted. \
             Split it: @domain/create a second host and @spatial/place the surplus into it."
        )
    })?;
    if units > MAX_UNITS_PER_DOMAIN {
        return Err(format!(
            "A domain of {units} units exceeds MAX_UNITS_PER_DOMAIN {MAX_UNITS_PER_DOMAIN}; one \
             tick would encode {} replay writes ({}% of MAX_AUTHORITATIVE_REPLAY_WRITES), {} \
             bundle bytes and {} outbox bytes. Promotion is refused rather than truncated. Split \
             it: @domain/create a second host and @spatial/place the surplus into it.",
            measured.replay_writes,
            measured.replay_write_percent(),
            measured.bundle_bytes,
            measured.outbox_payload_bytes
        ));
    }
    if measured.nearest_percent() >= 60 {
        return Err(format!(
            "A domain of {units} units would encode one tick at {}% of {} ({} replay writes, {} \
             bundle bytes, {} outbox bytes), past the 60% headroom the pin requires. Promotion is \
             refused. Split it: @domain/create a second host and @spatial/place the surplus into \
             it.",
            measured.nearest_percent(),
            measured.nearest_cap(),
            measured.replay_writes,
            measured.bundle_bytes,
            measured.outbox_payload_bytes
        ));
    }
    Ok(measured)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ic(at: i64) -> ClockStamp {
        ClockStamp {
            clock: ClockRef::WorldIc { generation: 1 },
            at,
        }
    }

    /// The one tick function is a pure, deterministic map from the world IC
    /// clock: two reads of one unchanged world agree, and a passive domain has
    /// no tick at all.
    #[test]
    fn e7c8_b1_the_tick_scalar_is_one_pure_function_of_the_world_clock() {
        let clock =
            DomainClock::pulse(crate::data::GROUND_PULSE_MS, 1, ic(100)).expect("a coherent pulse");
        assert_eq!(clock.period_secs(), Some(2));
        assert_eq!(clock.tick_at(&ic(100)), Some(0));
        assert_eq!(clock.tick_at(&ic(101)), Some(0));
        assert_eq!(clock.tick_at(&ic(102)), Some(1));
        assert_eq!(clock.tick_at(&ic(120)), Some(10));
        // Two reads agree.
        assert_eq!(clock.tick_at(&ic(120)), clock.tick_at(&ic(120)));
        // Before the epoch, and on another generation, there is no tick.
        assert_eq!(clock.tick_at(&ic(99)), None);
        assert_eq!(
            clock.tick_at(&ClockStamp {
                clock: ClockRef::WorldIc { generation: 2 },
                at: 200
            }),
            None
        );
        assert_eq!(DomainClock::Passive.tick_at(&ic(120)), None);
        assert_eq!(DomainClock::Passive.generation(), None);
    }

    /// Every arm of the clock wire re-runs its invariant, so a stored clock that
    /// could never have been written is refused on load (§4 rule 1).
    #[test]
    fn e7c8_b1_the_domain_clock_wire_re_runs_period_generation_and_epoch() {
        let clock = DomainClock::pulse(crate::data::SPACE_PULSE_MS, 3, ic(0)).expect("coherent");
        let text = serde_json::to_string(&clock).expect("serializes");
        assert_eq!(
            serde_json::from_str::<DomainClock>(&text).expect("re-enters"),
            clock
        );
        assert_eq!(
            DomainClock::pulse(1, 1, ic(0)),
            Err(crate::data::DomainClockError::PeriodOutOfRange)
        );
        assert_eq!(
            DomainClock::pulse(crate::data::GROUND_PULSE_MS, 0, ic(0)),
            Err(crate::data::DomainClockError::ZeroGeneration)
        );
        assert_eq!(
            DomainClock::pulse(
                crate::data::GROUND_PULSE_MS,
                1,
                ClockStamp {
                    clock: ClockRef::Utc,
                    at: 0
                }
            ),
            Err(crate::data::DomainClockError::NotWorldIc)
        );
        // A stored document with an out-of-band period is refused, not clamped.
        assert!(
            serde_json::from_str::<DomainClock>(
                "{\"mode\":\"Pulse\",\"period_ms\":99999999,\"generation\":1,\"started_at\":\
                 {\"clock\":{\"WorldIc\":{\"generation\":1}},\"at\":0}}"
            )
            .is_err()
        );
    }

    /// The `DomainId(u64)` ↔ `CanonicalId` map the landed `now(.., domain:
    /// &CanonicalId, ..)` signature needs round-trips exactly.
    #[test]
    fn e7c8_b1_the_domain_id_canonical_map_round_trips() {
        for id in [0_u64, 1, 7, 4096, u64::from(u32::MAX)] {
            let canonical = domain_canonical_id(DomainId(id)).expect("a canonical domain id");
            assert_eq!(domain_id_of(&canonical), Some(DomainId(id)));
        }
        // A canonical id that is not a domain answers None rather than guessing.
        assert_eq!(
            domain_id_of(&CanonicalId::new("arena").expect("static")),
            None
        );
    }

    /// A standing order's wire re-runs the band, the channel/value agreement and
    /// the revision, and refuses a stamp that is not a domain tick.
    #[test]
    fn e7c8_b3_the_standing_order_wire_re_runs_band_channel_revision_and_clock() {
        let owner = OwnerId::new(NS).expect("static owner");
        let clock = pulse_clock(&owner, DomainId(1), 1).expect("a domain clock");
        let at = ClockStamp {
            clock: clock.clone(),
            at: 12,
        };
        let order = StandingOrder::new(
            ObjId(9),
            OrderChannel::Heading,
            OrderValue::Vector { x: 1, y: 2, z: 3 },
            at.clone(),
            ObjId(4),
            1,
        )
        .expect("a coherent order");
        let text = serde_json::to_string(&order).expect("serializes");
        assert_eq!(
            serde_json::from_str::<StandingOrder>(&text).expect("re-enters"),
            order
        );
        assert_eq!(
            StandingOrder::new(
                ObjId(9),
                OrderChannel::Thrust,
                OrderValue::Vector { x: 1, y: 2, z: 3 },
                at.clone(),
                ObjId(4),
                1
            ),
            Err(TickRecordError::ChannelValueMismatch)
        );
        assert_eq!(
            StandingOrder::new(
                ObjId(9),
                OrderChannel::Speed,
                OrderValue::Scalar { value: 9_999 },
                at.clone(),
                ObjId(4),
                1
            ),
            Err(TickRecordError::ScalarOutOfRange)
        );
        assert_eq!(
            StandingOrder::new(
                ObjId(9),
                OrderChannel::Speed,
                OrderValue::Scalar { value: 1 },
                at,
                ObjId(4),
                0
            ),
            Err(TickRecordError::ZeroRevision)
        );
        assert_eq!(
            StandingOrder::new(
                ObjId(9),
                OrderChannel::Speed,
                OrderValue::Scalar { value: 1 },
                ic(3),
                ObjId(4),
                1
            ),
            Err(TickRecordError::NotDomainTick)
        );
    }

    /// The state hash covers the accepted set, the RNG root and the ordered
    /// deltas: a replay that omits an accepted intent **fails the hash** rather
    /// than reproducing it.
    ///
    /// **Re-pinned by the E7c8 fold (T7/T12)** with the two arms the original
    /// could not see: a delta whose ORDER VALUE diverges, and an edited `facts`
    /// array.
    #[test]
    fn e7c8_b4_a_commit_that_omits_an_accepted_intent_fails_its_state_hash() {
        let owner = OwnerId::new(NS).expect("static owner");
        let clock = pulse_clock(&owner, DomainId(2), 1).expect("a domain clock");
        let window = tick_window_id(&owner, DomainId(2), 1, 5).expect("a window id");
        let inputs =
            CanonicalHash::for_canonical_value("t.v1", 1, &json!({ "a": 1 }), 256).expect("hash");
        let accepted = vec![
            IntentRef {
                actor: ObjId(1),
                unit: ObjId(1),
                kind: IntentKind::Fire,
                request: RequestUid::from_bytes([1_u8; 16]),
            },
            IntentRef {
                actor: ObjId(2),
                unit: ObjId(2),
                kind: IntentKind::Fire,
                request: RequestUid::from_bytes([2_u8; 16]),
            },
        ];
        let deltas = vec![UnitDelta {
            unit: ObjId(1),
            kind: Some(IntentKind::Fire),
            outcome: DeltaOutcome::Resolved {
                draw: 4,
                effective: true,
            },
        }];
        let facts = vec![BoundaryFact {
            kind: "tactics.fired".to_owned(),
            domain: 2,
            generation: 1,
            tick: 5,
            unit: ObjId(1),
            ordinal: 0,
            provisional: true,
        }];
        let commit =
            |accepted: Vec<IntentRef>, deltas: Vec<UnitDelta>, facts: Vec<BoundaryFact>| {
                DomainTickCommit::new(
                    DomainId(2),
                    5,
                    clock.clone(),
                    window,
                    accepted,
                    deltas,
                    facts,
                    inputs,
                    vec![4],
                    Some(1),
                )
                .expect("a coherent commit")
            };
        let full = commit(accepted.clone(), deltas.clone(), facts.clone());
        let short = commit(accepted[..1].to_vec(), deltas.clone(), facts.clone());
        assert_ne!(
            full.state_hash(),
            short.state_hash(),
            "an omitted accepted intent must fail the hash"
        );
        // **T12:** the facts are inside the hash, so an edited fact list fails it.
        let no_facts = commit(accepted.clone(), deltas, Vec::new());
        assert_ne!(
            full.state_hash(),
            no_facts.state_hash(),
            "a commit whose fact list is edited must fail its hash"
        );
        // **T7:** two commits differing only in an ORDER VALUE have different
        // hashes. Before the fold the delta carried only its channel name, so
        // the entire mutable result of the tick was outside the hash.
        let order_of = |x: i32, revision: u64| {
            vec![UnitDelta {
                unit: ObjId(1),
                kind: Some(IntentKind::Heading),
                outcome: DeltaOutcome::OrderSet {
                    channel: OrderChannel::Heading,
                    value: Some(OrderValue::Vector { x, y: 0, z: 0 }),
                    revision,
                },
            }]
        };
        let one = commit(accepted.clone(), order_of(1, 1), Vec::new());
        let other = commit(accepted.clone(), order_of(999, 7), Vec::new());
        assert_ne!(
            one.state_hash(),
            other.state_hash(),
            "two commits differing only in an order value shared a hash"
        );
        // Re-entering the full commit reproduces its hash exactly.
        let text = serde_json::to_string(&full).expect("serializes");
        assert_eq!(
            serde_json::from_str::<DomainTickCommit>(&text).expect("re-enters"),
            full
        );
        // A document whose stored hash disagrees with its contents is refused.
        let mut wire: serde_json::Value = serde_json::from_str(&text).expect("json");
        wire["state_hash"] = json!(short.state_hash());
        assert!(serde_json::from_value::<DomainTickCommit>(wire).is_err());
        // And so is one whose FACTS were edited under an unchanged hash.
        let mut edited: serde_json::Value = serde_json::from_str(&text).expect("json");
        edited["facts"] = json!([]);
        assert!(serde_json::from_value::<DomainTickCommit>(edited).is_err());
    }

    /// **B5 — the measurement.** A tick at the pinned cap fits inside every K8
    /// bound with the mandated headroom, and the test names the nearest cap.
    #[test]
    fn e7c8_b5_a_tick_at_the_measured_cap_stays_under_sixty_percent_of_every_k8_cap() {
        let measured = measure_tick_bundle(MAX_UNITS_PER_DOMAIN).expect("the cap encodes");
        assert_eq!(measured.units, MAX_UNITS_PER_DOMAIN);
        assert!(
            measured.replay_writes < mush_core::MAX_AUTHORITATIVE_REPLAY_WRITES,
            "replay writes {} exceed the cap",
            measured.replay_writes
        );
        assert!(
            measured.bundle_bytes < mush_core::MAX_AUTHORITATIVE_BUNDLE_BYTES,
            "bundle bytes {} exceed the cap",
            measured.bundle_bytes
        );
        assert!(
            measured.outbox_payload_bytes < mush_core::MAX_DURABLE_OUTBOX_PAYLOAD_BYTES,
            "outbox payload {} exceeds the cap",
            measured.outbox_payload_bytes
        );
        assert!(
            measured.nearest_percent() < 60,
            "a tick at MAX_UNITS_PER_DOMAIN={} uses {}% of {} ({} replay writes, {} bundle bytes, \
             {} outbox bytes); the pin fails past 60%",
            MAX_UNITS_PER_DOMAIN,
            measured.nearest_percent(),
            measured.nearest_cap(),
            measured.replay_writes,
            measured.bundle_bytes,
            measured.outbox_payload_bytes
        );
    }

    /// Applying the pulse bumps the generation and never loses domain state;
    /// standing down restores `Passive` with everything else untouched (Q15A).
    #[test]
    fn e7c8_b1_apply_and_clear_pulse_change_only_the_cadence() {
        let mut record = DomainRecord {
            id: DomainId(3),
            host: ObjId(11),
            driver: crate::data::passive_definition_ref(),
            status: DomainStatus::Active,
            clock: DomainClock::Passive,
            presentation_location: ObjId(12),
            revision: 4,
            window: None,
        };
        let before = record.clone();
        let generation = apply_pulse(&mut record, crate::data::GROUND_PULSE_MS, &ic(50))
            .expect("an active domain applies");
        // The generation IS the record's new revision — monotone and never
        // reused, which is what a stand-down and a re-promotion must not undo.
        assert_eq!(generation, 5);
        assert!(record.clock.is_pulsing());
        assert_eq!(record.revision, 5);
        assert_eq!(record.host, before.host);
        assert_eq!(record.presentation_location, before.presentation_location);
        // A second apply opens a NEW generation, which is what makes an intent
        // pinned to the old one unreadable rather than silently re-timed.
        let second = apply_pulse(&mut record, crate::data::SPACE_PULSE_MS, &ic(60))
            .expect("a pulsing domain re-applies");
        assert!(second > generation);
        clear_pulse(&mut record).expect("a pulsing domain stands down");
        assert_eq!(record.clock, DomainClock::Passive);
        assert_eq!(record.status, before.status);
        assert!(clear_pulse(&mut record).is_err());
        // And a re-promotion after the stand-down never reuses a generation.
        let third = apply_pulse(&mut record, crate::data::GROUND_PULSE_MS, &ic(70))
            .expect("a stood-down domain re-promotes");
        assert!(
            third > second,
            "a generation was reused across a stand-down"
        );
        clear_pulse(&mut record).expect("stands down again");
        // A paused domain refuses the pulse entirely.
        record.status = DomainStatus::Paused;
        assert!(apply_pulse(&mut record, crate::data::GROUND_PULSE_MS, &ic(70)).is_err());
    }
}

#[cfg(test)]
mod measurement_report {
    use super::*;

    /// **B5 — the measurement table (§9 q6).** Printed with `--nocapture` and
    /// folded into 10b §7; the assertions are the pin.
    #[test]
    fn e7c8_b5_the_measurement_table_names_the_nearest_cap_at_every_n() {
        let mut rows = Vec::new();
        for units in [1_usize, 8, 16, 32, 64, 96, 128, 192, 256] {
            let Some(m) = measure_tick_bundle(units) else {
                continue;
            };
            println!(
                "N={:>4}  writes={:>5} ({:>2}%)  bundle={:>8}B ({:>2}%)  outbox={:>5}B ({:>2}%)  \
                 nearest={} at {}%",
                m.units,
                m.replay_writes,
                m.replay_write_percent(),
                m.bundle_bytes,
                m.bundle_percent(),
                m.outbox_payload_bytes,
                m.outbox_percent(),
                m.nearest_cap(),
                m.nearest_percent()
            );
            rows.push(m);
        }
        assert!(!rows.is_empty());
        // Monotone in N on every axis, so "the cap N" is a real frontier.
        for pair in rows.windows(2) {
            assert!(pair[1].replay_writes >= pair[0].replay_writes);
            assert!(pair[1].bundle_bytes >= pair[0].bundle_bytes);
        }
        // The nearest cap is the same one at every N the pulse can reach: the
        // replay-write COUNT, never the two byte caps.
        assert!(
            rows.iter()
                .all(|row| row.nearest_cap() == "MAX_AUTHORITATIVE_REPLAY_WRITES")
        );
    }

    /// **Hazard axis 2, both halves.** A domain AT the measured cap promotes
    /// inside every K8 bound; **one unit past it refuses and pages with the
    /// measured size** and the finishing command — never a silent truncation and
    /// never a silent split.
    #[test]
    fn e7c8_c_a_domain_at_the_cap_promotes_and_one_unit_past_it_refuses_with_its_measurement() {
        let ok = check_domain_capacity(MAX_UNITS_PER_DOMAIN).expect("the cap promotes");
        assert!(ok.nearest_percent() < 60);
        let refusal = check_domain_capacity(MAX_UNITS_PER_DOMAIN + 1)
            .expect_err("one unit past the cap refuses");
        // The page carries the numbers AND the command a person would run.
        assert!(
            refusal.contains("65 units exceeds MAX_UNITS_PER_DOMAIN 64"),
            "{refusal}"
        );
        assert!(refusal.contains("replay writes"), "{refusal}");
        assert!(refusal.contains("@domain/create"), "{refusal}");
        assert!(
            refusal.contains("refused rather than truncated"),
            "{refusal}"
        );
    }

    /// **The measured frontier, stated honestly — re-tabulated by the E7c8 fold
    /// (B2).** No K8 cap ever binds a tick: the encoded bundle stays under 60 %
    /// of all three at **every** N the window can admit, and what actually stops
    /// the count is `MAX_WINDOW_CAPACITY`/`MAX_INTENTS_PER_TICK` 256.
    ///
    /// What the fold corrected is where that ceiling sits. A window slot is one
    /// intent per `(actor, truth_subject)`, and the tick writes one
    /// `StandingOrder` per `(unit, channel)` over **four** continuous channels —
    /// so 64 units reach 256 order post-images and **453 replay writes (44 %)**,
    /// not 68 (6 %). `MAX_UNITS_PER_DOMAIN` 64 is therefore **exactly** the
    /// window's own ceiling on the continuous surface, not four times below it,
    /// and it remains a latency and offer-budget policy figure — still four
    /// times inside the replay-write cap.
    #[test]
    fn e7c8_b5_no_k8_cap_binds_before_the_window_capacity_does() {
        let mut frontier = MAX_UNITS_PER_DOMAIN;
        for units in MAX_UNITS_PER_DOMAIN..=(MAX_INTENTS_PER_TICK as usize) {
            let Some(measured) = measure_tick_bundle(units) else {
                break;
            };
            assert!(
                measured.nearest_percent() < 60,
                "N={units} already uses {}% of {}",
                measured.nearest_percent(),
                measured.nearest_cap()
            );
            frontier = units;
        }
        println!("every N up to {frontier} stays under 60% of every K8 cap");
        assert_eq!(frontier, MAX_INTENTS_PER_TICK as usize);
        // The window's own ceiling is what stops the count, and it is reached at
        // exactly `MAX_UNITS_PER_DOMAIN` units: past that the intent count is
        // clamped by `MAX_INTENTS_PER_TICK`, so the bundle stops growing.
        let at_cap = measure_tick_bundle(MAX_UNITS_PER_DOMAIN).expect("the cap encodes");
        let past_cap = measure_tick_bundle(MAX_INTENTS_PER_TICK as usize).expect("encodes");
        assert_eq!(
            at_cap.replay_writes, past_cap.replay_writes,
            "the window capacity, not the unit count, is what stops the write count"
        );
        assert_eq!(
            at_cap.replay_writes, 453,
            "the measured worst case at MAX_UNITS_PER_DOMAIN"
        );
        assert_eq!(at_cap.replay_write_percent(), 44);
        assert_eq!(
            MAX_UNITS_PER_DOMAIN * OrderChannel::all().len(),
            MAX_INTENTS_PER_TICK as usize,
            "the unit cap sits exactly on the window's own ceiling"
        );
    }
}
