//! **The order** (16b cut N.2, N3; doc 16 §6 "Deliberation": *an order
//! (objective, target, horizon, constraints) with a time to live*).
//!
//! One squad's standing instruction, as plain data. It lives **outside**
//! `crate::arena` because everything that produces one is outside it — the
//! trigger, the request and the reply are all the world's — and it is imported
//! *into* the arena because its effect is the arena's: [`Order`] is carried in as
//! an [`crate::arena::intents::IntentKind::Order`] and steers the reflex
//! controller's goal for as long as [`Order::ttl_ms`] says.
//!
//! # Why it is data and nothing else
//!
//! The arena library holds no I/O and no clock of its own (D-16-1, D-16-3), and
//! the gate in `crates/mush-contract-tests/tests/arena_library.rs` keeps it that
//! way by reading the source. So an order carries no handle, no callback and no
//! provider: it is a record the world hands over, exactly as a pilot's `HEADING`
//! is, and the arena treats it with the same suspicion — re-validated at apply,
//! refused by name, and expired against **arena** time rather than the clock it
//! was written on.
//!
//! # What the model may say
//!
//! Everything here is bounded, and the bounds are checked twice: once by
//! [`Order::validate`] when the structured reply is parsed (so a malformed order
//! never reaches the world task), and once by the arena when the intent is
//! applied (so an order that was legal when it was written is still legal when it
//! lands). The vocabulary — four objectives, three range bands — is closed and
//! lives in Rust, which is doc 16 §7's *what stays Rust*.

use mush_core::ObjId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::arena::state::ContactId;
use crate::deliberation::squad::SquadId;

/// The shortest time to live an order may carry, milliseconds of arena time.
///
/// Below a second an order would expire before the squad's next reflex tick could
/// read it, which is an order that never happened and a `order_expired` line that
/// says nothing.
pub const MIN_TTL_MS: u32 = 1_000;

/// The longest time to live an order may carry, milliseconds of arena time — ten
/// minutes.
///
/// Doc 16 §6's fallback is *the reflex doctrine, not idle*, and a squad that has
/// not heard from its commander in ten minutes is a squad on doctrine whatever the
/// order said. The ceiling is what makes "unrenewed" mean something.
pub const MAX_TTL_MS: u32 = 600_000;

/// The longest horizon an order may name, milliseconds — the same ten minutes.
///
/// The horizon is what the commander *intended* the order to cover and is carried
/// for the record and for the prompt that renews it; the time to live is what the
/// engine enforces. They are separate because a commander may reasonably say "take
/// that ridge in the next two minutes" and still want the order re-examined in
/// thirty seconds.
pub const MAX_HORIZON_MS: u32 = 600_000;

/// **What a squad is being told to do** (doc 16 §6: *objective*).
///
/// Four arms and no fifth: the machine has exactly four things it can be pointed
/// at, and an objective the reflex controller could not carry out would be an
/// order that reads well and does nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "objective")]
pub enum Objective {
    /// Stop where you are and hold. The machine parks: goal here, speed zero, no
    /// target taken and nothing fired.
    Hold,
    /// Make for this cell. The machine patrols to it instead of walking the
    /// waypoint graph.
    ///
    /// **16b cut E.2 (16c `B-41`): a cell no route reaches ends the order for
    /// that unit.** The unit's path search fails once, the `unreachable` edge's
    /// fallback waypoint takes, the order is dropped and the squad's next signal
    /// says `order_expired` — because the alternative, which is what the engine
    /// did until this cut, was a unit steering at ground it cannot reach for the
    /// whole time to live, paying a failed search every `repath_ms`, with the
    /// hostile edges suppressed and no way of telling the commander.
    Advance {
        /// The cell, as `(column, row)` of the arena window.
        to: (u16, u16),
    },
    /// Fight. The machine runs its ordinary `Approach → Engage`, preferring the
    /// hinted contact when the unit holds one on the same object.
    Engage {
        /// The **commander's** opaque contact id, or `None` for "whatever you
        /// find". It is resolved against the commander's own contact store and
        /// then looked for in each unit's; a unit that holds nothing on that
        /// object ignores the hint and takes its doctrine's best target, which is
        /// doc 16 §3.6's observer rule read through an order.
        target_hint: Option<ContactId>,
    },
    /// Break off toward your own entry zone and hold fire.
    Withdraw,
}

