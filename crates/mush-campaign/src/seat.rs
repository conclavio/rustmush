//! E7 cut 10 B2/B3/B7 — proxy seats, crew and musters (10b §3.10).
//!
//! **A proxy seat is a consent grant, not an E4 contract** (Q10D; doc 10 R6 as
//! amended). The governing fact is that **an OOC proxy player is not an in-world
//! principal** (C1A/R3/T13): no account principal, no contract naming the account
//! or the absent character, and no `ConsentSurface` stuffed with seat fields. So
//! the offer's `consent_hash` is `mush_economy::consent_digest` under
//! `"campaign.seat.v1"` — cut 9's shared discipline, never a forked hasher — and
//! nothing here reaches the contract engine at all.
//!
//! **The records are workflow, and only workflow.** `SeatOffer`, `SeatGrant`,
//! `CrewComplement` and `MusterReport` are bounded whole documents behind strict
//! `TryFrom` wires (§4 rule 1) at deterministic keys (rule 3), and a probe
//! asserts **no legality check reads a `SeatGrant`**: the live authority is the
//! billet, the narrowed `ChannelGrant` and K6's lease, all of them `mush-sim`'s.
//! Cut 10 persists **no lease-like record of its own**.
//!
//! **Attribution** (Q19A/T9/R7). A grant carries an opaque [`DriverToken`] — per
//! grant, derived from the seat's own coordinates and **never from the account**,
//! so the same player accepting two seats produces two tokens no projection
//! correlates. The token → account row lives **outside the world** behind
//! `mush_core::SeatAudit`, is implemented at the server, and is reachable only by
//! one staff verb.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{CanonicalHash, ClockStamp, ObjId};
use mush_economy::{CONSENT_DOMAIN_CAMPAIGN_SEAT, consent_digest, principal_key};
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::ids::OperationId;
use crate::record::{AudienceSpec, UnitRef};

// ---- §7 constants (cited where they already exist, defined where new) ----

/// The largest number of live seat offers one operation may carry (§7).
pub const MAX_SEAT_OFFERS_PER_OPERATION: usize = 32;
/// How long an unaccepted seat offer stands, in seconds (§7's `SEAT_OFFER_TTL`,
/// 7 days).
pub const SEAT_OFFER_TTL_SECS: i64 = 7 * 24 * 60 * 60;
/// The largest number of named crew one complement may carry (§7).
pub const MAX_CREW_NAMED: usize = 16;
/// The largest number of channels one seat scope may name. A seat's set is a
/// subset of its station's, which `mush_sim::MAX_GRANTED_CHANNELS` already
/// bounds; this is the wire's own bound so a stored document refuses on load.
pub const MAX_SEAT_CHANNELS: usize = 32;
/// The largest number of slots one muster report enumerates (D-LBK-5: the report
/// is bounded on records **visited**, and an over-cap report refuses on load).
pub const MAX_MUSTER_SLOTS: usize = 64;
/// The largest number of live grants one operation may carry — one per offer.
pub const MAX_SEAT_GRANTS_PER_OPERATION: usize = MAX_SEAT_OFFERS_PER_OPERATION;
/// The largest number of bytes in a free-text seat label (mission order, ROE).
pub const MAX_SEAT_LABEL_BYTES: usize = 240;

// ---- ids and keys ----

/// One seat offer, unique within its operation.
#[derive(
    Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct SeatOfferId(pub u64);

impl std::fmt::Display for SeatOfferId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "seat{}", self.0)
    }
}

impl SeatOfferId {
    /// Parse a rendered id, with or without its `seat` sigil.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let raw = text.trim();
        let raw = raw.strip_prefix("seat").unwrap_or(raw);
        let raw = raw.strip_prefix('#').unwrap_or(raw);
        let n: u64 = raw.parse().ok()?;
        (n != 0).then_some(Self(n))
    }
}

/// The document key one seat offer lives at (rule 3: deterministic).
#[must_use]
pub fn seat_offer_key(operation: OperationId, id: SeatOfferId) -> SmolStr {
    SmolStr::new(format!("seat_offer:{}:{}", operation.0, id.0))
}

