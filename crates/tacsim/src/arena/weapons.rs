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

    /// **The longest reach anything the unit carries has**, centimetres — the
    /// number `crate::arena::step::Frame::weapon_reach_cm` carries and
    /// `tempo.alert_range_cm` is derived from (16b §9). Zero for a unit that carries
    /// no weapon, and then nothing is in range of it.
    #[must_use]
    pub fn weapon_reach_cm(&self) -> u32 {
        self.weapons
            .iter()
            .map(|weapon| weapon.max_range_cm)
            .max()
            .unwrap_or(0)
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
/// The predicted point is the **target's pose at acceptance**, which is doc 16
/// §3.6's *the contact's position is what the platform knows*: the shadow contact
/// store keeps no position of its own (it is a lifecycle, not a track), and a held
/// contact was refreshed within `sensors.pass_ms` or it would have expired — so the
/// target's pose at acceptance *is* the platform's knowledge of where it is, and a
/// target that moves between the shot and the impact is exactly the case doc 16
/// §3.5 makes a miss.
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
        if state.overheated() {
            return Err(Refusal::Overheated);
        }
        // (5) The power, last (see the module head).
        if state.power != Power::Online {
            return Err(Refusal::PowerNotOnline);
        }
        // Accepted. The predicted point and the range are the target's pose *now*,
        // which is what the platform knows of it (doc 16 §3.6).
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
                    p_permille: aim.map(|aim| {
                        self.resolve_shot(platform, aim.target, &weapon.id, aim.range_cm, aim.cover)
                            .p_permille
                    }),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::state::{Classification, ShadowContact, Team};
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
            ShadowContact {
                target,
                class: Classification::Classified { team: Team::Blue },
                first_seen_ms: 0,
                last_seen_ms: 0,
            },
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
        let aim = |range_cm: u64| crate::arena::resolution::LockAim {
            target: BLUE,
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
