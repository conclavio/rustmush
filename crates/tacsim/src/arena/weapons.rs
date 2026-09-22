//! **Weapons and the shot** (16b cut W.1, W1; doc 16 §3.5 "Weapon definition",
//! "Ammunition definition", "Acceptance and cost" and "Scheduled impact", §3.6's
//! one refusal, §3.7's `LOCK` and `FIRE`, §3.1's tempo table).
//!
//! Three things live here and nothing else: the weapon's live state on a unit, the
//! two discrete actions doc 16 §3.7 leaves for this cut, and the record of a shot
//! in flight.
//!
//! # What a weapon *is*, in three layers
//!
//! 1. **The definition** — [`crate::arena::state::WeaponSummary`], crossed once at
//!    check-out (cut S.1 carried it, this cut reads it): the bands and their
//!    dispersion, the cycle, the burst, the muzzle speed, the load. Immutable for
//!    the life of the match, because a weapon's numbers are content and content
//!    cannot change under a running arena.
//! 2. **The two live cells** — `ready_at_ms` and `ammo` on
//!    [`crate::arena::state::UnitState`], one row per weapon, laid down complete at
//!    check-out. A shot is exactly *set one, decrement the other*, which is why
//!    there is no third place a weapon's state can disagree with itself.
//! 3. **What the pilot is shown** — [`FrameWeapon`], built per pulse off the two
//!    above plus the platform's lock, and rendered by `WEAPONSTATUS`. It is a
//!    projection and never a second model: `ready` is `ready_at_ms <= now_ms` and
//!    not a flag somebody has to remember to clear.
//!
//! # The refusal order is doc 16 §3.7's column, read literally
//!
//! `FIRE`'s refusals are *no lock, cycling, empty, power not online* — in that
//! order, and the order is the interesting part: **power is checked last**. Doc 16
//! §3.5 "Acceptance and cost" lists the four requirements and §3.7's table fixes
//! their order, and the reason it is a good order is that it tells a pilot the thing
//! they can act on first. A cold tank with no lock is told to get a lock, because
//! that is the longer job. K.2's cockpit already refused in this order with the
//! world's view of the frame, and [`Arena::resolve_fire`] refuses in the same order
//! against the arena's own state one pulse later, so a stale frame changes which
//! half answers and never what it says.
//!
//! # Why a blocked shot is not a refusal
//!
//! Doc 16 §3.5: *a shot whose line is blocked at acceptance, or whose target is
//! behind full cover, is accepted: it consumes ammunition and the cycle exactly like
//! any shot and resolves as a `miss` with the blocker as its reason. There is no
//! free probe of line of sight.* So the gates below are the lock, the cycle, the
//! rounds and the power, and **nothing about the ground between the two units**.
//! Acceptance does take a trace — cut W.2 needs the target's cover for the
//! resolution — and deliberately throws its verdict on the *line* away: the round
//! is spent, and the wall it is going to hit is named at the impact
//! ([`crate::arena::damage::BLOCKED`]) rather than handed back as a refusal a pilot
//! could have used to map the ground for free.
//!
//! # What acceptance resolves, and what the impact does with it
//!
//! **16b cut W.2** gave the shot its arithmetic. Doc 16 §3.5 resolves *at the
//! moment `FIRE` is accepted*, so [`Arena::resolve_fire`] takes one trace, runs
//! [`crate::arena::resolution`]'s five steps, and records the answer on the
//! [`Shot`] — the band, the presented width, the cone, the probability — beside the
//! predicted point. What arrives later re-validates against that record
//! ([`crate::arena::damage`]) and never re-aims: the shooter's speed, its gunner and
//! its sight were facts about the barrel, not about the flight.
//!
//! The trace at acceptance is **not** a gate. Doc 16 §3.5 is explicit that a shot
//! whose line is blocked or whose target is behind full cover is accepted and costs
//! its round; the trace here is what gives the resolution its cover, and its verdict
//! on the line is spent at the impact, by name.

use mush_core::ObjId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::arena::index::range_cm;
use crate::arena::intents::Refusal;
use crate::arena::ledger::EventKind;
use crate::arena::resolution::Resolution;
use crate::arena::state::{Arena, ContactId, DueKind, Power, UnitState, WeaponSummary};

// ---------------------------------------------------------------------------
// The definition's two derived readings
// ---------------------------------------------------------------------------

/// **The weapon a bare `FIRE` fires** — the stock tank's cannon (doc 16 §3.7:
/// *default cannon*); a unit that carries no component by this name falls back to
/// its first weapon in component order.
///
/// **16b cut F.3 moved it here** from `crate::match_::cockpit`, which re-exports
/// it under its old name. Three things read against this weapon's authored bands
/// and had to be one rule: the cockpit's `FIRE` with no argument, the band word a
/// contact row prints, and the ceiling an order's `max_range_band` caps the
/// reflex controller at.
///
/// **16b cut E.1** added a fourth reading that is deliberately *not* this one:
/// [`UnitState::loaded_weapon`], the same list read against the magazines, which
/// is what the machine's own fighting distances are priced off. A bare `FIRE`
/// with an empty cannon is owed the cannon's `Empty` refusal; a machine with an
/// empty cannon is owed the band of the gun it can still shoot.
pub const DEFAULT_WEAPON: &str = "cannon";

/// **One range band**, as the arena carries it (16b cut W.1; doc 16 §3.5 "Weapon
/// definition": *range bands with `max_range_cm` and a per-band
/// `dispersion_mrad`*).
///
/// The arena's own copy of `crate::definitions::weapon::RangeBand`, for the same
/// reason [`crate::arena::perception::SignatureFactor`] is a copy of the sensor
/// decoder's row: the content type is `serde`-shaped and lives beside the pack
/// loader, and the arena holds neither (D-16-1, D-16-3). Cut T's check-out copies
/// the rows across (`crate::match_::arena_def::weapon_summaries`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeaponBand {
    /// The band's outer edge, centimetres. Everything inside it and outside the
    /// previous band's edge is in this band.
    pub max_range_cm: u32,
    /// The weapon's own error cone in this band, milliradians — cut W.2's first
    /// term.
    pub dispersion_mrad: u32,
}

impl WeaponSummary {
    /// Whether this is a beam: doc 16 §3.5 makes `muzzle_speed_cms` zero *exactly*
    /// for one, and the content loader refuses a direct weapon at zero and a beam at
    /// anything else, so the zero is a discriminant and not a missing number.
    #[must_use]
    pub const fn is_beam(&self) -> bool {
        self.muzzle_speed_cms == 0
    }

    /// **This weapon's band edges, outward**, centimetres (16b cut F.3) — what
    /// `crate::match_::cockpit::contact_band` and
    /// `crate::deliberation::order::RangeBand::ceiling_cm` read a word against,
    /// and what [`UnitState::fight_inner_cm`] takes its first entry of.
    ///
    /// A weapon that authored no bands answers **one** band at its own
    /// [`WeaponSummary::max_range_cm`], which that field's own doc says is a shape
    /// content may field. Answering an empty list would make the word `beyond` at
    /// every range including point blank, and an ordered squad's engage ceiling
    /// nought — the band useless in the opposite direction from the defect this
    /// cut fixed. The empty list is reserved for a platform that carries nothing
    /// that fires, where `beyond` is the honest word.
    #[must_use]
    pub fn bands_cm(&self) -> Vec<u32> {
        if self.bands.is_empty() {
            return vec![self.max_range_cm];
        }
        self.bands.iter().map(|band| band.max_range_cm).collect()
    }