impl Objective {
    /// The wire word — what the ledger, the tool schema and a notice name it by.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Objective::Hold => "hold",
            Objective::Advance { .. } => "advance",
            Objective::Engage { .. } => "engage",
            Objective::Withdraw => "withdraw",
        }
    }

    /// The byte [`crate::arena::hash::state_hash`] folds in.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            Objective::Hold => 1,
            Objective::Advance { .. } => 2,
            Objective::Engage { .. } => 3,
            Objective::Withdraw => 4,
        }
    }
}

/// **The band an order may not shoot past** — the cockpit's own range words
/// ([`crate::match_::cockpit::contact_band`]) minus `beyond`, which is not a band
/// a weapon reaches at all.
///
/// A new vocabulary would be a second way to say the same thing; these three are
/// what `CONTACTS` already prints, so an order a staff member reads uses the words
/// a pilot reads — and since **16b cut E.1** they are the same *distances* too
/// (16c `T-81`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RangeBand {
    /// The nearest third of what the platform's sight and its gun's last band
    /// allow.
    Close,
    /// The middle third of it.
    Effective,
    /// Out to the weapon's own last band, which is past anything the sight can
    /// hold — so an order capped here caps nothing and is legal for exactly that
    /// reason: a commander may say "no restriction" without a fourth word.
    Far,
}

impl RangeBand {
    /// The wire word.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            RangeBand::Close => "close",
            RangeBand::Effective => "effective",
            RangeBand::Far => "far",
        }
    }

    /// The band a word names, or `None`.
    #[must_use]
    pub fn parse(word: &str) -> Option<RangeBand> {
        match word {
            "close" => Some(RangeBand::Close),
            "effective" => Some(RangeBand::Effective),
            "far" => Some(RangeBand::Far),
            _ => None,
        }
    }

    /// **The outer edge of this band**, centimetres, against the band edges of a
    /// weapon and a platform's own nominal sight — the exact inverse of
    /// [`crate::match_::cockpit::contact_band`], and pinned to it by a test.
    ///
    /// `limit` is `min(last band, sight_cm)`, and the last band alone when
    /// `sight_cm` is `0` (a hull with no scope at all), which is
    /// `contact_band`'s own rule. Then `Close` is `limit / 3`, `Effective` is
    /// `limit × 2 / 3` and `Far` is the last band, so
    /// `contact_band(band.ceiling_cm(bands, sight), bands, sight)` is that band's
    /// own word for all three.
    ///
    /// **16b cut E.1 gave it the sight** (16c `T-81`). Cut F.3 had re-based it
    /// from thirds of the platform's longest reach onto the fired weapon's bands,
    /// but the pilot's word had already moved on to the lesser of that and the
    /// sight — so a lead's `close` was 500 m while a pilot's was 200 m, and a
    /// squad ordered to hold `close` was ordered to a distance no pilot's
    /// `CONTACTS` could name. One rule, two callers: this and `contact_band`.
    ///
    /// Nought for an empty list, which is a platform that carries nothing that
    /// fires — the same reading `contact_band`'s `beyond` is, and it caps nothing
    /// a squad could have done anyway. A weapon with a reach and no authored bands
    /// is **not** that case; it answers one band at its reach
    /// ([`crate::arena::state::WeaponSummary::bands_cm`]).
    #[must_use]
    pub fn ceiling_cm(&self, bands_cm: &[u32], sight_cm: u32) -> u64 {
        let Some(last) = bands_cm.last() else {
            return 0;
        };
        let last = u64::from(*last);
        let sight = u64::from(sight_cm);
        let limit = if sight == 0 { last } else { last.min(sight) };
        match self {
            RangeBand::Close => limit / 3,
            RangeBand::Effective => limit.saturating_mul(2) / 3,
            RangeBand::Far => last,
        }
    }
}