/// The document key one seat grant lives at.
#[must_use]
pub fn seat_grant_key(operation: OperationId, id: SeatOfferId) -> SmolStr {
    SmolStr::new(format!("seat_grant:{}:{}", operation.0, id.0))
}

/// The document key one unit's crew complement lives at.
#[must_use]
pub fn crew_key(unit: UnitRef) -> SmolStr {
    SmolStr::new(format!("crew:{}", unit.0.0))
}

/// The document key one muster report lives at.
#[must_use]
pub fn muster_key(operation: OperationId, cycle: u64) -> SmolStr {
    SmolStr::new(format!("muster:{}:{cycle}", operation.0))
}

// ---- the records ----

/// Why a seat record refused. Every variant is rendered by the command, so two
/// refusals a viewer may not distinguish stay byte-identical there.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SeatRecordError {
    /// A bounded set is over its cap. **Refuses on load, never truncates.**
    #[error("{what} carries {actual}, over the cap of {max}")]
    TooMany {
        /// Which set.
        what: &'static str,
        /// How many it carried.
        actual: usize,
        /// The cap.
        max: usize,
    },
    /// A free-text label is over its byte cap.
    #[error("{what} is {actual} bytes, over the cap of {max}")]
    TooLong {
        /// Which label.
        what: &'static str,
        /// How many bytes.
        actual: usize,
        /// The cap.
        max: usize,
    },
    /// A required field is empty.
    #[error("{0} must not be empty")]
    Empty(&'static str),
    /// The record's own id disagrees with something it carries.
    #[error("{0}")]
    Inconsistent(&'static str),
}

fn bounded_label(what: &'static str, text: &str) -> Result<(), SeatRecordError> {
    if text.len() > MAX_SEAT_LABEL_BYTES {
        return Err(SeatRecordError::TooLong {
            what,
            actual: text.len(),
            max: MAX_SEAT_LABEL_BYTES,
        });
    }
    Ok(())
}

/// What a seat may reach: one station on one unit, a **subset** of that station's
/// control channels, and the orders and area that bound it.
///
/// `channels` is the half T11 enforces **mechanically**: it is handed to
/// `mush_sim::request_seat_activation`, stored on the billet as
/// `StationAssignment.granted`, and intersected with `StationSpec.channels`
/// inside the ONE production `ChannelGrant` projection. An action whose
/// `required_channels` fall outside it never projects an offer at all.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "SeatScopeWire", deny_unknown_fields)]
pub struct SeatScope {
    /// The operated unit.
    pub unit: UnitRef,
    /// Its station.
    pub station: SmolStr,
    /// The control channels the seat grants — never wider than the station's own.
    pub channels: BTreeSet<SmolStr>,
    /// The mission order the seat is bound to (free text, bounded).
    pub mission_order: SmolStr,
    /// The rules of engagement (free text, bounded).
    pub roe: SmolStr,
    /// The area the seat may operate in. `None` is "wherever the operation is",
    /// which is the operation's own bound and never wider.
    #[serde(default)]
    pub permitted_area: Option<UnitRef>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SeatScopeWire {
    unit: UnitRef,
    station: SmolStr,
    channels: BTreeSet<SmolStr>,
    mission_order: SmolStr,
    roe: SmolStr,
    #[serde(default)]
    permitted_area: Option<UnitRef>,
}

impl TryFrom<SeatScopeWire> for SeatScope {
    type Error = SeatRecordError;
    fn try_from(wire: SeatScopeWire) -> Result<Self, Self::Error> {
        let scope = SeatScope {
            unit: wire.unit,
            station: wire.station,
            channels: wire.channels,
            mission_order: wire.mission_order,
            roe: wire.roe,
            permitted_area: wire.permitted_area,
        };
        scope.check()?;
        Ok(scope)
    }
}

impl SeatScope {
    /// Re-run every constructor invariant (§4 rule 1). The wire calls it, so a
    /// stored set over a cap **refuses on load rather than truncating**.
    pub fn check(&self) -> Result<(), SeatRecordError> {
        if self.station.is_empty() {
            return Err(SeatRecordError::Empty("a seat scope's station"));
        }
        if self.channels.is_empty() {
            return Err(SeatRecordError::Empty("a seat scope's channel set"));
        }
        if self.channels.len() > MAX_SEAT_CHANNELS {
            return Err(SeatRecordError::TooMany {
                what: "a seat scope's channel set",
                actual: self.channels.len(),
                max: MAX_SEAT_CHANNELS,
            });
        }
        bounded_label("a seat's mission order", &self.mission_order)?;
        bounded_label("a seat's rules of engagement", &self.roe)?;
        bounded_label("a seat scope's station", &self.station)?;
        Ok(())
    }
}

/// Who may take a seat. Deliberately **not** an account predicate: matchmaking
/// standing is a crew-call eligibility policy input only (R6) and never an
/// in-world principal, so eligibility names an operation side and, optionally, a
/// certification label the crew call publishes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EligibilitySpec {
    /// The side of the operation the seat fights on.
    #[serde(default)]
    pub side: SmolStr,
    /// A published certification label, or empty for none.
    #[serde(default)]
    pub certification: SmolStr,
}

/// What ends a seat, stated up front so a driver consents to its own revocation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevocationTerms {
    /// Whether the offering commander may revoke at will (always true today; the
    /// field exists so a future negotiated seat can say otherwise and the
    /// consent hash moves visibly when it does).
    #[serde(default)]
    pub by_commander: bool,
    /// Whether the operation ending revokes the seat.
    #[serde(default)]
    pub on_operation_end: bool,
}