    /// **This weapon's innermost band edge**, centimetres (16b cut E.1) — exactly
    /// `bands_cm().first()`, without the allocation, because
    /// [`UnitState::fight_inner_cm`] is asked once per held contact per pulse by
    /// [`crate::arena::state::Arena::hostile_in_weapons_range`].
    ///
    /// A bandless weapon answers its own reach, which is the one band
    /// [`WeaponSummary::bands_cm`] gives it.
    #[must_use]
    pub fn inner_band_cm(&self) -> u32 {
        self.bands
            .first()
            .map_or(self.max_range_cm, |band| band.max_range_cm)
    }

    /// **The band a range falls in**, as a one-based index and the band itself
    /// (doc 16 §3.5 step 2).
    ///
    /// The bands run outward, so the first whose edge the range is inside is the
    /// one: at 1.5 km and 3 km, 40 m is band 1 and 2 km is band 2. A range past the
    /// last band is `None` — out of reach, which is a shot cut W.2 resolves as a
    /// miss rather than a refusal (there is no free probe of anything, §3.5).
    #[must_use]
    pub fn band_of(&self, range_cm: u64) -> Option<(u32, WeaponBand)> {
        self.bands
            .iter()
            .enumerate()
            .find(|(_, band)| range_cm <= u64::from(band.max_range_cm))
            .and_then(|(at, band)| {
                let index = u32::try_from(at).ok()?.checked_add(1)?;
                Some((index, *band))
            })
    }

    /// **The flight time of one shot**, milliseconds (doc 16 §3.5 "Scheduled
    /// impact": `range_cm × 1000 / muzzle_speed_cms`; a beam is `0`).
    ///
    /// Integer throughout, and the multiplication is in `u64`: the arena is 2.56 km
    /// on a side, so `range_cm × 1000` is at most about 3.6 × 10⁸ and the product
    /// cannot approach the type's ceiling. A beam, and a direct weapon whose
    /// definition somehow carries a zero muzzle speed, both answer `0` — an impact
    /// scheduled at `now`, which is the honest reading of "no flight time" and never
    /// a division by zero.
    #[must_use]
    pub fn flight_ms(&self, range_cm: u64) -> i64 {
        if self.is_beam() {
            return 0;
        }
        let ms = range_cm
            .saturating_mul(1_000)
            .saturating_div(u64::from(self.muzzle_speed_cms));
        i64::try_from(ms).unwrap_or(i64::MAX)
    }
}

// ---------------------------------------------------------------------------
// The live cells
// ---------------------------------------------------------------------------

impl UnitState {
    /// One weapon's definition row by its component id, or `None` when the unit
    /// does not carry it.
    #[must_use]
    pub fn weapon(&self, id: &str) -> Option<&WeaponSummary> {
        self.weapons.iter().find(|weapon| weapon.id == id)
    }

    /// The arena millisecond that weapon may next fire at; `0` for a weapon the
    /// unit does not carry, which no caller reaches without [`UnitState::weapon`]
    /// answering first.
    #[must_use]
    pub fn weapon_ready_at_ms(&self, id: &str) -> i64 {
        self.ready_at_ms.get(id).copied().unwrap_or(0)
    }

    /// Whether that weapon's cycle has finished by `now_ms` (doc 16 §3.5: the
    /// cycle is `ready_at_ms`, and readiness is a comparison and not a flag).
    #[must_use]
    pub fn weapon_ready(&self, id: &str, now_ms: i64) -> bool {
        self.weapon_ready_at_ms(id) <= now_ms
    }

    /// How long until that weapon is ready, milliseconds; `0` when it already is.
    #[must_use]
    pub fn weapon_ready_in_ms(&self, id: &str, now_ms: i64) -> i64 {
        self.weapon_ready_at_ms(id).saturating_sub(now_ms).max(0)
    }

    /// Rounds left in that weapon.
    #[must_use]
    pub fn weapon_ammo(&self, id: &str) -> u32 {
        self.ammo.get(id).copied().unwrap_or(0)
    }

    /// **Whether this platform came into the match carrying a gun** (16b cut
    /// Q.7), whatever has happened to it since.
    ///
    /// [`UnitState::ammo`] is laid down at check-out with one row per weapon and
    /// is **never** emptied — a turret shot off takes its `weapons` rows and its
    /// `ready_at_ms` rows and deliberately leaves the rounds behind (16b cut F.1,
    /// so the check-in can still say what the tank spent). That makes it the one
    /// durable record of *was there ever a weapon here*, which is the difference
    /// between a tank that has run out of ammunition and a tank that never had
    /// any: the first is what [`crate::arena::state::EndReason::Spent`] is about,
    /// and the second is a target.
    #[must_use]
    pub fn armed_at_checkout(&self) -> bool {
        !self.ammo.is_empty()
    }

    /// **The longest reach anything the unit carries has**, centimetres — the
    /// number `tempo.alert_range_cm` is derived from (16b §9). Zero for a unit
    /// that carries no weapon, and then nothing is in range of it.
    ///
    /// **16b cut E.1 left it exactly one reader**, the `alert_range_cm`
    /// derivation in [`crate::arena::state::Arena::new`], which calls it. Nothing
    /// the reflex controller does is priced off it any more: the longest reach a
    /// platform carries is 3 km on the stock tank, which is five times what its
    /// own sight can hold, so every range test made against it was always true
    /// (16c `T-47`, `B-05`, `B-39`, `T-32`). What the machine fights at is
    /// [`UnitState::fight_limit_cm`] and [`UnitState::fight_inner_cm`].
    #[must_use]
    pub fn weapon_reach_cm(&self) -> u32 {
        self.weapons
            .iter()
            .map(|weapon| weapon.max_range_cm)
            .max()
            .unwrap_or(0)
    }

    /// **How far this platform's own sight reaches**, centimetres (16b cut E.1;
    /// doc 16 §3.6), or `0` for a hull that carries no sensor at all.
    ///
    /// The **nominal** reach — `SensorProfile::max_range_cells × cell_cm` — and
    /// deliberately so, for the reason [`crate::arena::step::Frame::sight_cm`]
    /// gives at length: it is the size of the scope and not what the scope can
    /// resolve this second. It is the one product, and
    /// [`crate::arena::step::Frame::sight_cm`],
    /// [`crate::arena::command::SquadSignal::sight_cm`] and the three readings
    /// below are its readers.
    #[must_use]
    pub fn sight_cm(&self, cell_cm: u16) -> u32 {
        self.sensor.as_ref().map_or(0, |sensor| {
            u32::from(sensor.max_range_cells).saturating_mul(u32::from(cell_cm))
        })
    }

    /// **The weapon this platform would actually fire** (16b cut E.1, 16c
    /// `T-46`): [`DEFAULT_WEAPON`] while it has rounds, else the first in
    /// component order that has any, else `None`.
    ///
    /// It is [`UnitState::default_weapon`] **read against the magazines**, and
    /// the two are deliberately different questions. `default_weapon` is the
    /// cockpit's bare-`FIRE` rule and ignores ammunition on purpose — a pilot who
    /// types `FIRE` with an empty cannon is owed the cannon's refusal and not a
    /// burst from the coaxial. This one is what the machine closes to: a tank
    /// whose cannon is dry fights at its coaxial's band rather than holding 1.5 km
    /// with nothing to send there.
    #[must_use]
    pub fn loaded_weapon(&self) -> Option<&WeaponSummary> {
        self.default_weapon()
            .filter(|weapon| self.weapon_ammo(&weapon.id) > 0)
            .or_else(|| {
                self.weapons
                    .iter()
                    .find(|weapon| self.weapon_ammo(&weapon.id) > 0)
            })
    }

    /// **The furthest this platform fights at**, centimetres (16b cut E.1): the
    /// lesser of its own nominal sight and the reach of the weapon it would fire.
    ///
    /// The lesser, because the sight is what bounds a fight on this map — 16b cut
    /// D.1 ruled the same way for the pilot's band word, and this is that ruling
    /// for the machine. A platform with no sensor answers the weapon alone (a
    /// sight of `0` is *no scope*, not *no reach*); one with nothing loaded
    /// answers `0`, and then nothing is in range of it.
    #[must_use]
    pub fn fight_limit_cm(&self, cell_cm: u16) -> u64 {
        self.loaded_weapon().map_or(0, |loaded| {
            against_sight(u64::from(loaded.max_range_cm), self.sight_cm(cell_cm))
        })
    }