/// **The two things an order may forbid** (doc 16 §6: *constraints*).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Constraints {
    /// Hold fire: the machine locks and manoeuvres but stages no `FIRE`.
    pub hold_fire: bool,
    /// The furthest band the squad may shoot from, and the band `Approach` holds.
    /// `None` leaves both to `[tacsim.arena.reflex].engage_band_permille`.
    pub max_range_band: Option<RangeBand>,
}

/// **One squad's standing instruction** (doc 16 §6).
///
/// It is issued by a commander the squad cannot see, carried in as an intent, and
/// obeyed until its time to live runs out in **arena** milliseconds. Nothing on it
/// is authority: the arena re-checks every gate a pilot's order goes through, and
/// a unit that cannot do what the order says does the nearest thing its doctrine
/// allows rather than refusing to move.
// `deny_unknown_fields` is deliberately **absent**: `objective` is a flattened
// internally-tagged enum, and serde refuses to combine the two. It is not a hole —
// an `Order` is never decoded from anything a player or a model wrote. The model's
// reply is decoded as `crate::deliberation::request::OrderPayload`, which *is*
// closed, and this type is only ever encoded and decoded across the world task's
// own async seam.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Order {
    /// Which squad it was written for. Carried so the `order_expired` line and the
    /// notice can name it, and so a unit that changed squads mid-match does not
    /// obey its old one.
    pub squad: SquadId,
    /// What the squad is being told to do.
    #[serde(flatten)]
    pub objective: Objective,
    /// The squad-wide target preference, as the **commander's** opaque contact id.
    /// [`Objective::Engage`]'s own hint takes precedence where both are set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<ContactId>,
    /// How long the commander meant the order to cover, milliseconds. Carried for
    /// the record and the renewal prompt; the engine enforces [`Order::ttl_ms`].
    pub horizon_ms: u32,
    /// What the order forbids.
    #[serde(default)]
    pub constraints: Constraints,
    /// How long the order stands once the arena accepts it, milliseconds of arena
    /// time. Past it the squad drops to doctrine with one `order_expired` line per
    /// unit.
    pub ttl_ms: u32,
    /// When the commander issued it, **world** milliseconds. It is a record and
    /// never a clock the arena reads: expiry is measured from the arena
    /// millisecond the order was applied at (see [`StandingOrder`]).
    pub issued_ms: i64,
    /// **The unit whose scope a target hint is written in** (16b cut Y.1; 16c N3):
    /// a pilot's own tank when a pilot typed `ORDER`, `None` for the squad's own
    /// commander (the LLM seam). `ordered_target` resolves the hint there; the
    /// receiving unit still chooses only among what it holds itself.
    ///
    /// Folded into `crate::arena::hash::state_hash` beside the constraint byte
    /// (the one hash move of cut Y), because it decides which object a hint names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issuer: Option<ObjId>,
    /// **The commander's word for an `Advance` cell** (16b cut Y.1) — the key
    /// spot's name — for the record and the debrief. Read by nothing that steers,
    /// and deliberately not hashed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spot: Option<SmolStr>,
}

/// Why an order was refused before it ever reached the arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum OrderError {
    /// `ttl_ms` is outside `[MIN_TTL_MS, MAX_TTL_MS]`.
    #[error("an order's time to live must be between {MIN_TTL_MS} and {MAX_TTL_MS} milliseconds")]
    Ttl,
    /// `horizon_ms` is above [`MAX_HORIZON_MS`].
    #[error("an order's horizon may not exceed {MAX_HORIZON_MS} milliseconds")]
    Horizon,
    /// `Advance` named a cell that is not on the window.
    #[error("an order may not advance to a cell that is not on the window")]
    OffWindow,
    /// `issued_ms` is negative.
    #[error("an order may not be issued before the epoch")]
    Issued,
}