/// The controller the seat returns to at a safe boundary — the `mush-sim`
/// `sim.station_fallback` holder, named here so the K6 gate's leg (1) knows which
/// occupant is "the grant's own fallback" rather than a live human operator.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FallbackSpec {
    /// The declared fallback controller (an AI-driven NPC, normally the same
    /// object the seat drives).
    pub holder: UnitRef,
}

/// **A proxy seat offer** — frozen at acceptance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "SeatOfferWire", deny_unknown_fields)]
pub struct SeatOffer {
    /// Unique within the operation.
    pub id: SeatOfferId,
    /// The operation it belongs to.
    pub operation: OperationId,
    /// Who may see it at all (never wider than the operation's own audience).
    pub audience: AudienceSpec,
    /// Who may take it.
    pub eligibility: EligibilitySpec,
    /// When it lapses (world IC clock).
    pub expires: ClockStamp,
    /// What it reaches.
    pub scope: SeatScope,
    /// What ends it.
    pub revocation: RevocationTerms,
    /// Where the seat goes back to.
    pub fallback: FallbackSpec,
    /// `consent_digest("campaign.seat.v1", ..)` over the frozen surface.
    pub consent_hash: [u8; 32],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SeatOfferWire {
    id: SeatOfferId,
    operation: OperationId,
    audience: AudienceSpec,
    eligibility: EligibilitySpec,
    expires: ClockStamp,
    scope: SeatScope,
    revocation: RevocationTerms,
    fallback: FallbackSpec,
    consent_hash: [u8; 32],
}

impl TryFrom<SeatOfferWire> for SeatOffer {
    type Error = SeatRecordError;
    fn try_from(wire: SeatOfferWire) -> Result<Self, Self::Error> {
        let offer = SeatOffer {
            id: wire.id,
            operation: wire.operation,
            audience: wire.audience,
            eligibility: wire.eligibility,
            expires: wire.expires,
            scope: wire.scope,
            revocation: wire.revocation,
            fallback: wire.fallback,
            consent_hash: wire.consent_hash,
        };
        offer.check()?;
        Ok(offer)
    }
}

/// The **field-ordered canonical form** of a seat offer's frozen surface, with
/// `domain` first — the discipline `consent_digest` checks, so a seat hash can
/// never be replayed as an operation hash or a contract hash.
#[derive(Serialize)]
struct CanonicalSeatOffer<'a> {
    domain: &'a str,
    operation: u64,
    offer: u64,
    audience: Vec<&'a PrincipalId>,
    eligibility: &'a EligibilitySpec,
    expires: &'a ClockStamp,
    scope: &'a SeatScope,
    revocation: &'a RevocationTerms,
    fallback: &'a FallbackSpec,
}