    /// **The band this platform closes to**, centimetres (16b cut E.1): the same
    /// rule as [`UnitState::fight_limit_cm`] over the loaded weapon's **first**
    /// band edge ([`WeaponSummary::inner_band_cm`]).
    ///
    /// Doctrine's `engage_band_permille` is a fraction of *this*, so the stock
    /// tank stops and shoots at 420 m of its 600 m inner reach rather than at
    /// 2.1 km of a 3 km gun it cannot see that far with.
    #[must_use]
    pub fn fight_inner_cm(&self, cell_cm: u16) -> u64 {
        self.loaded_weapon().map_or(0, |loaded| {
            against_sight(u64::from(loaded.inner_band_cm()), self.sight_cm(cell_cm))
        })
    }

    /// **Whether this hull can close a range at all** (16b cut AA.1; 16c `B-76`):
    /// its motion is disabled — a destroyed engine's `disable_motion`, or both
    /// tracks under `disable_motion_when_all_of` — or it was authored with no
    /// speed, the shape of a fixed target. A wreck reads `true` too, and the
    /// controller's `Disabled` edge takes a wreck before anything asks.
    ///
    /// A zero-speed hull that is not `motion_disabled` still turns in place
    /// (`intents.rs` refuses a heading only under `motion_disabled`); it cannot
    /// close, which is the whole of what this says.
    ///
    /// Not folded into `state_hash` on its own: `limits.motion_disabled` already
    /// is, and `mobility.max_speed_cms` is a constant of the custody for the
    /// whole match, like the callsign.
    #[must_use]
    pub const fn cannot_close(&self) -> bool {
        self.limits.motion_disabled || self.mobility.max_speed_cms == 0
    }

    /// **The weapon a bare `FIRE` would fire** (16b cut F.3): [`DEFAULT_WEAPON`],
    /// else the first in component order — the one rule, so the cockpit's choice,
    /// the contact band's edges and an order's range cap cannot be three weapons.
    #[must_use]
    pub fn default_weapon(&self) -> Option<&WeaponSummary> {
        self.weapons
            .iter()
            .find(|weapon| weapon.id.as_str() == DEFAULT_WEAPON)
            .or_else(|| self.weapons.first())
    }

    /// **That weapon's authored band edges, outward**, centimetres (16b cut F.3)
    /// — what `crate::match_::cockpit::contact_band` and
    /// `crate::deliberation::order::RangeBand::ceiling_cm` are both read against,
    /// and the list a commander's brief bands its contacts with.
    ///
    /// Empty **only** for a platform that carries nothing that fires — see
    /// [`WeaponSummary::bands_cm`] for the bandless weapon.
    #[must_use]
    pub fn fire_bands_cm(&self) -> Vec<u32> {
        self.default_weapon()
            .map_or_else(Vec::new, WeaponSummary::bands_cm)
    }
}

/// **A weapon distance held down to what the platform can see** (16b cut E.1).
///
/// The one place the lesser-of rule is written: a sight of `0` is a hull with no
/// scope at all, which is not a shorter fight but no scope — it answers the
/// weapon's own figure, exactly as `crate::match_::cockpit::contact_band` does
/// for the same case.
fn against_sight(weapon_cm: u64, sight_cm: u32) -> u64 {
    if sight_cm == 0 {
        weapon_cm
    } else {
        weapon_cm.min(u64::from(sight_cm))
    }
}

// ---------------------------------------------------------------------------
// The shot
// ---------------------------------------------------------------------------

/// **One shot in flight** (16b cut W.1; doc 16 §3.5 "Scheduled impact").
///
/// Everything the re-validation at impact needs, recorded at the moment `FIRE` was
/// accepted, because that is the only moment at which it is true: doc 16 §3.5 says
/// the arena re-validates *the target exists and is not a wreck, its position is
/// within one cell of the prediction, the line is clear*, and "the prediction" is
/// [`Shot::aim_x_cm`] and [`Shot::aim_y_cm`] below.
///
/// The predicted point is the **target's pose at acceptance**, and that is
/// physics rather than knowledge: a round is thrown at a tank and not at a plot,
/// so it leaves the barrel on the line the tank is really on, and a target that
/// moves between the shot and the impact is exactly the case doc 16 §3.5 makes a
/// miss.
///
/// **It is deliberately not what the cockpit showed the pilot** (16b cut F.3).
/// The contact store does hold a position of its own — `ShadowContact::seen_at`,
/// the plot recorded at the sighting that last refreshed the contact — and every
/// number a pilot reads off a contact is computed from it, including the chance
/// `LOCK` and `WEAPONSTATUS` quote
/// (`crate::arena::resolution::Arena::resolve_seen`). For a target that has moved
/// since that sighting the two differ, which is what a stale track costs; while
/// the target is in continuous view the plot is at most one `sensors.pass_ms` old
/// and they agree. Cut S.1's justification for reading the target here — *a held
/// contact was refreshed within `sensors.pass_ms` or it would have expired* — was
/// false by a factor of `sensors.lost_after_passes`, and is not the reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shot {
    /// The shot's sequence within the match, from `1`, and the argument of its
    /// three draw labels (doc 16 §3.5).
    pub seq: u64,
    /// Who fired.
    pub shooter: ObjId,
    /// What they fired: the weapon's component id.
    pub weapon: SmolStr,
    /// The contact they were locked on.
    pub contact: ContactId,
    /// The object that contact is held on — the target, as the *record* names it.
    pub target: ObjId,
    /// How many rounds left the weapon: the burst, or what was left of it.
    pub rounds: u32,
    /// When it was fired, arena milliseconds.
    pub fired_at_ms: i64,
    /// When it arrives, arena milliseconds.
    pub impact_at_ms: i64,
    /// The range it was fired at, centimetres — the band, and the angular size of
    /// the target, are both read off this (doc 16 §3.5 steps 1 and 2).
    pub range_cm: u64,
    /// The predicted point's east coordinate, centimetres.
    pub aim_x_cm: i64,
    /// The predicted point's north coordinate, centimetres.
    pub aim_y_cm: i64,
    /// The predicted point's cell, or `None` when the target was off the window —
    /// which is what *within one cell of the prediction* is measured against.
    pub aim_cell: Option<(u16, u16)>,
    /// **What one round of this shot carries** (16b cut W.2), off the ammunition
    /// the weapon names. Recorded on the shot rather than looked up at the impact,
    /// because a turret shot off in flight takes the weapon row with it and the
    /// round already in the air still carries what it was loaded with.
    pub damage: u32,
    /// **The resolution taken at acceptance** (16b cut W.2; doc 16 §3.5's five
    /// steps), which is the moment the doc resolves at. The impact reads it and
    /// re-resolves only the angular size, and only when the target turned.
    pub resolution: Resolution,
}

// ---------------------------------------------------------------------------
// What the pilot is shown
// ---------------------------------------------------------------------------

/// **Which of a weapon's bands the platform's lock falls in** (16b cut W.1), as
/// `WEAPONSTATUS` shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameBand {
    /// The band's one-based index, nearest first.
    pub index: u32,
    /// How many bands the weapon declares, so a pilot reads "band 1 of 2".
    pub of: u32,
    /// The band's outer edge, centimetres.
    pub max_range_cm: u32,
    /// Its error cone, milliradians.
    pub dispersion_mrad: u32,
}