impl Order {
    /// **Check the bounds** — run once when the structured reply is parsed and
    /// again by nothing, because the arena's own gates are about the *unit* rather
    /// than the order.
    ///
    /// `window` is the arena window's `(columns, rows)`; pass `None` where it is
    /// not known and the cell check is skipped (the arena's path search treats an
    /// off-window goal as `unreachable` either way).
    ///
    /// # Errors
    ///
    /// [`OrderError`], one named reason each, so the repair turn can be told what
    /// was wrong without echoing the model's own words back at it.
    pub fn validate(&self, window: Option<(u16, u16)>) -> Result<(), OrderError> {
        if self.ttl_ms < MIN_TTL_MS || self.ttl_ms > MAX_TTL_MS {
            return Err(OrderError::Ttl);
        }
        if self.horizon_ms > MAX_HORIZON_MS {
            return Err(OrderError::Horizon);
        }
        if self.issued_ms < 0 {
            return Err(OrderError::Issued);
        }
        if let (Objective::Advance { to }, Some((cols, rows))) = (self.objective, window)
            && (to.0 >= cols || to.1 >= rows)
        {
            return Err(OrderError::OffWindow);
        }
        Ok(())
    }

    /// The contact the order points at, the objective's hint first.
    #[must_use]
    pub const fn preferred_contact(&self) -> Option<ContactId> {
        match self.objective {
            Objective::Engage { target_hint } => match target_hint {
                Some(contact) => Some(contact),
                None => self.target,
            },
            Objective::Hold | Objective::Advance { .. } | Objective::Withdraw => self.target,
        }
    }
}

/// **An order as the arena holds it**, with the two arena timestamps that decide
/// when it stops being one.
///
/// `accepted_ms` is the arena millisecond [`crate::arena::intents::apply_intent`]
/// took it at, and `expires_ms` is that plus [`Order::ttl_ms`]. Expiry is measured
/// in arena time for the same reason every other reflex deadline is (D-16-2): a
/// time to live counted in pulses would be a different order at every tempo.
///
/// **There are two ways an order stops steering a unit** (16b cut E.2, 16c
/// `B-41`). Running out unrenewed is the first and the one doc 16 §6 names; the
/// second is an [`Objective::Advance`] whose cell that unit's path search cannot
/// reach, which ends the order **for that unit alone**, at the tick the search
/// failed. Both drop the machine to doctrine, both journal one
/// `crate::arena::ledger::EventKind::OrderExpired` line, and both reach the
/// commander as `crate::arena::command::SignalReason::OrderExpired`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StandingOrder {
    /// The order.
    pub order: Order,
    /// The arena millisecond it was applied at.
    pub accepted_ms: i64,
    /// The arena millisecond it stops standing at.
    pub expires_ms: i64,
}

impl StandingOrder {
    /// Stamp an order with the arena time it arrived at.
    #[must_use]
    pub fn accepted(order: Order, now_ms: i64) -> StandingOrder {
        let expires_ms = now_ms.saturating_add(i64::from(order.ttl_ms));
        StandingOrder {
            order,
            accepted_ms: now_ms,
            expires_ms,
        }
    }

    /// Whether `t_ms` is at or past the order's expiry.
    #[must_use]
    pub const fn expired_at(&self, t_ms: i64) -> bool {
        t_ms >= self.expires_ms
    }
}