/// The one seat-offer hash. Any principal vector sorts by `principal_key`
/// (cut 9's rule, verbatim); every other vector's order **is** consented content.
#[must_use]
pub fn seat_consent_hash(offer: &SeatOffer) -> [u8; 32] {
    let mut audience: Vec<&PrincipalId> = offer.audience.members.iter().collect();
    audience.sort_by_key(|p| principal_key(p));
    consent_digest(
        CONSENT_DOMAIN_CAMPAIGN_SEAT,
        &CanonicalSeatOffer {
            domain: CONSENT_DOMAIN_CAMPAIGN_SEAT,
            operation: offer.operation.0,
            offer: offer.id.0,
            audience,
            eligibility: &offer.eligibility,
            expires: &offer.expires,
            scope: &offer.scope,
            revocation: &offer.revocation,
            fallback: &offer.fallback,
        },
    )
}

impl SeatOffer {
    /// Re-run every constructor invariant (§4 rule 1).
    pub fn check(&self) -> Result<(), SeatRecordError> {
        if self.id.0 == 0 {
            return Err(SeatRecordError::Inconsistent("a seat offer id is zero"));
        }
        if self.audience.members.len() > mush_economy::MAX_AUDIENCE {
            return Err(SeatRecordError::TooMany {
                what: "a seat offer's audience",
                actual: self.audience.members.len(),
                max: mush_economy::MAX_AUDIENCE,
            });
        }
        bounded_label("a seat offer's side", &self.eligibility.side)?;
        bounded_label(
            "a seat offer's certification",
            &self.eligibility.certification,
        )?;
        self.scope.check()?;
        Ok(())
    }

    /// Whether the offer has lapsed at `now` (world IC clock, on one generation:
    /// two stamps are only ever compared on the same clock).
    #[must_use]
    pub fn is_expired(&self, now: &ClockStamp) -> bool {
        now.clock == self.expires.clock && now.at >= self.expires.at
    }

    /// Whether the frozen surface still hashes to what was consented to.
    #[must_use]
    pub fn consent_intact(&self) -> bool {
        seat_consent_hash(self) == self.consent_hash
    }
}

/// The opaque per-grant decision-driver token (Q19A/T9/R7).
///
/// Derived from the seat's own coordinates — operation, offer, the driven NPC and
/// the acceptance instant — and **never from the account**, so a projection can
/// carry it safely and two seats taken by one player produce two tokens nothing
/// correlates. The token → account row lives outside the world behind
/// `mush_core::SeatAudit` and is reachable only by one staff verb.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DriverToken(pub SmolStr);

impl std::fmt::Display for DriverToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl DriverToken {
    /// Mint the token for one grant. Deterministic (no RNG anywhere in E7's
    /// non-tick paths — `provider_rng_policy`), and account-free by construction.
    #[must_use]
    pub fn mint(
        operation: OperationId,
        offer: SeatOfferId,
        actor: UnitRef,
        accepted_at: &ClockStamp,
    ) -> DriverToken {
        let body = serde_json::json!({
            "operation": operation.0,
            "offer": offer.0,
            "actor": actor.0.0,
            "at": accepted_at.at,
        });
        let hash = CanonicalHash::for_canonical_value("campaign.seat.driver.v1", 1, &body, 1024)
            .map(|h| h.to_hex())
            .unwrap_or_else(|_| format!("{}.{}", operation.0, offer.0));
        DriverToken(SmolStr::new(&hash[..hash.len().min(32)]))
    }
}