/// **One weapon, as the pilot's frame carries it** (16b cut W.1; doc 16 §5's status
/// block: *ammunition and ready state per weapon*).
///
/// Built per pulse from the definition row and the two live cells, so there is
/// nothing to keep in step. The world's own structural read of the M32 component
/// cells (`crate::match_::cockpit::weapons_of`) stays as the answer for a unit no
/// arena holds; while a match runs, **this** is what `WEAPONSTATUS` prints, because
/// during the match the arena is the single writer of both cells (D-16-1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameWeapon {
    /// The component id the pilot names in `FIRE <weapon>`.
    pub id: SmolStr,
    /// Whether its cycle has finished.
    pub ready: bool,
    /// How long until it has, milliseconds; `0` when it is ready.
    pub ready_in_ms: i64,
    /// Rounds left.
    pub rounds: u32,
    /// Rounds a full load carries.
    pub rounds_max: u32,
    /// **The band the platform's lock falls in for this weapon** (doc 16 §3.5 step
    /// 2), or `None` with no lock, with the lock past the last band, or on a weapon
    /// that declares no band at all.
    pub range_band: Option<FrameBand>,
    /// **The chance one round of THIS weapon would land**, per mille (16b cut
    /// F.2; doc 16 §3.5 step 5, and §5's *per weapon*), or `None` with no lock.
    ///
    /// Cut W.2 put one number on the frame — the **best** of the platform's
    /// weapons — and printed it above the rows, so a pilot who reached for the
    /// coaxial during the cannon's six-second cycle read the cannon's 46 % and
    /// fired at 21 %. Doc 16 §5 makes `WEAPONSTATUS` a per-weapon block and §3.5
    /// step 5 defines `p_permille` per **shot**, so the number belongs on the row.
    ///
    /// Resolved through the same five steps the shot itself takes, against the
    /// locked contact's own target and through the one trace
    /// [`crate::arena::resolution::LockAim`] already took — so what a pilot is
    /// shown is what a shot taken now would be drawn against.
    pub p_permille: Option<u32>,
    /// **The weapon's authored band edges, outward** (16b cut F.3), centimetres —
    /// the same list [`WeaponSummary::band_of`] walks.
    ///
    /// It is on the row because doc 16 §3.6's *range band* for a contact has to be
    /// read against something a pilot can act on, and cut S.2 read it against the
    /// longest reach the platform carried: thirds of the cannon's 3 km against a
    /// sight that sees 600 m, so every contact that has ever existed printed
    /// `close`. `crate::match_::cockpit::contact_band` reads the bands of the
    /// weapon a bare `FIRE` would fire — held down to the platform's own sight
    /// (16b cut D.1) — which is the weapon the word is advice about.
    ///
    /// Two `u32`s per weapon per pulse for a stock tank, and — for a weapon that
    /// declares no band at all — one, at its own reach
    /// ([`WeaponSummary::bands_cm`]).
    pub bands_cm: Vec<u32>,
    /// **What one shot of this weapon adds to the platform's heat** (16b cut D.2,
    /// `B-16`), and `0` for a weapon that makes none — which is every weapon of
    /// every stock tank.
    ///
    /// It is on the row so that the cockpit's own `FIRE` pre-check can skip the
    /// heat refusal for a cold gun, which is the same clause
    /// [`Arena::resolve_fire`] applies a pulse later and
    /// `crate::arena::reflex::would_overheat` applies to the machine: three
    /// readers, one rule, no drift.
    ///
    /// It is compared in [`FrameWeapon::same_as`] because it decides what the
    /// cockpit refuses, and it costs no frame anybody would not already have been
    /// sent: it is read off the content at check-out and never moves inside a
    /// match.
    pub heat_per_shot: u32,
}

impl FrameWeapon {
    /// Whether this row says the same thing as `other` — everything but
    /// [`FrameWeapon::ready_in_ms`].
    ///
    /// The countdown is left out on purpose. It is derived from `now_ms`, so it
    /// changes every pulse of a cycle, and including it would send a frame per pulse
    /// for the whole six seconds after a shot to a pilot whose scope is otherwise
    /// empty. What a pilot needs a *frame* for is the transition — ready, a round
    /// spent, a band changed — and the countdown rides whichever frame that produces.
    #[must_use]
    pub fn same_as(&self, other: &FrameWeapon) -> bool {
        self.id == other.id
            && self.ready == other.ready
            && self.rounds == other.rounds
            && self.rounds_max == other.rounds_max
            && self.range_band == other.range_band
            // **16b cut F.2**: the chance is compared, exactly as cut W.2's
            // platform-wide one was on the frame itself — a percentage that lagged
            // a tempo behind the range it is a function of would be a lie about the
            // one number a pilot decides to shoot on.
            && self.p_permille == other.p_permille
            // **16b cut D.2**: it decides whether the cockpit refuses this weapon
            // for heat, and it never moves inside a match — see the field.
            && self.heat_per_shot == other.heat_per_shot
    }
}

// ---------------------------------------------------------------------------
// The two discrete actions, and the impact
// ---------------------------------------------------------------------------

impl Arena {
    /// **Resolve `LOCK`** (16b cut W.1; doc 16 §3.7, §3.6's one refusal), in pulse
    /// phase 4.
    ///
    /// A lock is a contact **this platform holds**, resolved through
    /// [`crate::arena::state::ShadowContacts::resolve`] — the one lookup a pilot's
    /// id goes through — so a malformed id, another platform's id and an expired one
    /// are one answer here exactly as they are one sentence at the cockpit.
    ///
    /// **Idempotent**: locking what is already locked changes nothing and journals
    /// nothing, so a pilot mashing the key leaves one `locked` line in the record.
    ///
    /// Power is deliberately **not** required. Doc 16 §3.7's refusal column for
    /// `LOCK` is the one contact refusal and nothing else, and §3.6's sensor pass
    /// does not need power either: a cold tank with an intact sight can hold and name
    /// what it sees. What the cold tank cannot do is fire.
    ///
    /// # Errors
    ///
    /// [`Refusal::Unknown`] for a unit this arena does not hold, [`Refusal::Wreck`]
    /// for a wreck (doc 16 §3.3: no longer the subject of any order), and
    /// [`Refusal::NoContact`] for an id this platform does not hold.
    pub fn resolve_lock(&mut self, unit: ObjId, contact: ContactId) -> Result<(), Refusal> {
        let Some(state) = self.unit(unit) else {
            return Err(Refusal::Unknown);
        };
        if state.limits.wreck {
            return Err(Refusal::Wreck);
        }
        let held = state.lock;
        let Some(target) = self.contacts.resolve(unit, contact).map(|held| held.target) else {
            return Err(Refusal::NoContact);
        };
        if held == Some(contact) {
            return Ok(());
        }
        // The hash phase 4 took once, before any of its actions was resolved
        // ([`Arena::event_hash`]): an event says what the world looked like when it
        // happened, and everything this phase resolves happened at one moment.
        let hash = self.event_hash();
        if let Some(state) = self.unit_mut(unit) {
            state.lock = Some(contact);
        }
        let _ = self.ledger.append(
            self.now_ms,
            EventKind::Locked {
                unit,
                contact,
                target,
            },
            None,
            hash,
        );
        Ok(())
    }

