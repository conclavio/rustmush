//! # The minimal deterministic pulse driver — E7 cut 8 (10b §3.8)
//!
//! One promoted domain, one `ClockRef::DomainTick` scalar, one frozen
//! `ActionWindowRecord` of accepted intents per tick, and **one deterministic
//! domain-tick commit per domain per tick** with recorded RNG.
//!
//! ## What this module builds, and what it only drives
//!
//! Cut 8 **defines**: [`DomainClock::Pulse`](crate::data::DomainClock::Pulse)
//! (in `data.rs`), the production [`SimDomainClockProvider`], the
//! [`StandingOrder`] family, the [`DomainTickCommit`] family, the
//! `tactical_intent` submit action, the `domain_tick` resolution action, the
//! `domaintick` schedule source, and the boundary facts that leave the bundle
//! through one `mush-tactics`-owned durable outbox adapter.
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
    CanonicalId, ClockProviderContext, ClockProviderFailure, ClockRef, ClockStamp, ContactId,
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, DurableOutboxIntent,
    DurableSchedule, DurableScheduleDraft, DurableScheduleStateCodec, Engine, LegalActionQuery,
    ModError, ObjId, OwnerId, PerceivedRef, ProjectedParameter, ProjectedParameterKind,
    ProjectedParameterSchema, ProjectedTargetSchema, ProviderOfferDraft, Registrar, RequestUid,
    ResolvedActionInvocation, RevisionSet, RulesValue, ScheduleActionAdapter, ScheduleActionDraft,
    ScheduleAdvance, ScheduleCatchUpOrder, ScheduleEvaluation, ScheduleId, ScheduleRestartPolicy,
    StateFamilyDescriptor, StateFamilyHandle, StateFamilyId, SubjectKey, WindowIntent, WindowMode,
    WindowOrderKey, WindowRestartPolicy, WindowTimeoutPolicy,
    action_window_state_family_descriptor, canonical_json_bytes_with_limit,
    durable_schedule_state_family_descriptor,
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
/// The submit action every tactical intent enters through.
pub const INTENT_ACTION_ID: &str = "tactical_intent";
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
/// The submit action's outcome definition.
pub const INTENT_OUTCOME_ID: &str = "tactical_intent_accepted";

/// The per-domain intent window family (K's `ActionWindowRecord`).
pub const WINDOW_FAMILY: &str = "tick_windows";
/// The per-domain durable pulse cursor family.
pub const SCHEDULE_FAMILY: &str = "domaintick_schedules";
/// The per-`(domain, tick)` deterministic commit family.
pub const COMMIT_FAMILY: &str = "domain_tick_commits";
/// The per-`(unit, channel)` standing-order family.
pub const ORDER_FAMILY: &str = "standing_orders";

/// The `OwnerId` the tick's boundary facts are stamped with, and the
/// `DurableOutboxAdapter` that carries them out of the bundle. They are
/// **tactics'** facts even though `mush-sim` owns the pulse that emits them
/// (10b §3.8 OPEN 4; D-E7-17's T2 boundary).
pub const BOUNDARY_OUTBOX_OWNER: &str = "tactics";
/// The adapter id the tactics boundary-fact outbox registers under.
pub const BOUNDARY_OUTBOX_ADAPTER: &str = "tactics.boundary";

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

/// The parameters one tactical intent carries. Built by a trusted command
/// handler (or by the deterministic fallback controller), never by a client:
/// the station/epoch witness is pinned here at submit and **re-checked at
/// resolution** through `World::channel_grant_is_current` (cut 5's rule).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TacticalIntentDoc {
    /// The domain the unit is in.
    pub domain: u64,
    /// The tick the intent is submitted into.
    pub tick: i64,
    /// The commanded unit.
    pub unit: ObjId,
    /// The station the actor is focused on.
    pub station: String,
    /// The unit's control-lease/grant epoch, pinned at submit.
    pub epoch: u64,
    /// The channel the station grants.
    pub channel: String,
    /// What is intended.
    pub kind: IntentKind,
    /// The continuous value, for a continuous kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<OrderValue>,
    /// The opaque contact a discrete kind targets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact: Option<ContactId>,
    /// The bay or section a discrete kind names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<String>,
}