/// A grant's lifecycle. `Active { epoch }` is recorded **only** from what
/// `checked_apply_handoff` produced — observed by a pull over the lease, never
/// asserted by the command that requested it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum GrantState {
    /// Accepted, seizing nothing. **Accepting an offer does not take focus.**
    Dormant,
    /// The lease landed; `epoch` is the unit's control generation the seat was
    /// observed live under. A later bump makes it stale, which is the whole
    /// revocation mechanism.
    Active {
        /// The unit's control generation at activation.
        epoch: u64,
    },
    /// Revoked, by the commander or by an unclean loss. **Terminal.**
    Revoked,
    /// The offer lapsed, or its operation ended, before it was ever activated.
    /// **Terminal.**
    Expired,
}

impl GrantState {
    /// Whether the grant can never change again.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(self, GrantState::Revoked | GrantState::Expired)
    }
}

/// **The accepted seat.** Naming neither an account nor the absent character
/// (§11.2): the only in-world identity here is `actor`, the NPC the seat drives.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "SeatGrantWire", deny_unknown_fields)]
pub struct SeatGrant {
    /// The operation the offer belongs to.
    pub operation: OperationId,
    /// The offer it was accepted from.
    pub offer: SeatOfferId,
    /// The in-world actor the seat drives — a named NPC.
    pub actor: UnitRef,
    /// The opaque driver token.
    pub driver: DriverToken,
    /// When it was accepted (world IC clock).
    pub accepted_at: ClockStamp,
    /// The offer's consent hash, copied at acceptance: the grant is frozen to
    /// exactly the surface that was consented to.
    pub consent_hash: [u8; 32],
    /// Where it is in its life.
    pub state: GrantState,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SeatGrantWire {
    operation: OperationId,
    offer: SeatOfferId,
    actor: UnitRef,
    driver: DriverToken,
    accepted_at: ClockStamp,
    consent_hash: [u8; 32],
    state: GrantState,
}

impl TryFrom<SeatGrantWire> for SeatGrant {
    type Error = SeatRecordError;
    fn try_from(wire: SeatGrantWire) -> Result<Self, Self::Error> {
        let grant = SeatGrant {
            operation: wire.operation,
            offer: wire.offer,
            actor: wire.actor,
            driver: wire.driver,
            accepted_at: wire.accepted_at,
            consent_hash: wire.consent_hash,
            state: wire.state,
        };
        grant.check()?;
        Ok(grant)
    }
}

impl SeatGrant {
    /// Re-run every constructor invariant (§4 rule 1).
    pub fn check(&self) -> Result<(), SeatRecordError> {
        if self.offer.0 == 0 {
            return Err(SeatRecordError::Inconsistent(
                "a seat grant's offer is zero",
            ));
        }
        if self.driver.0.is_empty() {
            return Err(SeatRecordError::Empty("a seat grant's driver token"));
        }
        if self.driver.0.len() > 64 {
            return Err(SeatRecordError::TooLong {
                what: "a seat grant's driver token",
                actual: self.driver.0.len(),
                max: 64,
            });
        }
        Ok(())
    }
}

/// **The aggregate complement** (C11A/C13A/C14A/C15A). The named half of a crew
/// **is** cut 5's billets (`HostAssignments.seats` + `Derived::seats_by_actor`) —
/// there is no second topology and no new roster record. This document carries
/// the aggregate only: readiness, capacity, morale, and the readiness
/// obligation's id.
///
/// A crew is **not an org scope** (R2: the org tree never mirrors the order of
/// battle) and **not a party** (Q21A: `mush_party::MAX_PARTY_SIZE` is 8 against
/// `MAX_OPERATION_PARTICIPANTS` 128, and `mush-party` is consumed unchanged and
/// never extended).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "CrewComplementWire", deny_unknown_fields)]
pub struct CrewComplement {
    /// The crewed unit.
    pub unit: UnitRef,
    /// Dramatically relevant positions: station → the named NPC that fills it.
    /// A projection of the billets, kept for the aggregate's own arithmetic.
    pub named: BTreeMap<SmolStr, UnitRef>,
    /// Readiness, 0–100.
    pub readiness: u8,
    /// The routine watch the aggregate stands for.
    pub capacity: u32,
    /// Morale, −100…100.
    pub morale: i16,
    /// The periodic readiness `Obligation`'s id — the **only** thing campaign
    /// persists about it (the economy carve-out owns the obligation itself).
    #[serde(default)]
    pub obligation: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CrewComplementWire {
    unit: UnitRef,
    named: BTreeMap<SmolStr, UnitRef>,
    readiness: u8,
    capacity: u32,
    morale: i16,
    #[serde(default)]
    obligation: Option<u64>,
}

impl TryFrom<CrewComplementWire> for CrewComplement {
    type Error = SeatRecordError;
    fn try_from(wire: CrewComplementWire) -> Result<Self, Self::Error> {
        let crew = CrewComplement {
            unit: wire.unit,
            named: wire.named,
            readiness: wire.readiness,
            capacity: wire.capacity,
            morale: wire.morale,
            obligation: wire.obligation,
        };
        crew.check()?;
        Ok(crew)
    }
}

impl CrewComplement {
    /// A fresh complement for a unit.
    #[must_use]
    pub fn new(unit: UnitRef) -> CrewComplement {
        CrewComplement {
            unit,
            named: BTreeMap::new(),
            readiness: 100,
            capacity: 0,
            morale: 0,
            obligation: None,
        }
    }