    /// **Resolve `FIRE`** (16b cut W.1; doc 16 §3.5 "Acceptance and cost" and
    /// "Scheduled impact"), in pulse phase 4, answering the accepted shot's sequence.
    ///
    /// The gates, in doc 16 §3.7's own order — the weapon exists, there is a lock,
    /// the weapon is ready, there are rounds, the power is up — and then, in one
    /// step, the four things an accepted shot *is*:
    ///
    /// 1. `ready_at_ms = now + cycle_ms` (the floor of 1,000 is the content
    ///    loader's, W14);
    /// 2. `ammo` less the burst;
    /// 3. one `fired` ledger event carrying the shot's sequence, its target and its
    ///    range;
    /// 4. a [`DueKind::Impact`] at `now + range_cm × 1000 / muzzle_speed_cms`, and a
    ///    [`DueKind::WeaponReady`] at the ready time.
    ///
    /// The `WeaponReady` event changes no state: readiness is
    /// [`UnitState::weapon_ready`], a comparison against `now_ms`. What it is for is
    /// `next_due_ms` — it makes the arena step at the millisecond the weapon comes
    /// ready, so the pilot's frame says so then rather than at the next period
    /// boundary.
    ///
    /// **A last short burst rather than a refusal.** A weapon with fewer rounds left
    /// than its burst fires what it has: the round count is `min(ammo, burst_rounds)`
    /// and `Empty` is refused only at zero. Doc 16 says *decrements `ammo`* and names
    /// no rule for the remainder; firing it keeps the conservation property exact
    /// (rounds fired plus rounds left equals the load) with no stranded tail, and it
    /// is what a crew with four rounds of belt left would do.
    ///
    /// # Errors
    ///
    /// One [`Refusal`] word: [`Refusal::Unknown`], [`Refusal::Wreck`],
    /// [`Refusal::NoWeapon`], [`Refusal::NoLock`], [`Refusal::Cycling`],
    /// [`Refusal::Empty`], [`Refusal::Overheated`], [`Refusal::PowerNotOnline`].
    pub fn resolve_fire(&mut self, unit: ObjId, weapon: &str) -> Result<u64, Refusal> {
        let now_ms = self.now_ms;
        let Some(state) = self.unit(unit) else {
            return Err(Refusal::Unknown);
        };
        if state.limits.wreck {
            return Err(Refusal::Wreck);
        }
        let Some(summary) = state.weapon(weapon).cloned() else {
            return Err(Refusal::NoWeapon);
        };
        // (1) The lock. A lock whose contact this platform no longer holds is no
        // lock: doc 16 §3.5 requires *a lock on a contact the platform holds*, and
        // the `lost` arm of the pulse clears one the moment it expires — this is the
        // same rule read again at the moment of the shot, for a contact the world's
        // frame had and the arena has since let go.
        let Some(contact) = state.lock else {
            return Err(Refusal::NoLock);
        };
        let Some(target) = self.contacts.resolve(unit, contact).map(|held| held.target) else {
            return Err(Refusal::NoLock);
        };
        let Some(state) = self.unit(unit) else {
            return Err(Refusal::Unknown);
        };
        // (2) The cycle.
        if !state.weapon_ready(weapon, now_ms) {
            return Err(Refusal::Cycling);
        }
        // (3) The rounds.
        let held = state.weapon_ammo(weapon);
        if held == 0 {
            return Err(Refusal::Empty);
        }
        // (4) **16b cut X.1** (doc 16 §9 item 1: *a shot refused above a heat
        // ceiling*): the platform, not the weapon. It goes after the weapon's own
        // two refusals and before the power for the reason the module head gives —
        // a pilot is told the thing they can act on first, and the weapon's cycle
        // and its magazine are what they are looking at. A unit whose content
        // declares no heat is never refused here, which is every stock tank.
        //
        // **16b cut D.2** (`B-16`) narrowed it to the weapons that make heat. A
        // platform at its ceiling was refused every weapon it carried, including
        // the ones whose `heat_per_shot` is nought — so a laser tank that had
        // cooked itself could not fire the coaxial machine gun either, which is a
        // rule doc 16 §9 item 1 does not have: the ceiling is what a *hot* shot
        // would add to, and a cold gun adds nothing. `reflex::would_overheat`
        // answers the same question the same way, so the machine and the pilot
        // agree about whether the gun works.
        if state.overheated()
            && state
                .heat
                .as_ref()
                .is_some_and(|heat| heat.per_shot(weapon) > 0)
        {
            return Err(Refusal::Overheated);
        }
        // (5) The power, last (see the module head).
        if state.power != Power::Online {
            return Err(Refusal::PowerNotOnline);
        }
        // Accepted. The predicted point and the range are the target's pose *now*
        // — where the round is thrown, which is physics and not what the platform
        // knows (16b cut F.3; the argument is on `Shot`).
        let Some(aim) = self.unit(target).map(|state| state.pose.clone()) else {
            // A contact held on a unit that has left the match: the platform's
            // knowledge is of something that is not there, and there is nothing to
            // shoot at. Refused rather than fired, because a shot needs a point.
            return Err(Refusal::NoLock);
        };
        let range = self
            .unit(unit)
            .map(|state| range_cm(&state.pose, &aim))
            .unwrap_or(0);
        let rounds = held.min(summary.burst_rounds.max(1));
        let ready_at = now_ms.saturating_add(i64::from(summary.cycle_ms));
        let impact_at = now_ms.saturating_add(summary.flight_ms(range));
        let seq = self.next_shot_seq.saturating_add(1);
        self.next_shot_seq = seq;
        // **16b cut W.2**: doc 16 §3.5 resolves here, at acceptance. One trace, for
        // the cover the presented silhouette is scaled by; its verdict on the line
        // is the impact's to spend, because acceptance never refuses for it.
        let cover = self.fire_cover(unit, target);
        let resolution = self.resolve_shot(unit, target, weapon, range, cover);
        let hash = self.event_hash();
        if let Some(state) = self.unit_mut(unit) {
            state.ready_at_ms.insert(SmolStr::new(weapon), ready_at);
            let left = state.weapon_ammo(weapon).saturating_sub(rounds);
            state.ammo.insert(SmolStr::new(weapon), left);
            state.shots_fired = state.shots_fired.saturating_add(1);
        }
        self.shots.insert(
            seq,
            Shot {
                seq,
                shooter: unit,
                weapon: SmolStr::new(weapon),
                contact,
                target,
                rounds,
                fired_at_ms: now_ms,
                impact_at_ms: impact_at,
                range_cm: range,
                aim_x_cm: aim.x_cm,
                aim_y_cm: aim.y_cm,
                aim_cell: self.definition.terrain.cell_of(&aim),
                damage: summary.damage,
                resolution,
            },
        );
        // **16b cut X.1**: what the shot cost the platform, after the round and the
        // cycle are spent and before the shot is journalled — so the `fired` line
        // and the heat it made belong to one moment.
        self.raise_heat(unit, weapon, seq, now_ms);
        self.due
            .schedule(impact_at, DueKind::Impact { shot_seq: seq });
        self.due.schedule(
            ready_at,
            DueKind::WeaponReady {
                unit,
                weapon: SmolStr::new(weapon),
            },
        );
        let _ = self.ledger.append(
            now_ms,
            EventKind::Fired {
                unit,
                weapon: SmolStr::new(weapon),
                shot_seq: seq,
                target,
                range_cm: range,
                rounds,
            },
            None,
            hash,
        );
        Ok(seq)
    }

    /// One shot in flight, by its sequence.
    #[must_use]
    pub fn shot(&self, seq: u64) -> Option<&Shot> {
        self.shots.get(&seq)
    }