impl TacticalIntentDoc {
    /// Structural coherence: a continuous kind carries a matching value, a
    /// discrete one does not.
    ///
    /// # Errors
    ///
    /// [`TickRecordError::ChannelValueMismatch`] when the two disagree.
    pub fn validate(&self) -> Result<(), TickRecordError> {
        match (self.kind.channel(), &self.value) {
            (Some(channel), Some(value)) => {
                value.validate()?;
                if value.fits(channel) {
                    Ok(())
                } else {
                    Err(TickRecordError::ChannelValueMismatch)
                }
            }
            (None, None) => Ok(()),
            _ => Err(TickRecordError::ChannelValueMismatch),
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
    OrderSet {
        /// The channel it settled on.
        channel: OrderChannel,
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
    /// What was intended.
    pub kind: IntentKind,
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
    /// The tick it happened on.
    pub tick: i64,
    /// The unit it happened to.
    pub unit: ObjId,
}

impl BoundaryFact {
    /// The stable id cut 9 indexes on: `{kind}/{domain}/{tick}/{unit}`.
    #[must_use]
    pub fn stable_id(&self) -> String {
        format!(
            "{}/{}/{}/{}",
            self.kind, self.domain, self.tick, self.unit.0
        )
    }
}

/// The `(domain, tick)` key one commit occupies.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TickCommitKey {
    /// The domain.
    pub domain: DomainId,
    /// The tick.
    pub tick: i64,
}

impl TickCommitKey {
    /// The canonical encoded key. Zero-padded so the family's canonical scan
    /// order is tick order.
    #[must_use]
    pub fn encoded(&self) -> String {
        format!("{:020}:{:020}", self.domain.0, self.tick.max(0))
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
    tick: i64,
    clock: ClockRef,
    window: ActionWindowId,
    accepted: Vec<IntentRef>,
    deltas: Vec<UnitDelta>,
    facts: Vec<BoundaryFact>,
    rng_inputs: CanonicalHash,
    draws: Vec<u32>,
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
    /// The hash over the accepted set, the RNG root and the ordered deltas.
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
    #[allow(clippy::too_many_arguments)]
    pub fn derive_state_hash(
        domain: DomainId,
        tick: i64,
        clock: &ClockRef,
        window: ActionWindowId,
        accepted: &[IntentRef],
        deltas: &[UnitDelta],
        rng_inputs: CanonicalHash,
        draws: &[u32],
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
                "rng_inputs": rng_inputs,
                "draws": draws,
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
    ) -> Result<DomainTickCommit, TickRecordError> {
        if !matches!(clock, ClockRef::DomainTick { .. }) {
            return Err(TickRecordError::NotDomainTick);
        }
        if deltas.len() > MAX_TICK_DELTAS || accepted.len() > MAX_TICK_DELTAS {
            return Err(TickRecordError::TooManyDeltas);
        }
        if facts.len() > MAX_BOUNDARY_FACTS_PER_TICK {
            return Err(TickRecordError::TooManyFacts);
        }
        let state_hash = Self::derive_state_hash(
            domain, tick, &clock, window, &accepted, &deltas, rng_inputs, &draws,
        )?;
        let commit = DomainTickCommit {
            domain,
            tick,
            clock,
            window,
            accepted,
            deltas,
            facts,
            rng_inputs,
            draws,
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

/// The stable window identity for one `(domain, tick)`.
#[must_use]
pub fn tick_window_id(owner: &OwnerId, domain: DomainId, tick: i64) -> Option<ActionWindowId> {
    let hash = CanonicalHash::for_canonical_value(
        "sim.domain_tick_window_local.v1",
        1,
        &json!({ "domain": domain, "tick": tick }),
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
            "fields": { "tick": "i64", "accepted": "usize", "facts": "usize" },
            "contains_truth_subject": false
        }),
    )?
    .as_current())
}

/// The submit outcome descriptor.
pub(crate) fn intent_outcome_descriptor() -> Result<DefinitionDescriptor, ModError> {
    Ok(descriptor(
        "outcome",
        INTENT_OUTCOME_ID,
        json!({
            "fields": { "tick": "i64", "replaced": "bool" },
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

/// The submit action descriptor.
pub(crate) fn intent_action_descriptor(
    outcome: &DefinitionRef,
) -> Result<DefinitionDescriptor, ModError> {
    Ok(descriptor(
        "action",
        INTENT_ACTION_ID,
        json!({
            "internal": false,
            "subject": "self_actor",
            "targets": 0,
            "state_family": WINDOW_FAMILY,
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

fn err(code: ActionErrorCode, message: &'static str) -> ActionError {
    ActionError::new(code, false, message).expect("static tick error is bounded")
}

fn integrity(message: &'static str) -> ActionError {
    err(ActionErrorCode::Integrity, message)
}

fn state_error(_error: &ActionStateError) -> ActionError {
    integrity("the tactical tick's own state is unavailable")
}

/// The one refusal a viewer may not distinguish: naming a contact the actor does
/// not hold is byte-identical to naming one that never existed (§4 rule 4).
const NO_SUCH_CONTACT: &str = "You hold no such contact.";

// ----------------------------------------------------------------------
// B3 — the submit provider
// ----------------------------------------------------------------------

/// The one ingress every tactical intent takes.
///
/// **It opens the window lazily and submits into it.** `checked_submit` accepts
/// one intent per `(actor, truth_subject)` slot and a resubmission **replaces**
/// it, which is S2A's latest-order-wins for free; a 257th distinct slot refuses
/// on `MAX_WINDOW_CAPACITY`.
#[derive(Clone)]
struct TacticalIntentProvider {
    action: DefinitionRef,
    outcome: DefinitionRef,
    families: TickFamilies,
}

impl TacticalIntentProvider {
    fn document(invocation: &ActionInvocation) -> Result<TacticalIntentDoc, ActionError> {
        let raw = invocation
            .parameters()
            .get("intent")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                err(
                    ActionErrorCode::InvalidRequest,
                    "a tactical intent carries one `intent` document",
                )
            })?;
        let doc: TacticalIntentDoc = serde_json::from_str(raw).map_err(|_| {
            err(
                ActionErrorCode::InvalidRequest,
                "that tactical intent will not decode",
            )
        })?;
        doc.validate().map_err(|_| {
            err(
                ActionErrorCode::InvalidRequest,
                "that tactical intent's value does not match its kind",
            )
        })?;
        Ok(doc)
    }

    fn observed_at(invocation: &ActionInvocation) -> Result<ClockStamp, ActionError> {
        let value = invocation
            .parameters()
            .get("observed_at")
            .cloned()
            .ok_or_else(|| integrity("a tactical intent carries its domain-tick stamp"))?;
        let stamp: ClockStamp = serde_json::from_value(value)
            .map_err(|_| integrity("that tactical intent's stamp will not decode"))?;
        if !matches!(stamp.clock, ClockRef::DomainTick { .. }) {
            return Err(err(
                ActionErrorCode::InvalidRequest,
                "a tactical intent is stamped on its domain's tick clock",
            ));
        }
        Ok(stamp)
    }
}

impl ActionProvider for TacticalIntentProvider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        if invocation.subject() != &PerceivedRef::SelfActor || !invocation.targets().is_empty() {
            return Err(err(
                ActionErrorCode::InvalidRequest,
                "a tactical intent acts on self and names its contact opaquely",
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
        {
            return Ok(None);
        }
        // **One offer per action kind, never one per target** (08i answer 11):
        // the whole tactical vocabulary is a single offer whose `intent` document
        // names the kind, and the opaque contact choices ride the offer's own
        // ≤ `MAX_PROJECTED_ACTION_CHOICES` bound. There is no per-station
        // `ProjectionScopeId`.
        let definitions = engine
            .registry
            .definitions()
            .ok_or_else(|| integrity("the tactical definition registry is unavailable"))?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|_| integrity("the tactical guard registry is unavailable"))?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| integrity("the tactical definition guard is unavailable"))?;
        Ok(Some(ProviderOfferDraft {
            fixed_parameters: json!({}),
            parameters: ProjectedParameterSchema::new(vec![
                ProjectedParameter {
                    id: CanonicalId::new("intent")
                        .map_err(|_| integrity("the tactical parameter id is invalid"))?,
                    required: true,
                    kind: ProjectedParameterKind::Text {
                        maximum_bytes: mush_core::MAX_PROJECTED_ACTION_TEXT_BYTES,
                    },
                },
                ProjectedParameter {
                    id: CanonicalId::new("observed_at")
                        .map_err(|_| integrity("the tactical parameter id is invalid"))?,
                    required: true,
                    kind: ProjectedParameterKind::Text {
                        maximum_bytes: mush_core::MAX_PROJECTED_ACTION_TEXT_BYTES,
                    },
                },
            ])
            .map_err(|_| integrity("the tactical parameter schema is invalid"))?,
            targets: ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: ActionPresentation::new(
                "tactical_intent",
                "submit a tactical intent",
                "join this tick's frozen intent set",
            )
            .map_err(|_| integrity("the tactical presentation is invalid"))?,
            guards: guards
                .finish()
                .map_err(|_| integrity("the tactical offer guards are invalid"))?,
        }))
    }

    fn prepare_invocation(
        &self,
        _invocation: &ActionInvocation,
        _txn: &mut ActionTxn<'_>,
    ) -> Result<(), ActionError> {
        Err(err(
            ActionErrorCode::Unavailable,
            "a tactical intent requires the authoritative action coordinator",
        ))
    }

    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<mush_core::AuthoritativeActionOutput, ActionError> {
        self.validate_invocation(invocation)?;
        let doc = Self::document(invocation)?;
        let now = Self::observed_at(invocation)?;
        let owner = self.families.windows.owner().clone();
        let domain = DomainId(doc.domain);
        let window_id = tick_window_id(&owner, domain, doc.tick)
            .ok_or_else(|| integrity("that domain's window identity is unconstructible"))?;
        let ClockRef::DomainTick { generation, .. } = &now.clock else {
            return Err(integrity("a tactical intent is stamped on a domain tick"));
        };
        let generation = *generation;
        let existing = txn
            .get(&self.families.windows, &window_id)
            .map_err(|error| state_error(&error))?
            .cloned();
        let record = match existing {
            Some(record) => record,
            None => ActionWindowRecord::open(
                open_window(window_id, &now.clock, doc.tick, generation)
                    .map_err(|_| integrity("that tick's window is unconstructible"))?,
            ),
        };
        let intent = WindowIntent::new(
            window_id,
            generation,
            SubjectKey::Object { id: doc.unit },
            Vec::new(),
            self.action.clone(),
            invocation.clone(),
            WindowOrderKey::new(vec![doc.kind.order_rank(), doc.unit.0 as i64])
                .map_err(|_| integrity("that intent's order key is invalid"))?,
        )
        .map_err(|_| integrity("that intent will not join a window"))?;
        let transition = record
            .checked_submit(record.revision(), &now, intent)
            .map_err(|error| match error {
                mush_core::ActionWindowError::CapacityReached { .. } => err(
                    ActionErrorCode::BoundsExceeded,
                    "this tick's intent window is full; the next tick opens a fresh one",
                ),
                mush_core::ActionWindowError::OutsideWindow
                | mush_core::ActionWindowError::NotOpen { .. } => err(
                    ActionErrorCode::Stale,
                    "that tick has already closed; re-issue the order",
                ),
                _ => integrity("that intent could not be admitted"),
            })?;
        let replaced = matches!(
            transition.receipt().result(),
            mush_core::WindowMutationResult::Replaced { .. }
        );
        let updated = transition.into_record();
        if txn
            .get(&self.families.windows, &window_id)
            .map_err(|error| state_error(&error))?
            .is_some()
        {
            txn.replace(&self.families.windows, &window_id, updated)
        } else {
            txn.insert(&self.families.windows, &window_id, updated)
        }
        .map_err(|error| state_error(&error))?;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({ "tick": doc.tick, "replaced": replaced }),
        )
        .map_err(|_| integrity("the tactical outcome is invalid"))?;
        Ok(mush_core::AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

/// One tick's collection interval: `[tick, tick + 1)` on the domain's own clock.
fn open_window(
    id: ActionWindowId,
    clock: &ClockRef,
    tick: i64,
    generation: u64,
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
        resolution_secret_version: None,
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
    /// The intents whose station/channel/epoch witness was still current when
    /// the adapter planned this fire (cut 5's rule: pinned at submit, re-checked
    /// against `World::channel_grant_is_current` at apply). Every accepted
    /// intent NOT in this list is **refused with a reason**, never skipped.
    current: Vec<RequestUid>,
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
        let current: Vec<RequestUid> = parameters
            .get("current")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .ok_or_else(|| integrity("a domain tick names its revalidated set"))?;
        Ok(TickPlan {
            domain,
            tick,
            observed_at,
            window,
            current,
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
        let (accepted_docs, consumed) = match record {
            Some(record) => {
                let closed = if record.phase() == mush_core::ActionWindowPhase::Open {
                    record
                        .checked_close_due(
                            record.revision(),
                            Self::operation(invocation.request(), "close")?,
                            &plan.observed_at,
                        )
                        .map_err(|_| integrity("this tick's window would not close"))?
                        .into_record()
                } else {
                    record
                };
                let ordered = closed
                    .resolution_order()
                    .map_err(|_| integrity("this tick's window would not order"))?;
                let mut docs = Vec::with_capacity(ordered.len());
                for intent in ordered {
                    let request = intent.invocation().request();
                    let doc = TacticalIntentProvider::document(intent.invocation()).ok();
                    docs.push((request, intent.invocation().actor(), doc));
                }
                let consumed = closed
                    .checked_consume(
                        closed.revision(),
                        Self::operation(invocation.request(), "consume")?,
                        commit_token,
                        &plan.observed_at,
                    )
                    .map_err(|_| integrity("this tick's window would not consume"))?
                    .into_record();
                (docs, Some(consumed))
            }
            None => (Vec::new(), None),
        };

        // ---- recorded RNG (K3.1a): one stream, never re-drawn on replay ----
        let rng_domain = CanonicalId::new("sim.domain_tick")
            .map_err(|_| integrity("the tick's rng domain is invalid"))?;
        let inputs_hash = CanonicalHash::for_canonical_value(
            "sim.domain_tick_inputs.v1",
            1,
            &json!({
                "domain": plan.domain,
                "tick": plan.tick,
                "window": plan.window,
                "accepted": accepted_docs
                    .iter()
                    .map(|(request, actor, doc)| json!({
                        "request": request,
                        "actor": actor,
                        "kind": doc.as_ref().map(|doc| doc.kind),
                    }))
                    .collect::<Vec<_>>(),
            }),
            MAX_TICK_COMMIT_BYTES,
        )
        .map_err(|_| integrity("the tick's rng inputs will not encode"))?;
        let mut rng = context
            .start_resolution(&rng_domain, inputs_hash)
            .map_err(|_| integrity("the tick's resolution stream is unavailable"))?;

        // ---- resolution ----
        let mut accepted = Vec::with_capacity(accepted_docs.len());
        let mut deltas = Vec::with_capacity(accepted_docs.len());
        let mut facts: Vec<BoundaryFact> = Vec::new();
        let mut draws: Vec<u32> = Vec::new();
        let mut orders: BTreeMap<StandingOrderKey, StandingOrder> = BTreeMap::new();
        for (request, actor, doc) in accepted_docs {
            let Some(doc) = doc else {
                // An intent this build cannot classify REFUSES; it is never
                // dropped silently (10b §3.8, review addition).
                deltas.push(UnitDelta {
                    unit: actor,
                    kind: IntentKind::Eject,
                    outcome: DeltaOutcome::Refused {
                        reason: "unknown intent kind".to_owned(),
                    },
                });
                continue;
            };
            accepted.push(IntentRef {
                actor,
                unit: doc.unit,
                kind: doc.kind,
                request,
            });
            if !plan.current.contains(&request) {
                deltas.push(UnitDelta {
                    unit: doc.unit,
                    kind: doc.kind,
                    outcome: DeltaOutcome::Refused {
                        reason: "stale station authority".to_owned(),
                    },
                });
                continue;
            }
            match (doc.kind.channel(), doc.value.clone()) {
                (Some(channel), Some(value)) => {
                    let key = StandingOrderKey {
                        unit: doc.unit,
                        channel,
                    };
                    let prior = txn
                        .get(&self.families.orders, &key)
                        .map_err(|error| state_error(&error))?
                        .map(StandingOrder::revision)
                        .unwrap_or(0);
                    let revision = prior.checked_add(1).ok_or_else(|| {
                        err(
                            ActionErrorCode::BoundsExceeded,
                            "that unit has exhausted its standing-order revisions",
                        )
                    })?;
                    let order = StandingOrder::new(
                        doc.unit,
                        channel,
                        value,
                        plan.observed_at.clone(),
                        actor,
                        revision,
                    )
                    .map_err(|_| integrity("that standing order is unconstructible"))?;
                    orders.insert(key, order);
                    deltas.push(UnitDelta {
                        unit: doc.unit,
                        kind: doc.kind,
                        outcome: DeltaOutcome::OrderSet { channel },
                    });
                }
                _ => {
                    let label = CanonicalId::new(doc.kind.token())
                        .map_err(|_| integrity("that intent kind has no rng label"))?;
                    let draw = rng
                        .draw_u32(&label)
                        .map_err(|_| integrity("the tick's resolution stream is exhausted"))?;
                    draws.push(draw);
                    let effective = draw % 2 == 0;
                    deltas.push(UnitDelta {
                        unit: doc.unit,
                        kind: doc.kind,
                        outcome: DeltaOutcome::Resolved { draw, effective },
                    });
                    if effective && facts.len() < MAX_BOUNDARY_FACTS_PER_TICK {
                        facts.push(BoundaryFact {
                            kind: boundary_kind(doc.kind).to_owned(),
                            domain: plan.domain.0,
                            tick: plan.tick,
                            unit: doc.unit,
                        });
                    }
                }
            }
        }

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
        )
        .map_err(|_| {
            err(
                ActionErrorCode::BoundsExceeded,
                "this tick's commit exceeds its record bound",
            )
        })?;
        let key = commit.key();
        let accepted_count = accepted.len();

        for (order_key, order) in orders {
            if txn
                .get(&self.families.orders, &order_key)
                .map_err(|error| state_error(&error))?
                .is_some()
            {
                txn.replace(&self.families.orders, &order_key, order)
            } else {
                txn.insert(&self.families.orders, &order_key, order)
            }
            .map_err(|error| state_error(&error))?;
        }
        if let Some(consumed) = consumed {
            // The commit retains the accepted set, so the spent window record is
            // reaped in the same bundle rather than growing a family a two-second
            // pulse would add 43k records a day to.
            let _ = consumed;
            txn.remove(&self.families.windows, &plan.window)
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
        // The retention ring: drop exactly the commit that falls out of it.
        if let Some(expired) = plan.tick.checked_sub(MAX_RETAINED_TICK_COMMITS as i64) {
            let expired_key = TickCommitKey {
                domain: plan.domain,
                tick: expired,
            };
            if txn
                .get(&self.families.commits, &expired_key)
                .map_err(|error| state_error(&error))?
                .is_some()
            {
                txn.remove(&self.families.commits, &expired_key)
                    .map_err(|error| state_error(&error))?;
            }
        }
        // The next collection interval, so the following tick's submissions have
        // somewhere to go without a second authority opening it.
        let next_tick = plan.tick.saturating_add(1);
        if let (Some(next_id), ClockRef::DomainTick { generation, .. }) = (
            tick_window_id(&owner, plan.domain, next_tick),
            &plan.observed_at.clock,
        ) && txn
            .get(&self.families.windows, &next_id)
            .map_err(|error| state_error(&error))?
            .is_none()
        {
            let window = open_window(next_id, &plan.observed_at.clock, next_tick, *generation)
                .map_err(|_| integrity("the next tick's window is unconstructible"))?;
            txn.insert(
                &self.families.windows,
                &next_id,
                ActionWindowRecord::open(window),
            )
            .map_err(|error| state_error(&error))?;
        }

        // ---- T2: typed boundary facts leave the bundle; nothing economic runs ----
        let mut outbox = Vec::with_capacity(facts.len());
        for (ordinal, fact) in facts.iter().enumerate() {
            let ordinal = u16::try_from(ordinal)
                .map_err(|_| integrity("too many boundary facts for one tick"))?;
            outbox.push(
                DurableOutboxIntent::new(
                    ordinal,
                    OwnerId::new(BOUNDARY_OUTBOX_OWNER)
                        .map_err(|_| integrity("the boundary owner is invalid"))?,
                    CanonicalId::new(BOUNDARY_OUTBOX_ADAPTER)
                        .map_err(|_| integrity("the boundary adapter is invalid"))?,
                    json!({
                        "id": fact.stable_id(),
                        "kind": fact.kind,
                        "domain": fact.domain,
                        "tick": fact.tick,
                        "unit": fact.unit,
                    }),
                )
                .map_err(|_| integrity("a boundary fact will not encode"))?,
            );
        }

        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({
                "tick": plan.tick,
                "accepted": accepted_count,
                "facts": facts.len(),
            }),
        )
        .map_err(|_| integrity("the tick outcome is invalid"))?;
        let prepared = rng
            .into_prepared(
                Vec::new(),
                RulesValue::new(self.outcome.clone(), json!({}))
                    .map_err(|_| integrity("the tick modifiers are invalid"))?,
                RulesValue::new(self.outcome.clone(), json!({}))
                    .map_err(|_| integrity("the tick resolution outcome is invalid"))?,
            )
            .map_err(|_| integrity("the tick's resolution will not close"))?;
        Ok(mush_core::AuthoritativeActionOutput::new(outcome)
            .with_resolution(prepared)
            .with_durable_outbox(outbox))
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
        let owner = self.families.schedules.owner().clone();
        let domain = DomainId(doc.domain);
        let id = tick_schedule_id(&owner, domain)
            .ok_or_else(|| integrity("that domain's pulse cursor identity is unconstructible"))?;
        let prior = txn
            .get(&self.families.schedules, &id)
            .map_err(|error| state_error(&error))?
            .is_some();
        if !doc.arm {
            if prior {
                txn.remove(&self.families.schedules, &id)
                    .map_err(|error| state_error(&error))?;
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
        let schedule = pulse_cursor(&owner, domain, &clock, doc.from_tick)
            .map_err(|_| integrity("that domain's pulse cursor is unconstructible"))?;
        if prior {
            txn.replace(&self.families.schedules, &id, schedule)
        } else {
            txn.insert(&self.families.schedules, &id, schedule)
        }
        .map_err(|error| state_error(&error))?;
        // The first collection interval, so an intent submitted before the first
        // occurrence has a window to join.
        let window_id = tick_window_id(&owner, domain, doc.from_tick)
            .ok_or_else(|| integrity("that tick's window identity is unconstructible"))?;
        if txn
            .get(&self.families.windows, &window_id)
            .map_err(|error| state_error(&error))?
            .is_none()
        {
            let window = open_window(window_id, &clock, doc.from_tick, doc.generation)
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
/// re-checks every accepted intent's station witness against
/// `World::channel_grant_is_current` — the one place in the tick lane that has
/// a `&World` at all.
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
        let ClockRef::DomainTick { id, .. } = advance.observed_at().clock.clone() else {
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
        let window = tick_window_id(&owner, domain, tick)
            .ok_or_else(|| integrity("that tick's window identity is unconstructible"))?;
        let host = engine
            .world
            .module_state::<SpatialState>()
            .and_then(|state| state.domains.get(&domain).map(|record| record.host))
            .ok_or_else(|| err(ActionErrorCode::Stale, "that domain is gone"))?;

        // Re-check every accepted intent's pinned station witness. A grant that
        // has since moved leaves the intent in the frozen set and REFUSES it in
        // the commit, so the tick still resolves and the refusal is recorded.
        let mut still_current: Vec<RequestUid> = Vec::new();
        if let Ok(Some(record)) = engine.action_state().get(&self.families.windows, &window) {
            for intent in record.intents() {
                let Ok(doc) = TacticalIntentProvider::document(intent.invocation()) else {
                    continue;
                };
                let Some(channel) = mush_core::ChannelId::new(&doc.channel) else {
                    continue;
                };
                if engine.world.channel_grant_is_current(
                    intent.invocation().actor(),
                    doc.unit,
                    NS,
                    &doc.station,
                    doc.epoch,
                    &channel,
                ) {
                    still_current.push(intent.invocation().request());
                }
            }
        }
        still_current.sort();
        still_current.dedup();

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
                "current": still_current,
            }),
            guards: guards
                .finish()
                .map_err(|_| integrity("the tick guards are invalid"))?,
        })
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

/// The `tactical_intent` action, by the same string lookup — this is how
/// `mush-tactics`' combat verbs reach the window without a `mush-sim` edge.
#[must_use]
pub fn intent_action_ref(reg: &mush_core::Registry) -> Option<DefinitionRef> {
    reg.definitions()?
        .current(&mush_core::DefinitionKey {
            owner: OwnerId::new(NS).ok()?,
            kind: DefinitionKind::new("action").ok()?,
            id: DefinitionId::new(INTENT_ACTION_ID).ok()?,
        })
        .cloned()
}

/// What one maintenance pass decided, so a command can single-step it and a
/// test can assert on it without reading a log.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PulseMaintenance {
    /// Domains whose live activation request was applied this pass.
    pub applied: Vec<DomainId>,
    /// Domains whose cursor was armed this pass.
    pub armed: Vec<DomainId>,
    /// Domains that were stood back down to `Passive` this pass.
    pub stood_down: Vec<DomainId>,
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
    let period = ctx
        .w
        .module_state::<PulseConfig>()
        .copied()
        .unwrap_or_default()
        .period_ms(false);
    let plan = {
        let Some(state) = ctx.w.module_state::<SpatialState>() else {
            return PulseMaintenance::default();
        };
        let book = ctx
            .w
            .module_state::<crate::domain::ActivationBook>()
            .cloned()
            .unwrap_or_default();
        plan_maintenance(state, &book, &now, period, only)
    };
    // ---- the applying authority: one `sim` document per applied domain ----
    let mut applied_records = Vec::new();
    let mut stood_down_records = Vec::new();
    if let Some(state) = ctx.w.module_state_mut::<SpatialState>() {
        for id in &plan.applied {
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
    // ---- one arm/disarm per fire ----
    let Some(action) = arm_action_ref(ctx.reg) else {
        return plan;
    };
    let mut submitted = None;
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
        submitted = Some((*id, generation, from_tick, true));
        break;
    }
    if submitted.is_none()
        && let Some(id) = plan.stood_down.first()
    {
        submitted = Some((*id, 1, 0, false));
    }
    let Some((id, generation, from_tick, arm)) = submitted else {
        return plan;
    };
    let doc = ArmDoc {
        domain: id.0,
        generation,
        from_tick,
        arm,
    };
    let (Ok(document), Ok(key)) = (
        serde_json::to_string(&doc),
        mush_core::IdempotencyKeyText::parse_canonical(format!(
            "sim.pulse.{}.{}.{}.{}",
            id.0,
            generation,
            from_tick.max(0),
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

/// Decide what the maintenance pass should do, without mutating anything.
///
/// Pure over `(state, book, now)`, so the world-side tick and `@domain/tick`
/// share one decision and cannot disagree. Bounded on domains **visited**
/// (D-LBK-5).
#[must_use]
pub fn plan_maintenance(
    state: &SpatialState,
    book: &crate::domain::ActivationBook,
    now: &ClockStamp,
    period_ms: u32,
    only: Option<DomainId>,
) -> PulseMaintenance {
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
        let live = book.is_promoted(*id);
        match (&record.clock, live, record.status == DomainStatus::Active) {
            // A live request on an active, passive domain: apply it.
            (DomainClock::Passive, true, true) => {
                let units = state
                    .derived
                    .local_objects
                    .get(id)
                    .map_or(0, std::collections::BTreeSet::len);
                match check_domain_capacity(units) {
                    Ok(_) => {
                        let mut candidate = record.clone();
                        match apply_pulse(&mut candidate, period_ms, now) {
                            Ok(_) => plan.applied.push(*id),
                            Err(reason) => plan.refused.push(reason),
                        }
                    }
                    Err(reason) => plan.refused.push(reason),
                }
            }
            // Pulsing but no longer eligible: stand it back down. State is
            // preserved both ways; only the cadence changes (Q15A).
            (DomainClock::Pulse { .. }, false, _) | (DomainClock::Pulse { .. }, _, false) => {
                plan.stood_down.push(*id);
            }
            // Pulsing and eligible: make sure the cursor is armed. Arming is
            // idempotent under its deterministic key, so re-arming a live cursor
            // is a HIT that changes nothing.
            (DomainClock::Pulse { .. }, true, true) => plan.armed.push(*id),
            _ => {}
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
/// The encoded shape is exactly what `DomainTickProvider` writes at saturation:
/// one `StandingOrder` post-image per unit, the consumed window's removal, the
/// `DomainTickCommit` with one delta per unit and its recorded draws, the ring's
/// expiry removal, the next window's post-image, and one boundary fact (hence
/// one outbox payload) per unit up to `MAX_BOUNDARY_FACTS_PER_TICK`.
///
/// # Panics
///
/// Never: every fallible step returns `None` instead.
#[must_use]
pub fn measure_tick_bundle(units: usize) -> Option<TickBundleMeasurement> {
    let owner = OwnerId::new(NS).ok()?;
    let domain = DomainId(7);
    let tick = 4_096_i64;
    let clock = pulse_clock(&owner, domain, 1)?;
    let stamp = ClockStamp {
        clock: clock.clone(),
        at: tick,
    };
    let window = tick_window_id(&owner, domain, tick)?;
    let mut accepted = Vec::with_capacity(units);
    let mut deltas = Vec::with_capacity(units);
    let mut facts = Vec::new();
    let mut draws = Vec::with_capacity(units);
    let mut orders = Vec::with_capacity(units);
    for index in 0..units {
        let unit = ObjId(1_000_000 + u32::try_from(index).ok()?);
        let request = RequestUid::from_bytes([(index % 251) as u8; 16]);
        accepted.push(IntentRef {
            actor: unit,
            unit,
            kind: IntentKind::Heading,
            request,
        });
        deltas.push(UnitDelta {
            unit,
            kind: IntentKind::Heading,
            outcome: DeltaOutcome::OrderSet {
                channel: OrderChannel::Heading,
            },
        });
        draws.push(0xDEAD_BEEF_u32.wrapping_add(index as u32));
        orders.push(
            StandingOrder::new(
                unit,
                OrderChannel::Heading,
                OrderValue::Vector {
                    x: -999,
                    y: 999,
                    z: -999,
                },
                stamp.clone(),
                unit,
                u64::from(u32::MAX),
            )
            .ok()?,
        );
        if facts.len() < MAX_BOUNDARY_FACTS_PER_TICK {
            facts.push(BoundaryFact {
                kind: "tactics.sortie_started".to_owned(),
                domain: domain.0,
                tick,
                unit,
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
        draws,
    )
    .ok()?;
    let next_window = ActionWindowRecord::open(
        open_window(
            tick_window_id(&owner, domain, tick + 1)?,
            &clock,
            tick + 1,
            1,
        )
        .ok()?,
    );
    // Post-images: N orders + the commit + the next window. Removals: the spent
    // window, the expired ring entry.
    let replay_writes = units + 1 + 1 + 2;
    let mut payloads = Vec::with_capacity(facts.len());
    for fact in &facts {
        payloads.push(json!({
            "id": fact.stable_id(),
            "kind": fact.kind,
            "domain": fact.domain,
            "tick": fact.tick,
            "unit": fact.unit,
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
    let bundle = json!({
        "orders": orders,
        "commit": commit,
        "next_window": next_window,
        "outbox": payloads,
    });
    let bundle_bytes =
        canonical_json_bytes_with_limit(&bundle, mush_core::MAX_AUTHORITATIVE_BUNDLE_BYTES)
            .ok()?
            .len();
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

/// The four definition descriptors the pulse contributes.
///
/// `mush-sim` may register **one** `definition_validator` per kind, and
/// `handoff.rs` already owns the `"action"`/`"outcome"` ones — so the pulse
/// contributes its descriptors there rather than registering a second validator,
/// exactly as cut 7's sensor ingress does inside `mush-tactics`.
pub(crate) struct TickDescriptors {
    pub(crate) tick_outcome: DefinitionDescriptor,
    pub(crate) intent_outcome: DefinitionDescriptor,
    pub(crate) arm_outcome: DefinitionDescriptor,
    pub(crate) tick_action: DefinitionDescriptor,
    pub(crate) intent_action: DefinitionDescriptor,
    pub(crate) arm_action: DefinitionDescriptor,
}

/// The registered form of [`TickDescriptors`].
pub(crate) struct TickDefinitions {
    pub(crate) tick_outcome: DefinitionRef,
    pub(crate) intent_outcome: DefinitionRef,
    pub(crate) arm_outcome: DefinitionRef,
    pub(crate) tick_action: DefinitionRef,
    pub(crate) intent_action: DefinitionRef,
    pub(crate) arm_action: DefinitionRef,
}

/// Build the pulse's descriptors, in registration order.
pub(crate) fn definition_descriptors() -> Result<TickDescriptors, ModError> {
    let tick_outcome = tick_outcome_descriptor()?;
    let intent_outcome = intent_outcome_descriptor()?;
    let arm_outcome = arm_outcome_descriptor()?;
    let tick_action = tick_action_descriptor(&definition_ref_of(&tick_outcome)?)?;
    let intent_action = intent_action_descriptor(&definition_ref_of(&intent_outcome)?)?;
    let arm_action = arm_action_descriptor(&definition_ref_of(&arm_outcome)?)?;
    Ok(TickDescriptors {
        tick_outcome,
        intent_outcome,
        arm_outcome,
        tick_action,
        intent_action,
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

/// Register the pulse: four typed families, the window guard validator, the one
/// domain clock provider, two action providers and **one** schedule source.
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
    let families = TickFamilies {
        windows,
        schedules,
        commits,
        orders,
    };

    // **B1 — the one owner-scoped production domain clock provider.**
    reg.domain_clock_provider(SimDomainClockProvider)?;
    // The READ handles a staff command pages. Owner-fenced by the kernel, so
    // carrying them in world state grants nothing another module could use.
    reg.state(TickHandles {
        commits: families.commits.clone(),
        orders: families.orders.clone(),
        windows: families.windows.clone(),
    });

    reg.action_provider(
        definitions.intent_action.clone(),
        TacticalIntentProvider {
            action: definitions.intent_action.clone(),
            outcome: definitions.intent_outcome,
            families: families.clone(),
        },
    )?;
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

/// The byte-identical refusal a viewer receives for a contact they do not hold
/// and for one that never existed (§4 rule 4).
#[must_use]
pub const fn no_such_contact() -> &'static str {
    NO_SUCH_CONTACT
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
    #[test]
    fn e7c8_b4_a_commit_that_omits_an_accepted_intent_fails_its_state_hash() {
        let owner = OwnerId::new(NS).expect("static owner");
        let clock = pulse_clock(&owner, DomainId(2), 1).expect("a domain clock");
        let window = tick_window_id(&owner, DomainId(2), 5).expect("a window id");
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
            kind: IntentKind::Fire,
            outcome: DeltaOutcome::Resolved {
                draw: 4,
                effective: true,
            },
        }];
        let full = DomainTickCommit::new(
            DomainId(2),
            5,
            clock.clone(),
            window,
            accepted.clone(),
            deltas.clone(),
            Vec::new(),
            inputs,
            vec![4],
        )
        .expect("a coherent commit");
        let short = DomainTickCommit::new(
            DomainId(2),
            5,
            clock,
            window,
            accepted[..1].to_vec(),
            deltas,
            Vec::new(),
            inputs,
            vec![4],
        )
        .expect("a coherent commit");
        assert_ne!(full.state_hash(), short.state_hash());
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

    /// **The measured frontier, stated honestly.** No K8 cap ever binds a tick:
    /// the encoded bundle stays under 60 % of all three at **every** N the window
    /// can admit, and what actually stops the count is
    /// `MAX_WINDOW_CAPACITY`/`MAX_INTENTS_PER_TICK` 256 — one intent per
    /// `(actor, truth_subject)` slot. So `MAX_UNITS_PER_DOMAIN` 64 is confirmed
    /// as a **latency and offer-budget policy figure**, four times below the
    /// window's own ceiling and sixteen times below the replay-write cap.
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
        // One past the window capacity is unencodable at all — the commit record
        // refuses more deltas than the window could ever have admitted.
        assert!(measure_tick_bundle(MAX_INTENTS_PER_TICK as usize + 1).is_none());
        // And the pinned cap sits well inside that.
        assert!(MAX_UNITS_PER_DOMAIN * 4 <= MAX_INTENTS_PER_TICK as usize);
    }
}