/// The one in-world sentence doc 16 §6 names, said when an order expires
/// unrenewed: *the visible in-world effect ("mind link dropped, unit on
/// doctrine")*.
pub const MIND_LINK_DROPPED: &str = "mind link dropped, unit on doctrine";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::state::Team;

    fn squad() -> SquadId {
        SquadId {
            team: Team::Red,
            index: 1,
        }
    }

    fn order(objective: Objective) -> Order {
        Order {
            squad: squad(),
            objective,
            target: None,
            horizon_ms: 60_000,
            constraints: Constraints::default(),
            ttl_ms: 30_000,
            issued_ms: 1,
            issuer: None,
            spot: None,
        }
    }

    #[test]
    fn the_objectives_and_bands_round_trip_their_words() {
        for objective in [
            Objective::Hold,
            Objective::Advance { to: (1, 2) },
            Objective::Engage { target_hint: None },
            Objective::Withdraw,
        ] {
            assert!(!objective.as_str().is_empty());
            assert!(objective.code() > 0);
        }
        for band in [RangeBand::Close, RangeBand::Effective, RangeBand::Far] {
            assert_eq!(RangeBand::parse(band.as_str()), Some(band));
        }
        assert_eq!(RangeBand::parse("beyond"), None);
    }

    /// **16b cut E.1 (`T-81`)**: `RangeBand::ceiling_cm` is the inverse of
    /// `cockpit::contact_band`, so a commander's word and a pilot's word name one
    /// distance.
    ///
    /// The property, for every band `b`:
    /// `contact_band(b.ceiling_cm(bands, sight), bands, sight) == b.as_str()`.
    /// It is asserted twice, because the sight is what cut E.1 added: once at
    /// `sight_cm = 0`, the hull with no scope, where the limit is the last band;
    /// and once at a sight **shorter than the first band**, which is every shipped
    /// platform (600 m of sight against a 1.5 km inner band) and the case a
    /// ceiling read off the gun alone got wrong.
    #[test]
    fn the_bands_agree_with_the_cockpits_own_words() {
        let bands = [9_000_u32, 18_000];
        for sight in [0_u32, 6_000] {
            for band in [RangeBand::Close, RangeBand::Effective, RangeBand::Far] {
                let at = band.ceiling_cm(&bands, sight);
                assert_eq!(
                    crate::match_::cockpit::contact_band(at, &bands, sight),
                    band.as_str(),
                    "at a sight of {sight}, the ceiling of {} ({at} cm) must still read as {}",
                    band.as_str(),
                    band.as_str()
                );
            }
        }
        // And the sight really does move the ceiling: thirds of 6,000 rather than
        // thirds of 18,000.
        assert_eq!(RangeBand::Close.ceiling_cm(&bands, 0), 6_000);
        assert_eq!(RangeBand::Close.ceiling_cm(&bands, 6_000), 2_000);
        assert_eq!(RangeBand::Effective.ceiling_cm(&bands, 6_000), 4_000);
        assert_eq!(
            RangeBand::Far.ceiling_cm(&bands, 6_000),
            18_000,
            "`far` is the weapon's own last band: a commander saying no restriction"
        );
        // A platform with nothing that fires caps nothing it could have done.
        assert_eq!(RangeBand::Effective.ceiling_cm(&[], 6_000), 0);
    }

    #[test]
    fn validation_refuses_a_malformed_order() {
        assert_eq!(order(Objective::Hold).validate(None), Ok(()));
        let mut short = order(Objective::Hold);
        short.ttl_ms = 0;
        assert_eq!(short.validate(None), Err(OrderError::Ttl));
        let mut long = order(Objective::Hold);
        long.ttl_ms = MAX_TTL_MS + 1;
        assert_eq!(long.validate(None), Err(OrderError::Ttl));
        let mut far = order(Objective::Hold);
        far.horizon_ms = MAX_HORIZON_MS + 1;
        assert_eq!(far.validate(None), Err(OrderError::Horizon));
        let mut early = order(Objective::Hold);
        early.issued_ms = -1;
        assert_eq!(early.validate(None), Err(OrderError::Issued));
        let off = order(Objective::Advance { to: (16, 3) });
        assert_eq!(off.validate(Some((16, 16))), Err(OrderError::OffWindow));
        assert_eq!(off.validate(Some((17, 16))), Ok(()));
        assert_eq!(off.validate(None), Ok(()));
    }

    #[test]
    fn the_objectives_hint_beats_the_squads_target() {
        let hint = ContactId(7);
        let wide = ContactId(9);
        let mut engage = order(Objective::Engage {
            target_hint: Some(hint),
        });
        engage.target = Some(wide);
        assert_eq!(engage.preferred_contact(), Some(hint));
        let mut blind = order(Objective::Engage { target_hint: None });
        blind.target = Some(wide);
        assert_eq!(blind.preferred_contact(), Some(wide));
        let mut hold = order(Objective::Hold);
        hold.target = Some(wide);
        assert_eq!(hold.preferred_contact(), Some(wide));
    }

    #[test]
    fn a_standing_order_expires_in_arena_time() {
        let standing = StandingOrder::accepted(order(Objective::Hold), 4_000);
        assert_eq!(standing.accepted_ms, 4_000);
        assert_eq!(standing.expires_ms, 34_000);
        assert!(!standing.expired_at(33_999));
        assert!(standing.expired_at(34_000));
    }
}