    /// **What one platform carries, as its pilots' frame carries it** (16b cut W.1),
    /// in component order.
    ///
    /// `aim` is the platform's own lock, resolved once by
    /// [`Arena::lock_aim`](crate::arena::state::Arena); `None` is no lock, and then
    /// no weapon reports a band or a chance.
    ///
    /// **16b cut F.2** widened it from the bare range to the whole aim, because a
    /// per-weapon chance needs the target and the cover as well — and resolving
    /// the one trace per platform rather than per weapon is what keeps the cost
    /// where cut W.2's single platform-wide number left it.
    ///
    /// **16b cut F.3** made every target-side input of the chance come off the
    /// aim's **plot** ([`Arena::resolve_seen`]), so a row is advice about what the
    /// platform knows and cannot move with a target it has not sighted.
    #[must_use]
    pub fn frame_weapons(
        &self,
        platform: ObjId,
        aim: Option<&crate::arena::resolution::LockAim>,
    ) -> Vec<FrameWeapon> {
        let Some(state) = self.unit(platform) else {
            return Vec::new();
        };
        let of = u32::try_from(
            state
                .weapons
                .iter()
                .map(|weapon| weapon.bands.len())
                .max()
                .unwrap_or(0),
        )
        .unwrap_or(0);
        state
            .weapons
            .iter()
            .map(|weapon| {
                let bands = u32::try_from(weapon.bands.len()).unwrap_or(of);
                FrameWeapon {
                    id: weapon.id.clone(),
                    ready: state.weapon_ready(&weapon.id, self.now_ms),
                    ready_in_ms: state.weapon_ready_in_ms(&weapon.id, self.now_ms),
                    rounds: state.weapon_ammo(&weapon.id),
                    rounds_max: weapon.count_per_load,
                    range_band: aim.and_then(|aim| {
                        weapon.band_of(aim.range_cm).map(|(index, band)| FrameBand {
                            index,
                            of: bands,
                            max_range_cm: band.max_range_cm,
                            dispersion_mrad: band.dispersion_mrad,
                        })
                    }),
                    // **16b cut F.3**: `resolve_seen` and not `resolve_shot` —
                    // what a pilot is shown is advice, and every target-side
                    // input of it comes off the contact's plot. Read live, the
                    // face and the crossing speed of a tank that has not been
                    // sighted since it drove behind a building would still move
                    // this number every pulse, and `FrameWeapon::same_as`
                    // compares it, so the pilot would be pushed a frame each time
                    // the hidden tank turned.
                    p_permille: aim
                        .map(|aim| self.resolve_seen(platform, aim, &weapon.id).p_permille),
                    // 16b cut F.3: the edges, for the contact band.
                    bands_cm: weapon.bands_cm(),
                    // 16b cut D.2: what a shot of it costs the platform, so the
                    // cockpit's own heat refusal knows which guns are cold.
                    heat_per_shot: state
                        .heat
                        .as_ref()
                        .map_or(0, |heat| heat.per_shot(&weapon.id)),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::state::{Classification, SeenPose, ShadowContact, Team};
    use crate::arena::test_support::{arena_of, cannon, pose, scout, tank};

    const RED: ObjId = ObjId(4);
    const BLUE: ObjId = ObjId(9);

    /// Two scouts four cells apart, both carrying the shipped cannon and the
    /// shipped sight.
    fn two_scouts() -> Arena {
        arena_of(vec![
            scout(RED, Team::Red, pose(1_500, 1_500)),
            scout(BLUE, Team::Blue, pose(1_500, 5_500)),
        ])
    }

    /// Hand `viewer` a classified contact on `target` and answer its id.
    fn hold(arena: &mut Arena, viewer: ObjId, target: ObjId) -> ContactId {
        let id = crate::arena::rng::draw_contact_id(arena.seed, viewer, target);
        arena.contacts.insert(
            viewer,
            id,
            ShadowContact::sighted(
                target,
                Classification::Classified { team: Team::Blue },
                0,
                // 16b cut F.3: the plot is where the target actually stands, so
                // every range these fixtures assert is unchanged.
                arena.unit(target).map(SeenPose::of).unwrap_or_default(),
            ),
        );
        id
    }

    fn online(arena: &mut Arena, unit: ObjId) {
        if let Some(state) = arena.unit_mut(unit) {
            state.power = Power::Online;
        }
    }

    #[test]
    fn a_check_out_lays_down_one_live_row_per_weapon() {
        let arena = two_scouts();
        let state = arena.unit(RED).unwrap_or_else(|| panic!("the red scout"));
        assert_eq!(state.ready_at_ms.len(), 1);
        assert_eq!(state.ammo.len(), 1);
        assert_eq!(state.weapon_ready_at_ms("cannon"), 0);
        assert_eq!(state.weapon_ammo("cannon"), cannon().count_per_load);
        assert!(state.weapon_ready("cannon", 0));
        assert_eq!(state.weapon_ready_in_ms("cannon", 0), 0);
        assert_eq!(state.weapon_reach_cm(), cannon().max_range_cm);
        // A unit with no weapon has no rows and no reach.
        let bare = arena_of(vec![tank(RED, Team::Red, pose(1_500, 1_500))]);
        let state = bare.unit(RED).unwrap_or_else(|| panic!("the bare tank"));
        assert!(state.ready_at_ms.is_empty());
        assert!(state.ammo.is_empty());
        assert_eq!(state.weapon_reach_cm(), 0);
        assert_eq!(state.weapon_ammo("cannon"), 0);
    }

    /// **16b cut F.3**: the band word and a squad order's engage ceiling are both
    /// read against the fired weapon's band edges, so a weapon that declares a
    /// reach and authors no bands must not answer an empty list. An empty one
    /// makes `contact_band` say `beyond` at point blank and
    /// `RangeBand::ceiling_cm` nought — and a nought ceiling stops an ordered
    /// reflex unit ever reaching `Engage` and puts its stand-off goal on the
    /// target's own cell. The empty list belongs to a platform with nothing that
    /// fires, and to nothing else.
    ///
    /// **16b cut E.1** re-based both readers on the platform's own sight, so the
    /// ceilings below are thirds of the scout's 600 m scope and not of the
    /// bandless gun's 2.5 km, and [`UnitState::fight_inner_cm`] reads the one
    /// band the weapon does declare.
    #[test]
    fn a_weapon_with_a_reach_and_no_bands_is_one_band_at_that_reach() {
        use crate::deliberation::order::RangeBand;

        let bandless = WeaponSummary {
            bands: Vec::new(),
            ..cannon()
        };
        assert_eq!(bandless.bands_cm(), vec![250_000]);
        assert_eq!(bandless.inner_band_cm(), 250_000);
        assert_eq!(cannon().bands_cm(), vec![150_000, 250_000]);
        assert_eq!(cannon().inner_band_cm(), 150_000);

        let mut arena = two_scouts();
        if let Some(state) = arena.unit_mut(RED) {
            state.weapons = vec![bandless];
        }
        let state = arena.unit(RED).unwrap_or_else(|| panic!("the red scout"));
        let edges = state.fire_bands_cm();
        assert_eq!(edges, vec![250_000]);
        // The scout's own sight: 60 cells of 1,000 cm.
        let sight = state.sight_cm(1_000);
        assert_eq!(sight, 60_000);
        // Both consumers, at a range a tank really fights at.
        assert_eq!(
            crate::match_::cockpit::contact_band(4_000, &edges, sight),
            "close"
        );
        assert_eq!(RangeBand::Effective.ceiling_cm(&edges, sight), 40_000);
        assert_ne!(RangeBand::Close.ceiling_cm(&edges, sight), 0);
        // And a bandless weapon's inner band is its reach, held to the sight.
        assert_eq!(state.fight_inner_cm(1_000), 60_000);
        assert_eq!(state.fight_limit_cm(1_000), 60_000);

        // And the empty list is still a platform that carries nothing that fires.
        let bare = arena_of(vec![tank(RED, Team::Red, pose(1_500, 1_500))]);
        let state = bare.unit(RED).unwrap_or_else(|| panic!("the bare tank"));
        assert!(state.fire_bands_cm().is_empty());
        assert_eq!(
            crate::match_::cockpit::contact_band(0, &[], sight),
            "beyond"
        );
        assert_eq!(RangeBand::Effective.ceiling_cm(&[], sight), 0);
        assert_eq!(state.loaded_weapon().map(|weapon| weapon.id.clone()), None);
        assert_eq!(state.fight_inner_cm(1_000), 0);
        assert_eq!(state.fight_limit_cm(1_000), 0);
    }

    /// **The four readings 16b cut E.1 prices the machine off** (16c `T-46`,
    /// `T-47`), on a platform that carries two guns.
    ///
    /// The claim that matters is the dry one: `default_weapon` is the cockpit's
    /// bare-`FIRE` rule and ignores ammunition on purpose, and
    /// [`UnitState::loaded_weapon`] is the same list read against the magazines —
    /// so a tank whose cannon is empty fights at its second gun's band instead of
    /// holding a distance it has nothing to send to.
    #[test]
    fn e1_the_fighting_reach_is_the_lesser_of_the_sight_and_the_loaded_weapon() {
        let short = WeaponSummary {
            id: SmolStr::new("carbine"),
            max_range_cm: 20_000,
            bands: vec![WeaponBand {
                max_range_cm: 8_000,
                dispersion_mrad: 4,
            }],
            ..cannon()
        };
        let mut arena = two_scouts();
        if let Some(state) = arena.unit_mut(RED) {
            state.weapons = vec![cannon(), short];
            state.ammo.insert(SmolStr::new("carbine"), 30);
        }
        let Some(state) = arena.unit(RED) else {
            panic!("the red scout")
        };
        // Loaded: the cannon, whose inner band is 1.5 km — held down to the
        // scout's 600 m scope, which is the whole of cut E.1's ruling.
        assert_eq!(
            state.loaded_weapon().map(|weapon| weapon.id.to_string()),
            Some("cannon".to_owned())
        );
        assert_eq!(state.sight_cm(1_000), 60_000);
        assert_eq!(state.fight_inner_cm(1_000), 60_000);
        assert_eq!(state.fight_limit_cm(1_000), 60_000);
        assert_eq!(
            state.weapon_reach_cm(),
            250_000,
            "the longest gun is still the longest gun: the alert derivation reads it"
        );

        // Dry cannon: the carbine's own 80 m band, which is inside the sight.
        if let Some(state) = arena.unit_mut(RED) {
            state.ammo.insert(SmolStr::new("cannon"), 0);
        }
        let Some(state) = arena.unit(RED) else {
            panic!("the red scout")
        };
        assert_eq!(
            state.default_weapon().map(|weapon| weapon.id.to_string()),
            Some("cannon".to_owned()),
            "a bare FIRE still means the cannon, and still refuses `Empty`"
        );
        assert_eq!(
            state.loaded_weapon().map(|weapon| weapon.id.to_string()),
            Some("carbine".to_owned())
        );
        assert_eq!(state.fight_inner_cm(1_000), 8_000);
        assert_eq!(state.fight_limit_cm(1_000), 20_000);

        // Both dry: nothing loaded, no reach, and `best_hostile` has no target.
        if let Some(state) = arena.unit_mut(RED) {
            state.ammo.insert(SmolStr::new("carbine"), 0);
        }
        let Some(state) = arena.unit(RED) else {
            panic!("the red scout")
        };
        assert!(state.loaded_weapon().is_none());
        assert_eq!(state.fight_limit_cm(1_000), 0);

        // And a hull with no scope at all fights at the weapon's own figure: a
        // sight of nought is no scope, not no reach.
        if let Some(state) = arena.unit_mut(RED) {
            state.ammo.insert(SmolStr::new("cannon"), 40);
            state.sensor = None;
        }
        let Some(state) = arena.unit(RED) else {
            panic!("the red scout")
        };
        assert_eq!(state.sight_cm(1_000), 0);
        assert_eq!(state.fight_inner_cm(1_000), 150_000);
        assert_eq!(state.fight_limit_cm(1_000), 250_000);
    }

    /// **16b cut AA.1 (16c `B-76`)**: a hull that cannot close is one whose
    /// motion damage has disabled it, or one authored with no speed at all — and
    /// nothing else. The scout drives, so it reads `false`; each of the two
    /// facts alone turns it `true`.
    #[test]
    fn aa1_a_hull_cannot_close_when_its_motion_is_disabled_or_it_has_no_speed() {
        let mut arena = two_scouts();
        let Some(state) = arena.unit(RED) else {
            panic!("the red scout")
        };
        assert_ne!(state.mobility.max_speed_cms, 0);
        assert!(!state.limits.motion_disabled);
        assert!(!state.cannot_close(), "a scout that drives can close");

        // A destroyed engine, the way `damage.rs`'s `disable_motion` writes it.
        if let Some(state) = arena.unit_mut(RED) {
            state.limits.motion_disabled = true;
        }
        let Some(state) = arena.unit(RED) else {
            panic!("the red scout")
        };
        assert!(state.cannot_close(), "a dead engine cannot close");

        // An authored zero-speed hull, whose motion is not disabled.
        let mut arena = two_scouts();
        if let Some(state) = arena.unit_mut(RED) {
            state.mobility.max_speed_cms = 0;
        }
        let Some(state) = arena.unit(RED) else {
            panic!("the red scout")
        };
        assert!(!state.limits.motion_disabled);
        assert!(state.cannot_close(), "a hull with no speed cannot close");
    }

    #[test]
    fn the_bands_run_outward_and_a_beam_arrives_at_once() {
        let weapon = cannon();
        // The fixture's two bands: 1.5 km at 3 mrad, 2.5 km at 6 mrad.
        assert_eq!(
            weapon
                .band_of(0)
                .map(|(index, band)| (index, band.dispersion_mrad)),
            Some((1, 3))
        );
        assert_eq!(
            weapon
                .band_of(150_000)
                .map(|(index, band)| (index, band.dispersion_mrad)),
            Some((1, 3))
        );
        assert_eq!(
            weapon
                .band_of(150_001)
                .map(|(index, band)| (index, band.dispersion_mrad)),
            Some((2, 6))
        );
        assert_eq!(weapon.band_of(250_001), None, "past the last band");
        // The flight time, and the beam's absence of one.
        assert!(!weapon.is_beam());
        assert_eq!(
            weapon.flight_ms(145_000),
            1_000,
            "the muzzle speed's own distance in one second"
        );
        assert_eq!(weapon.flight_ms(0), 0);
        let beam = WeaponSummary {
            muzzle_speed_cms: 0,
            ..cannon()
        };
        assert!(beam.is_beam());
        assert_eq!(beam.flight_ms(250_000), 0);
    }

    #[test]
    fn lock_holds_a_contact_the_platform_holds_and_refuses_every_other_id() {
        let mut arena = two_scouts();
        // An id nobody minted.
        assert_eq!(
            arena.resolve_lock(RED, ContactId(1)),
            Err(Refusal::NoContact)
        );
        assert_eq!(arena.unit(RED).and_then(|state| state.lock), None);
        // Another platform's id is the same answer.
        let blues = hold(&mut arena, BLUE, RED);
        assert_eq!(arena.resolve_lock(RED, blues), Err(Refusal::NoContact));
        // The platform's own id is a lock, and one `locked` line.
        let id = hold(&mut arena, RED, BLUE);
        let before = arena.ledger.seq();
        assert_eq!(arena.resolve_lock(RED, id), Ok(()));
        assert_eq!(arena.unit(RED).and_then(|state| state.lock), Some(id));
        assert_eq!(arena.ledger.seq(), before.saturating_add(1));
        // Idempotent: the same lock again journals nothing.
        assert_eq!(arena.resolve_lock(RED, id), Ok(()));
        assert_eq!(arena.ledger.seq(), before.saturating_add(1));
        // A wreck is the subject of no order.
        if let Some(state) = arena.unit_mut(RED) {
            state.limits.wreck = true;
        }
        assert_eq!(arena.resolve_lock(RED, id), Err(Refusal::Wreck));
        assert_eq!(arena.resolve_lock(ObjId(77), id), Err(Refusal::Unknown));
    }

    #[test]
    fn fire_refuses_in_doc_16_s_order_and_an_accepted_shot_costs_its_round_and_its_cycle() {
        let mut arena = two_scouts();
        // No weapon by that name, before anything else about the shot.
        assert_eq!(arena.resolve_fire(RED, "trebuchet"), Err(Refusal::NoWeapon));
        // No lock, while the tank is also cold: the lock is the answer (doc 16
        // §3.7's column order puts power last).
        assert_eq!(arena.resolve_fire(RED, "cannon"), Err(Refusal::NoLock));
        let id = hold(&mut arena, RED, BLUE);
        assert_eq!(arena.resolve_lock(RED, id), Ok(()));
        // Cycling, still cold.
        if let Some(state) = arena.unit_mut(RED) {
            state.ready_at_ms.insert(SmolStr::new("cannon"), 1);
        }
        assert_eq!(arena.resolve_fire(RED, "cannon"), Err(Refusal::Cycling));
        // Empty, ready, still cold.
        if let Some(state) = arena.unit_mut(RED) {
            state.ready_at_ms.insert(SmolStr::new("cannon"), 0);
            state.ammo.insert(SmolStr::new("cannon"), 0);
        }
        assert_eq!(arena.resolve_fire(RED, "cannon"), Err(Refusal::Empty));
        // Loaded, ready, locked — and cold.
        if let Some(state) = arena.unit_mut(RED) {
            state.ammo.insert(SmolStr::new("cannon"), 40);
        }
        assert_eq!(
            arena.resolve_fire(RED, "cannon"),
            Err(Refusal::PowerNotOnline)
        );
        // Online, and the shot is accepted.
        online(&mut arena, RED);
        let seq = arena
            .resolve_fire(RED, "cannon")
            .unwrap_or_else(|_| panic!("the shot is accepted"));
        assert_eq!(seq, 1, "the first shot of a match is 1");
        let state = arena.unit(RED).unwrap_or_else(|| panic!("the red scout"));
        assert_eq!(state.weapon_ammo("cannon"), 39, "one round, one burst");
        assert_eq!(
            state.weapon_ready_at_ms("cannon"),
            i64::from(cannon().cycle_ms)
        );
        assert!(!state.weapon_ready("cannon", 0));
        // And the shot is in flight, at the range it was fired at.
        let shot = arena
            .shot(seq)
            .cloned()
            .unwrap_or_else(|| panic!("the shot"));
        assert_eq!((shot.shooter, shot.target, shot.rounds), (RED, BLUE, 1));
        assert_eq!(shot.range_cm, 4_000, "four cells of 1,000 cm");
        assert_eq!((shot.aim_x_cm, shot.aim_y_cm), (1_500, 5_500));
        assert_eq!(shot.impact_at_ms, cannon().flight_ms(4_000));
        // The cycle refuses the next one.
        assert_eq!(arena.resolve_fire(RED, "cannon"), Err(Refusal::Cycling));
    }

    #[test]
    fn a_last_short_burst_is_fired_and_the_rounds_are_conserved() {
        let mut arena = two_scouts();
        let id = hold(&mut arena, RED, BLUE);
        let _ = arena.resolve_lock(RED, id);
        online(&mut arena, RED);
        // A ten-round burst with four rounds left fires four.
        if let Some(state) = arena.unit_mut(RED) {
            state.weapons = vec![WeaponSummary {
                burst_rounds: 10,
                count_per_load: 4,
                ..cannon()
            }];
            state.ammo.insert(SmolStr::new("cannon"), 4);
        }
        let seq = arena
            .resolve_fire(RED, "cannon")
            .unwrap_or_else(|_| panic!("the shot is accepted"));
        assert_eq!(arena.shot(seq).map(|shot| shot.rounds), Some(4));
        assert_eq!(
            arena.unit(RED).map(|state| state.weapon_ammo("cannon")),
            Some(0)
        );
        // And at zero it is empty.
        if let Some(state) = arena.unit_mut(RED) {
            state.ready_at_ms.insert(SmolStr::new("cannon"), 0);
        }
        assert_eq!(arena.resolve_fire(RED, "cannon"), Err(Refusal::Empty));
    }

    #[test]
    fn an_impact_journals_its_arrival_and_then_exactly_one_outcome() {
        let mut arena = two_scouts();
        let id = hold(&mut arena, RED, BLUE);
        let _ = arena.resolve_lock(RED, id);
        online(&mut arena, RED);
        let seq = arena
            .resolve_fire(RED, "cannon")
            .unwrap_or_else(|_| panic!("the shot is accepted"));
        let mut journal: Vec<crate::arena::perception::Notice> = Vec::new();
        arena.resolve_impact(seq, 100, &mut journal);
        assert_eq!(journal.len(), 2, "the arrival and its outcome: {journal:?}");
        assert!(matches!(
            journal.first().map(|(_, kind, _)| kind),
            Some(EventKind::Impact { .. })
        ));
        // These scouts carry no authored silhouette, so the shot resolves at zero
        // and the outcome is a miss — which is cut W.2's arithmetic and not a
        // placeholder (see `crate::arena::resolution`).
        assert!(matches!(
            journal.get(1).map(|(_, kind, _)| kind),
            Some(EventKind::Miss { .. })
        ));
        assert!(arena.shot(seq).is_none(), "the shot left the table");
        // A second arrival for the same shot says nothing.
        journal.clear();
        arena.resolve_impact(seq, 200, &mut journal);
        assert!(journal.is_empty());
    }

    #[test]
    fn the_frame_row_reports_readiness_rounds_and_the_locks_band() {
        let mut arena = two_scouts();
        let rows = arena.frame_weapons(RED, None);
        assert_eq!(rows.len(), 1);
        let row = rows
            .first()
            .cloned()
            .unwrap_or_else(|| panic!("the cannon"));
        assert!(row.ready && row.ready_in_ms == 0);
        assert_eq!((row.rounds, row.rounds_max), (40, 40));
        assert_eq!(row.range_band, None, "no lock, no band");
        assert_eq!(row.p_permille, None, "and no chance to quote");
        // A lock inside the first band, and one past the last. **16b cut F.2**:
        // the aim carries the target and the cover as well as the range, because
        // each weapon resolves its own chance against them.
        // **16b cut F.3** put the plot on it too, because the advertised chance
        // reads the face and the crossing speed off the sighting; here the plot is
        // where the blue tank actually stands.
        let plot = arena.unit(BLUE).map(SeenPose::of).unwrap_or_default();
        let aim = move |range_cm: u64| crate::arena::resolution::LockAim {
            target: BLUE,
            seen_at: plot,
            range_cm,
            cover: mush_atlas::Cover::None,
        };
        let inside = arena
            .frame_weapons(RED, Some(&aim(4_000)))
            .first()
            .and_then(|row| row.range_band)
            .unwrap_or_else(|| panic!("a band"));
        assert_eq!((inside.index, inside.of), (1, 2));
        assert_eq!(
            arena
                .frame_weapons(RED, Some(&aim(400_000)))
                .first()
                .and_then(|row| row.range_band),
            None,
            "past the last band"
        );
        // Cycling, after a shot.
        let id = hold(&mut arena, RED, BLUE);
        let _ = arena.resolve_lock(RED, id);
        online(&mut arena, RED);
        let _ = arena.resolve_fire(RED, "cannon");
        let row = arena
            .frame_weapons(RED, Some(&aim(4_000)))
            .first()
            .cloned()
            .unwrap_or_else(|| panic!("the cannon"));
        assert!(!row.ready);
        assert_eq!(row.ready_in_ms, i64::from(cannon().cycle_ms));
        assert_eq!(row.rounds, 39);
        // The countdown alone is not a change a pilot is sent a frame for.
        let later = FrameWeapon {
            ready_in_ms: row.ready_in_ms.saturating_sub(500),
            ..row.clone()
        };
        assert!(row.same_as(&later));
        let spent = FrameWeapon {
            rounds: 38,
            ..row.clone()
        };
        assert!(!row.same_as(&spent));
    }
}