    /// Re-run every constructor invariant (§4 rule 1).
    pub fn check(&self) -> Result<(), SeatRecordError> {
        if self.named.len() > MAX_CREW_NAMED {
            return Err(SeatRecordError::TooMany {
                what: "a crew complement's named positions",
                actual: self.named.len(),
                max: MAX_CREW_NAMED,
            });
        }
        if self.readiness > 100 {
            return Err(SeatRecordError::Inconsistent(
                "a crew complement's readiness is over 100",
            ));
        }
        for station in self.named.keys() {
            bounded_label("a crew position", station)?;
        }
        Ok(())
    }
}

/// What one muster slot resolved to (T14). **Vacancy is a resolution**, not an
/// error: missing players reduce discretion and resilience but do not cancel a
/// valid operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotFill {
    /// A player character holds the billet directly.
    Player,
    /// A human directs an NPC through a live proxy seat.
    HumanDirectedNpc,
    /// An AI-controlled named NPC.
    AiNpc,
    /// A deterministic controller / automation holds the lease.
    Automation,
    /// Nobody holds it.
    Vacant,
}

/// One line of a muster report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MusterSlot {
    /// The unit.
    pub unit: UnitRef,
    /// Its station.
    pub station: SmolStr,
    /// What the slot resolved to.
    pub fill: SlotFill,
}

/// **One muster report** — an exempt workflow record written by a bounded pull
/// arm on cut 9's ONE campaign `Coarse` tick. It leaks nothing: it names the
/// operation's own units and stations and **no account id, no covert operation
/// and no unobserved unit's position**.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "MusterReportWire", deny_unknown_fields)]
pub struct MusterReport {
    /// The operation.
    pub operation: OperationId,
    /// Which sweep produced it (the tick's own monotonic cycle).
    pub cycle: u64,
    /// When (world IC clock).
    pub taken_at: ClockStamp,
    /// The slots, bounded on records visited.
    pub slots: Vec<MusterSlot>,
    /// **The distinctness verdict, and only the verdict** (Q11A/R3/C23A): whether
    /// every human-directed slot resolved to a distinct account. `None` means the
    /// oracle could not answer, which **fails closed** at roster lock — the
    /// account id itself never enters this or any other record.
    #[serde(default)]
    pub accounts_distinct: Option<bool>,
    /// Whether the roster is locked (the distinctness gate has run and passed).
    #[serde(default)]
    pub locked: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MusterReportWire {
    operation: OperationId,
    cycle: u64,
    taken_at: ClockStamp,
    slots: Vec<MusterSlot>,
    #[serde(default)]
    accounts_distinct: Option<bool>,
    #[serde(default)]
    locked: bool,
}

impl TryFrom<MusterReportWire> for MusterReport {
    type Error = SeatRecordError;
    fn try_from(wire: MusterReportWire) -> Result<Self, Self::Error> {
        let report = MusterReport {
            operation: wire.operation,
            cycle: wire.cycle,
            taken_at: wire.taken_at,
            slots: wire.slots,
            accounts_distinct: wire.accounts_distinct,
            locked: wire.locked,
        };
        report.check()?;
        Ok(report)
    }
}

impl MusterReport {
    /// Re-run every constructor invariant (§4 rule 1).
    pub fn check(&self) -> Result<(), SeatRecordError> {
        if self.slots.len() > MAX_MUSTER_SLOTS {
            return Err(SeatRecordError::TooMany {
                what: "a muster report's slots",
                actual: self.slots.len(),
                max: MAX_MUSTER_SLOTS,
            });
        }
        for slot in &self.slots {
            bounded_label("a muster slot's station", &slot.station)?;
        }
        if self.locked && self.accounts_distinct != Some(true) {
            return Err(SeatRecordError::Inconsistent(
                "a locked roster must carry a passing distinctness verdict",
            ));
        }
        Ok(())
    }

    /// How many slots resolved to nobody. **A vacancy never cancels a valid
    /// operation** — this is a report, not a gate.
    #[must_use]
    pub fn vacancies(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| slot.fill == SlotFill::Vacant)
            .count()
    }
}

/// A named NPC the seat may drive, resolved from an object id.
#[must_use]
pub fn unit_ref(id: ObjId) -> UnitRef {
    UnitRef(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::ClockRef;

    fn stamp(at: i64) -> ClockStamp {
        ClockStamp {
            clock: ClockRef::WorldIc { generation: 1 },
            at,
        }
    }

    fn scope() -> SeatScope {
        SeatScope {
            unit: UnitRef(ObjId(10)),
            station: SmolStr::new_static("pilot"),
            channels: BTreeSet::from([SmolStr::new_static("movement")]),
            mission_order: SmolStr::new_static("escort"),
            roe: SmolStr::new_static("defensive"),
            permitted_area: None,
        }
    }

    fn offer() -> SeatOffer {
        let mut offer = SeatOffer {
            id: SeatOfferId(1),
            operation: OperationId(3),
            audience: AudienceSpec::default(),
            eligibility: EligibilitySpec::default(),
            expires: stamp(SEAT_OFFER_TTL_SECS),
            scope: scope(),
            revocation: RevocationTerms {
                by_commander: true,
                on_operation_end: true,
            },
            fallback: FallbackSpec {
                holder: UnitRef(ObjId(11)),
            },
            consent_hash: [0; 32],
        };
        offer.consent_hash = seat_consent_hash(&offer);
        offer
    }

    #[test]
    fn e7c10_b2_a_seat_offer_round_trips_through_its_wire() {
        let offer = offer();
        let value = serde_json::to_value(&offer).expect("serializes");
        let back: SeatOffer = serde_json::from_value(value).expect("decodes");
        assert_eq!(back, offer);
        assert!(back.consent_intact());
    }

    #[test]
    fn e7c10_b2_an_over_cap_channel_set_refuses_on_load_never_truncates() {
        let mut offer = offer();
        offer.scope.channels = (0..=MAX_SEAT_CHANNELS)
            .map(|n| SmolStr::new(format!("c{n}")))
            .collect();
        let value = serde_json::to_value(&offer).expect("serializes");
        let back = serde_json::from_value::<SeatOffer>(value);
        assert!(back.is_err(), "an over-cap stored set must refuse on load");
    }

    #[test]
    fn e7c10_b2_an_over_cap_audience_refuses_on_load() {
        let mut offer = offer();
        offer.audience.members = (0..=mush_economy::MAX_AUDIENCE)
            .map(|n| PrincipalId::Object(ObjId(n as u32 + 1)))
            .collect();
        let value = serde_json::to_value(&offer).expect("serializes");
        assert!(serde_json::from_value::<SeatOffer>(value).is_err());
    }

    #[test]
    fn e7c10_b2_the_seat_domain_never_collides_with_the_operation_or_contract_domain() {
        // The domain is the FIRST canonical field, so a seat hash can never be
        // replayed into another hash space (cut 9 B1's discipline).
        let offer = offer();
        let hash = seat_consent_hash(&offer);
        let other = consent_digest(
            mush_economy::CONSENT_DOMAIN_CAMPAIGN_OPERATION,
            &serde_json::json!({ "domain": "campaign.operation.v1", "operation": 3u64 }),
        );
        assert_ne!(hash, other);
    }

    #[test]
    fn e7c10_b2_a_mutated_surface_breaks_the_consent_hash() {
        let mut offer = offer();
        assert!(offer.consent_intact());
        offer.scope.channels.insert(SmolStr::new_static("fire"));
        assert!(
            !offer.consent_intact(),
            "widening the seat after acceptance must break the hash"
        );
    }

    #[test]
    fn e7c10_b3_two_seats_for_one_player_mint_two_uncorrelated_tokens() {
        let a = DriverToken::mint(
            OperationId(3),
            SeatOfferId(1),
            UnitRef(ObjId(50)),
            &stamp(1),
        );
        let b = DriverToken::mint(
            OperationId(3),
            SeatOfferId(2),
            UnitRef(ObjId(51)),
            &stamp(1),
        );
        assert_ne!(a, b);
        // Neither token carries anything that could reach an account: the mint's
        // whole input is the seat's own coordinates.
        assert!(a.0.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(b.0.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn e7c10_b3_a_grant_round_trips_and_refuses_an_empty_token() {
        let grant = SeatGrant {
            operation: OperationId(3),
            offer: SeatOfferId(1),
            actor: UnitRef(ObjId(50)),
            driver: DriverToken::mint(
                OperationId(3),
                SeatOfferId(1),
                UnitRef(ObjId(50)),
                &stamp(1),
            ),
            accepted_at: stamp(1),
            consent_hash: [7; 32],
            state: GrantState::Dormant,
        };
        let value = serde_json::to_value(&grant).expect("serializes");
        assert_eq!(
            serde_json::from_value::<SeatGrant>(value.clone()).expect("decodes"),
            grant
        );
        let mut bad = value;
        bad["driver"] = serde_json::json!("");
        assert!(serde_json::from_value::<SeatGrant>(bad).is_err());
    }

    #[test]
    fn e7c10_b7_an_over_cap_crew_complement_refuses_on_load() {
        let mut crew = CrewComplement::new(UnitRef(ObjId(10)));
        for n in 0..=MAX_CREW_NAMED {
            crew.named
                .insert(SmolStr::new(format!("s{n}")), UnitRef(ObjId(n as u32 + 20)));
        }
        let value = serde_json::to_value(&crew).expect("serializes");
        assert!(serde_json::from_value::<CrewComplement>(value).is_err());
    }

    #[test]
    fn e7c10_b7_a_locked_roster_without_a_passing_verdict_refuses_on_load() {
        let report = MusterReport {
            operation: OperationId(3),
            cycle: 1,
            taken_at: stamp(1),
            slots: Vec::new(),
            accounts_distinct: None,
            locked: true,
        };
        let value = serde_json::to_value(&report).expect("serializes");
        assert!(
            serde_json::from_value::<MusterReport>(value).is_err(),
            "an oracle that answered `None` must fail closed on load too"
        );
    }

    #[test]
    fn e7c10_b7_a_vacancy_is_a_resolution_not_an_error() {
        let report = MusterReport {
            operation: OperationId(3),
            cycle: 1,
            taken_at: stamp(1),
            slots: vec![
                MusterSlot {
                    unit: UnitRef(ObjId(10)),
                    station: SmolStr::new_static("pilot"),
                    fill: SlotFill::Vacant,
                },
                MusterSlot {
                    unit: UnitRef(ObjId(10)),
                    station: SmolStr::new_static("gunner"),
                    fill: SlotFill::AiNpc,
                },
            ],
            accounts_distinct: Some(true),
            locked: false,
        };
        assert!(report.check().is_ok());
        assert_eq!(report.vacancies(), 1);
    }
}
