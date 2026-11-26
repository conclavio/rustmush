//! **A unit leaving a running field at its carrier's ramp** (doc 32 cut CF.5; Q-CF-18).
//!
//! The kernel had no way for a unit to leave a running arena: a unit stayed until the end, and
//! its check-in rode [`crate::arena_task::ArenaOut::Ended`]. Recovery as exit needs one. The IC
//! field host sends [`crate::arena_task::ArenaMsg::Withdraw`] with the ramp's site position; at
//! the next pulse's head, before the step, [`Arena::withdraw`] decides it on the arena's own
//! facts — the unit's live pose and its live damage, which only the arena holds while it runs —
//! and either refuses with a [`WithdrawRefusal`] or takes the unit out and answers its check-in
//! row, exactly the row the end would have carried ([`Arena::returned_units`]).
//!
//! **A carrier lifting off its pad** (doc 32 F-6, ruled 2026-10-31; built in cut CF.7) asks with
//! no ramp. After the pilot's leave-behind confirmation the carrier is removed from the field
//! outright and lifts: it stands on its pad, about four cells from its own ramp, and a lift is
//! not a drive, so neither test below applies to it. What a hit does to lift is F-3's.
//!
//! **The rule** (recorded in doc 32 §8): the unit stands **beside the ramp** — its cell within
//! one cell of the ramp's, Chebyshev, the surface recover's own rule (doc 20 SM.3b) and the DT
//! recover's (`at_ramp`) — and it can **return under its own power**: no wreck, motion not
//! disabled, not disabled, and its anatomy holds power and control and, where it has a mobility
//! profile, locomotion (the DT recover's own `recover_unit` test, read here on the live cells so
//! the recover the host submits next cannot refuse what the kernel let go).
//!
//! **A carrier lost** (doc 40, the ramp exit's boarding window; the owner's ruling, 2026-11-04,
//! "back in the field"): a withdrawal counts only once the unit is aboard, so a raider's hull,
//! whose host names the carrier it boards ([`Arena::withdraw_to`]), is kept while that carrier is
//! a wreck or no longer in the field — [`WithdrawRefusal::CarrierLost`], decided on the arena's
//! own live facts at the same pulse head as the release, so no wreck can land between the two.
//!
//! **What leaves with it.** The unit's row, its seats, its mind and its runner, its motor row,
//! its queued orders and card changes, its jamming fold; the spatial index is rebuilt. Every
//! other reader already treats a unit "gone from the match" as gone — the frame drops a contact
//! row whose target left, an order watch on it lapses, a catch-up names neither — so the
//! sensors lose it at their next pass, which is what a hull driving up a ramp looks like.
//! Nothing is journalled: like a leader's mark, the exit is the host's to record (its check-in
//! and the site record), and the black box keeps the pulses either side of it.

use mush_core::ObjId;

use crate::arena::damage::UnitReturn;
use crate::arena::state::Arena;

/// **Why the arena kept a unit the host asked to withdraw.**
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WithdrawRefusal {
    /// The arena holds no such unit (it left already, or never came).
    NotHere,
    /// The unit cannot return under its own power: a wreck, motion disabled, disabled, or its
    /// anatomy short of power, control or locomotion.
    NotUnderOwnPower,
    /// The unit does not stand within one cell of the ramp.
    NotAtRamp,
    /// The carrier it would board is a wreck or no longer in the field (this module's head):
    /// there is nothing to go aboard, so it stays in the field.
    CarrierLost,
    /// **The ask was recalled** (doc 44 cut MW.7; Q-44-37): the host asked the unit out for a
    /// player who was away, and that player took the controls back before the pulse that
    /// would have let it go. Answered by the task, never by [`Arena::withdraw`].
    Recalled,
    /// **Its mind stands a posture** (doc 45 unit AP.6; Q-45-7): the unit is flown for an
    /// absent player on `GUARD` or `HOLD`, which stay where they stand and are never taken
    /// home by the hand-over. Its player, back at the controls, recovers it as ever; a
    /// posture set to `RETURN` sends it home.
    Standing,
}

impl WithdrawRefusal {
    /// The refusal's line to whoever asked.
    #[must_use]
    pub const fn sentence(self) -> &'static str {
        match self {
            WithdrawRefusal::NotHere => "That unit is not in the field.",
            WithdrawRefusal::NotUnderOwnPower => {
                "That unit cannot come in under its own power; it stays in the field."
            }
            WithdrawRefusal::NotAtRamp => {
                "That unit is not beside the ramp; drive it there first, then recover it."
            }
            WithdrawRefusal::CarrierLost => {
                "That unit's carrier is lost; there is nothing to go aboard, so it stays in the field."
            }
            WithdrawRefusal::Recalled => {
                "That unit's pilot is back at its controls; it stays in the field."
            }
            WithdrawRefusal::Standing => {
                "That unit's mind stands its posture where it is while its player is away; RETURN brings it in."
            }
        }
    }
}

/// **Whether `state` can return under its own power** (this module's head): no wreck, motion
/// not disabled, not disabled, and its anatomy holds power and control and, where it has a
/// mobility profile, locomotion.
fn under_own_power(state: &crate::arena::state::UnitState) -> bool {
    !state.limits.wreck
        && !state.limits.motion_disabled
        && !state.disabled
        && crate::field_ops::assess_tacsim_unit(&state.unit, None).is_ok_and(|c| {
            c.has_power && c.has_control && (c.can_move || state.unit.mobility().is_none())
        })
}

/// Cells apart, Chebyshev.
fn cells_apart(a: (u16, u16), b: (u16, u16)) -> u16 {
    a.0.abs_diff(b.0).max(a.1.abs_diff(b.1))
}

impl Arena {
    /// **Whether a unit is still on its way home** (the field-close branch of doc 36a lane 3): a
    /// unit its crew is driving home: its mind holds a WITHDRAW, it could still come in under its
    /// own power, and no absent player stands behind it. While one is, a quiet field has not
    /// disengaged: the raid that turned for home is still leaving, and a close now would stand its
    /// hull on the ground outside any field, neither aboard nor on the site record. A hull its
    /// kernel keeps ([`WithdrawRefusal::CarrierLost`]) has its WITHDRAW lifted by the host, and one
    /// that can no longer drive is not under way, so neither holds the field; the idle bound
    /// (`[tacsim.field].idle_demote_s`) still closes a field whose way home is blocked.
    ///
    /// An absent player's unit is not counted (doc 44 cut MW.7): its house mind lays every such
    /// unit a WITHDRAW, a rig on its own pad among them, which has no ramp to reach, and a close
    /// checks it in where it stands as ever.
    #[must_use]
    pub fn homeward_under_way(&self) -> bool {
        self.units.iter().any(|state| {
            !state.absent
                && self
                    .minds
                    .get(&state.id)
                    .and_then(|row| row.order.as_ref())
                    .is_some_and(|order| {
                        order.objective == crate::arena::perception::MindObjective::Withdraw
                    })
                && under_own_power(state)
        })
    }

    /// **Take `unit` out of the field at the ramp standing at `ramp` (cm)**, answering its
    /// check-in row (this module's head). `None` is a carrier lifting off its pad (doc 32 F-6):
    /// no ramp and no drive, so only [`WithdrawRefusal::NotHere`] refuses it.
    ///
    /// # Errors
    ///
    /// [`WithdrawRefusal`], and the arena is unchanged.
    pub fn withdraw(
        &mut self,
        unit: ObjId,
        ramp: Option<(i64, i64)>,
    ) -> Result<UnitReturn, WithdrawRefusal> {
        self.withdraw_to(unit, ramp, None)
    }

    /// **[`Arena::withdraw`], boarding `carrier`** (this module's head, "a carrier lost"): with a
    /// ramp and a carrier named, the unit is kept with [`WithdrawRefusal::CarrierLost`] while
    /// that carrier is a wreck or not in the field, wherever the unit stands.
    ///
    /// # Errors
    ///
    /// [`WithdrawRefusal`], and the arena is unchanged.
    pub fn withdraw_to(
        &mut self,
        unit: ObjId,
        ramp: Option<(i64, i64)>,
        carrier: Option<ObjId>,
    ) -> Result<UnitReturn, WithdrawRefusal> {
        let state = self.unit(unit).ok_or(WithdrawRefusal::NotHere)?;
        let Some(ramp) = ramp else {
            return self.take_out(unit);
        };
        if let Some(carrier) = carrier
            && self
                .unit(carrier)
                .is_none_or(|carrier| carrier.limits.wreck)
        {
            return Err(WithdrawRefusal::CarrierLost);
        }
        // **Doc 40 cut FR.5** (§4.1 item 5, Q-40-25): a casualty under tow cannot come in under
        // its own power, so it is judged by its tug — the tug in the field, under its own power
        // and beside the ramp — and goes out in tow with its own check-in row. The task applies
        // a casualty's withdrawal before its tug's, so the tug still stands here to be read.
        // **Doc 45 unit AP.6** (Q-45-7): a unit its mind stands on `GUARD` or `HOLD` for an
        // absent player stays. The host asks every absent player's unit out once a second
        // and the kernel decides here, on the posture the unit is flown on now: a `GUARD`
        // that can no longer defend itself is `RETURN` and is let go like one. A casualty
        // under tow is its tug's to take out.
        // **Doc 45 unit AP.7**: and it is judged by its tug here too. A tug nobody sits in is
        // an absent player's for the whole field; were the casualty let out on the tug's pose
        // and the tug then kept for its posture, a load typed by the tug's player would split
        // the pair. Both stay, with the one refusal.
        let judged = match state.coupled_to {
            Some(tug) => self.unit(tug).ok_or(WithdrawRefusal::NotUnderOwnPower)?,
            None => state,
        };
        if judged.absent && !crate::arena::absent::flown(judged).is_return() {
            return Err(WithdrawRefusal::Standing);
        }
        if !under_own_power(judged) {
            return Err(WithdrawRefusal::NotUnderOwnPower);
        }
        if !self.beside(judged, ramp) {
            return Err(WithdrawRefusal::NotAtRamp);
        }
        self.take_out(unit)
    }

    /// Whether `state` stands within one cell of the ramp at `ramp` (cm), Chebyshev.
    fn beside(&self, state: &crate::arena::state::UnitState, ramp: (i64, i64)) -> bool {
        let terrain = &self.definition.terrain;
        match (
            terrain.cell_of(&state.pose),
            terrain.cell_at(ramp.0, ramp.1),
        ) {
            (Some(here), Some(at)) => cells_apart(here, at) <= 1,
            _ => false,
        }
    }

    /// The unit's check-in row, and the unit out of the arena.
    fn take_out(&mut self, unit: ObjId) -> Result<UnitReturn, WithdrawRefusal> {
        let back = self
            .returned_units()
            .into_iter()
            .find(|row| row.unit == unit)
            .ok_or(WithdrawRefusal::NotHere)?;
        self.remove_unit(unit);
        Ok(back)
    }

    /// Garbage-collect `self.cards` (`CardCatalog`), retaining only digests
    /// referenced by live profiles, unit seals, or pending/in-flight thinks
    /// and recomputing `encoded_bytes` (P2-TSC-14).
    pub(crate) fn gc_cards(&mut self) {
        let mut live = std::collections::BTreeSet::new();
        for runtime in self.profiles.values() {
            live.extend(runtime.profiles.digests());
            live.insert(runtime.card.key().definition);
            live.insert(runtime.rom.key().definition);
        }
        for state in &self.units {
            live.extend(state.seal.profiles.digests());
            live.insert(state.seal.rom.definition);
        }
        for mind in self.minds.values() {
            if let Some(pending) = &mind.pending {
                live.insert(pending.profile_token.key.definition);
            }
        }
        for think in self.collected.iter().chain(self.unsettled.iter()) {
            live.insert(think.profile_token.key.definition);
        }
        self.cards.retain_referenced(&live);
    }

    /// **Drop `unit` from the arena** with no check: its row and everything the arena keeps for
    /// it alone (this module's head). A boot resuming a field from a snapshot taken before a
    /// unit's exit uses it too, since that unit has already been checked in.
    pub fn remove_unit(&mut self, unit: ObjId) {
        use crate::arena::state::{ChassisWords, DueKind, WithdrawnPlot};
        use std::collections::BTreeSet;

        let removed_pilots: Vec<ObjId> = self
            .pilots
            .iter()
            .filter(|pilot| pilot.unit == unit)
            .map(|pilot| pilot.pilot)
            .collect();
        // Clear the departing unit's own viewer contacts first so its own tracks
        // never keep tombstones alive.
        self.contacts.clear_viewer(unit);
        // P2-TSC-15: if any surviving viewer still holds a stale plot on `unit`,
        // snapshot static display & fire-control metadata into `withdrawn_plots`
        // so stale plots render, compute odds/range/cover/deliberation coords,
        // and expire naturally at `contact_ttl_ms` without leaking withdrawal.
        if self.contacts.has_target(unit)
            && let Some(at) = self.unit_at(unit)
            && let Some(state) = self.units.get(at)
        {
            let words = self
                .chassis
                .get(at)
                .cloned()
                .unwrap_or_else(|| ChassisWords::of(&state.unit, &state.profile));
            self.withdrawn_plots.insert(
                unit,
                WithdrawnPlot {
                    callsign: state.callsign,
                    armed: state.armed_at_checkout() || state.shots_fired > 0,
                    sensor_height_cm: state.sensor_height_cm,
                    fatal_mask: words.fatal_mask,
                    fatal_words: words.fatal_words,
                    section_names: words.section_names,
                    unit_type: words.unit_type,
                    profile: state.profile.clone(),
                },
            );
        }
        self.units.retain(|state| state.id != unit);
        self.pilots.retain(|pilot| pilot.unit != unit);
        self.epochs.remove_unit(unit);
        self.lost_designators
            .retain(|(viewer, _), _| *viewer != unit);
        self.held_losses.retain(|(viewer, _), _| *viewer != unit);
        self.held_loss_targets
            .retain(|(viewer, _), _| *viewer != unit);
        self.spent_said
            .retain(|(told, target, _), _| *told != unit && *target != unit);
        self.motors.release(unit);
        self.minds.remove(&unit);
        self.runners.remove(&unit);
        self.profiles.remove(&unit);
        self.conditions.clear(unit);
        self.held_said
            .retain(|(on, _, cid), _| *on != unit && self.contacts.get(*on, *cid).is_some());
        self.detachment_commands.remove(&unit);
        self.purge_order_watches_for_unit(unit);
        self.pending.remove(&unit);
        self.pending_cards.remove(&unit);
        self.awaiting.remove(&unit);
        self.collected.retain(|think| think.unit != unit);
        self.unsettled.retain(|think| think.unit != unit);
        self.traces.remove_unit(unit);
        self.pending_notices.remove(&unit);
        self.at_release.remove(&unit);
        for pilot in removed_pilots {
            self.pending_catchup.remove(&pilot);
            self.last_frames.remove(&pilot);
        }
        // P2-TSC-14: retain only live unit IDs in surviving units' `fired_on_by`.
        let live_ids: BTreeSet<ObjId> = self.units.iter().map(|state| state.id).collect();
        for surviving in &mut self.units {
            surviving.fired_on_by.retain(|id| live_ids.contains(id));
        }
        // P2-TSC-15: prune any tombstones no longer referenced by any viewer.
        self.withdrawn_plots
            .retain(|target, _| self.contacts.has_target(*target));
        // P2-TSC-14: prune withdrawn viewer entries (and unreferenced withdrawn
        // target entries) from `DesignatorBook`.
        let mut keep_targets = BTreeSet::new();
        for (viewer, _, held) in self.contacts.all() {
            if held.target == unit {
                keep_targets.insert((viewer, unit));
            }
        }
        for (&(viewer, _), &target) in &self.held_loss_targets {
            if target == unit {
                keep_targets.insert((viewer, unit));
            }
        }
        self.designators.prune_unit(unit, &keep_targets);
        // P2-TSC-14: mark-and-sweep `CardCatalog` for definitions unreferenced by
        // any surviving unit/profile/pending think.
        self.gc_cards();
        self.due.retain(|kind| match kind {
            DueKind::RuleLeaseExpired { unit: on, .. }
            | DueKind::StartupComplete { unit: on }
            | DueKind::WeaponReady { unit: on, .. }
            | DueKind::SensorPass { unit: on, .. }
            | DueKind::Share { unit: on }
            | DueKind::MotorTick { unit: on }
            | DueKind::MindThink { unit: on, .. }
            | DueKind::MindLand { unit: on, .. }
            | DueKind::LossSettles { viewer: on, .. } => *on != unit,
            DueKind::Impact { .. }
            | DueKind::PositionSummary
            | DueKind::MatchDeadline
            | DueKind::HoldSettles { .. } => true,
        });
        // Doc 40 cut FR.5 (§4.1 item 5): a tug gone uncouples its trailer, which stands where it
        // is.
        self.drop_tow(unit);
        self.jam_fold.clear();
        // Doc 36 cut GC.1b: the chassis words are kept in the unit table's row order, so a row
        // gone shifts every later one; laid down again, as a join lays them.
        self.chassis = crate::arena::state::ChassisWords::table(&self.units);
        let terrain = std::sync::Arc::clone(&self.definition);
        self.index.rebuild(&terrain.terrain, &self.units);
        self.alert_range_cm =
            crate::arena::state::alert_range_cm(&self.arena_cfg, &terrain.terrain, &self.units);
        self.jam_radius_cm = crate::arena::state::jam_radius_cm(&self.units);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::state::{PartyId, Power};
    use crate::arena::test_support::{arena_of, line_tank, pose};

    const NEAR: ObjId = ObjId(4);
    const FAR: ObjId = ObjId(9);

    fn field() -> Arena {
        let mut arena = arena_of(vec![
            line_tank(NEAR, PartyId::RED, pose(1_500, 1_500)),
            line_tank(FAR, PartyId::BLUE, pose(5_500, 5_500)),
        ]);
        for id in [NEAR, FAR] {
            if let Some(state) = arena.unit_mut(id) {
                state.power = Power::Online;
            }
        }
        arena
    }

    #[test]
    fn a_unit_beside_the_ramp_leaves_and_the_field_carries_on() {
        let mut arena = field();
        // The ramp in the next cell: within one cell, Chebyshev.
        let back = arena
            .withdraw(NEAR, Some((2_500, 2_500)))
            .expect("it leaves");
        assert_eq!(back.unit, NEAR);
        assert_eq!((back.pose.x_cm, back.pose.y_cm), (1_500, 1_500));
        assert!(arena.unit(NEAR).is_none(), "gone from the arena");
        assert!(arena.pilots.iter().all(|pilot| pilot.unit != NEAR));
        assert_eq!(
            arena.withdraw(NEAR, Some((2_500, 2_500))).err(),
            Some(WithdrawRefusal::NotHere)
        );
        // The rest of the field steps on.
        for wall in (100..=2_000).step_by(100) {
            arena.step_settled(wall, &[], &[]);
        }
        assert!(arena.unit(FAR).is_some());
        assert!(arena.ended.is_none(), "the field carries on without it");
    }

    #[test]
    fn a_unit_away_from_the_ramp_or_unable_to_move_stays() {
        let mut arena = field();
        assert_eq!(
            arena.withdraw(FAR, Some((1_500, 1_500))).err(),
            Some(WithdrawRefusal::NotAtRamp)
        );
        if let Some(state) = arena.unit_mut(NEAR) {
            state.limits.motion_disabled = true;
        }
        assert_eq!(
            arena.withdraw(NEAR, Some((1_500, 1_500))).err(),
            Some(WithdrawRefusal::NotUnderOwnPower)
        );
        if let Some(state) = arena.unit_mut(NEAR) {
            state.limits.motion_disabled = false;
            state.limits.wreck = true;
        }
        assert_eq!(
            arena.withdraw(NEAR, Some((1_500, 1_500))).err(),
            Some(WithdrawRefusal::NotUnderOwnPower)
        );
        assert!(arena.unit(NEAR).is_some() && arena.unit(FAR).is_some());
    }

    #[test]
    fn a_unit_whose_carrier_is_lost_stays() {
        // Doc 40, the ramp exit's boarding window: NEAR boards FAR.
        let mut arena = field();
        if let Some(state) = arena.unit_mut(FAR) {
            state.limits.wreck = true;
        }
        assert_eq!(
            arena
                .withdraw_to(NEAR, Some((2_500, 2_500)), Some(FAR))
                .err(),
            Some(WithdrawRefusal::CarrierLost)
        );
        assert_eq!(
            arena
                .withdraw_to(NEAR, Some((2_500, 2_500)), Some(ObjId(77)))
                .err(),
            Some(WithdrawRefusal::CarrierLost),
            "a carrier not in the field is lost too"
        );
        assert!(arena.unit(NEAR).is_some(), "it stays in the field");
        if let Some(state) = arena.unit_mut(FAR) {
            state.limits.wreck = false;
        }
        let back = arena
            .withdraw_to(NEAR, Some((2_500, 2_500)), Some(FAR))
            .expect("a whole carrier takes it");
        assert_eq!(back.unit, NEAR);
    }

    #[test]
    fn f6_a_lift_needs_no_ramp_and_no_drive() {
        let mut arena = field();
        if let Some(state) = arena.unit_mut(FAR) {
            state.limits.motion_disabled = true;
        }
        let back = arena.withdraw(FAR, None).expect("it lifts");
        assert_eq!(back.unit, FAR);
        assert!(arena.unit(FAR).is_none(), "gone from the arena");
        assert_eq!(
            arena.withdraw(FAR, None).err(),
            Some(WithdrawRefusal::NotHere)
        );
        assert!(arena.unit(NEAR).is_some(), "the unit it leaves stays");
    }

    #[test]
    fn high61_remove_unit_purges_all_unit_state_and_snapshot_round_trips() {
        use crate::arena::conditions::{ASPECTS_ANY, Arming, FireCondition, SlotTarget};
        use crate::arena::detachment::{DetachmentAuthority, DetachmentControl};
        use crate::arena::snapshot::{Snapshot, SnapshotError};
        use crate::arena::state::{
            Channel, Classification, ContactId, DueKind, SeenPose, ShadowContact,
        };

        let mut arena = field();
        // Populate unit-scoped state across detachment_commands, conditions,
        // held_said, contacts, epochs, due events, and spent_said for NEAR.
        assert!(arena.apply_detachment_control(DetachmentControl {
            unit: NEAR,
            epoch: 1,
            authority: Some(DetachmentAuthority {
                detachment: 70,
                revision: 1,
                term: 1,
            }),
        }));
        let fire = FireCondition {
            weapon: smol_str::SmolStr::new_static("cannon"),
            target: SlotTarget::Lock,
            min_permille: 200,
            aspects: ASPECTS_ANY,
            min_rounds_left: 0,
            min_range_m: 0,
            max_range_m: 0,
        };
        assert!(
            arena
                .resolve_manual_arming(NEAR, &Arming::Arm(fire.clone()), 0)
                .is_ok()
        );
        arena.held_said.insert((NEAR, 1, ContactId(7)), 100);
        arena.epochs.raise(NEAR, Channel::Helm, 3);
        let plot = SeenPose::of(arena.unit(FAR).expect("far"));
        arena.contacts.insert(
            NEAR,
            ContactId(7),
            ShadowContact::sighted(
                FAR,
                Classification::Classified {
                    party: PartyId::BLUE,
                },
                0,
                plot,
            ),
        );
        arena.due.schedule(
            5_000,
            DueKind::RuleLeaseExpired {
                unit: NEAR,
                generation: 1,
                lands_ms: 1_000,
            },
        );
        arena.due.schedule(
            2_000,
            DueKind::SensorPass {
                unit: NEAR,
                pass_seq: 9,
            },
        );

        arena.remove_unit(NEAR);

        assert!(arena.unit(NEAR).is_none());
        assert!(!arena.detachment_commands.contains_key(&NEAR));
        assert!(arena.conditions.is_empty_for(NEAR));
        assert!(arena.held_said.keys().all(|(on, _, _)| *on != NEAR));
        assert_eq!(arena.epochs.current(NEAR, Channel::Helm), 0);
        assert!(arena.contacts.of(NEAR).next().is_none());
        assert!(arena.due.iter().all(|event| match &event.kind {
            DueKind::RuleLeaseExpired { unit, .. }
            | DueKind::StartupComplete { unit }
            | DueKind::WeaponReady { unit, .. }
            | DueKind::SensorPass { unit, .. }
            | DueKind::Share { unit }
            | DueKind::MotorTick { unit }
            | DueKind::MindThink { unit, .. }
            | DueKind::MindLand { unit, .. }
            | DueKind::LossSettles { viewer: unit, .. } => *unit != NEAR,
            _ => true,
        }));

        // Snapshot encode AND decode succeed cleanly after unit removal.
        let snapshot = Snapshot::encode(&arena).expect("snapshot encodes after remove_unit");
        let decoded = Snapshot::decode(&snapshot.document, (*arena.definition).clone())
            .expect("snapshot decodes after remove_unit");
        assert!(decoded.unit(NEAR).is_none());
        assert!(decoded.unit(FAR).is_some());

        // Snapshot::encode refuses dangling conditions for non-existent units directly.
        let mut corrupt = decoded;
        let _ = corrupt
            .conditions
            .arm(ObjId(999), fire, crate::arena::core::MAX_CONDITION_SLOTS);
        assert!(matches!(
            Snapshot::encode(&corrupt),
            Err(SnapshotError::Malformed(_))
        ));
    }

    #[test]
    fn p2_tsc_14_remove_unit_gcs_cards_prunes_designators_and_stale_refs() {
        use crate::arena::card::line as line_card;
        use crate::arena::party::PartyId;
        use crate::arena::profile_seal::SealedCards;
        use crate::arena::runtime_card::{MAX_CARD_DEFINITIONS, stock_definition};
        use crate::arena::state::ContactId;
        use crate::arena::test_support::custody;
        use crate::arena::{DesignatorBook, EndCondition, FieldHost, Tempo};
        use crate::config::{ArenaConfig, MatchConfig, ReflexConfig, SensorsConfig};
        use smol_str::SmolStr;

        let p_a = PartyId::nth(0).expect("party A");
        let p_b = PartyId::nth(1).expect("party B");
        let host = FieldHost {
            all_hostile: false,
            marks: Vec::new(),
            friends: Vec::new(),
            end: EndCondition::Host,
            every_class: true,
            weapons_hold: false,
            tows: Vec::new(),
        };
        let mut out = custody(
            vec![
                line_tank(NEAR, p_a, pose(1_500, 1_500)),
                line_tank(FAR, p_b, pose(6_500, 6_500)),
            ],
            Vec::new(),
        );
        out.host = host.clone();
        let mut arena = Arena::new(
            out,
            &ArenaConfig::default(),
            &MatchConfig::default(),
            &SensorsConfig::default(),
        )
        .expect("ic field");
        let base_cards = arena.cards.len();
        let base_bytes = arena.cards.encoded_bytes();
        let reflex_cfg = ReflexConfig::default();

        // Cycle > MAX_CARD_DEFINITIONS (512) distinct card definitions through
        // join + withdraw; card GC keeps `self.cards` bounded to live hulls.
        let cycles = MAX_CARD_DEFINITIONS + 12;
        for i in 0..cycles {
            let joiner = ObjId(1_000 + u32::try_from(i).expect("idx"));
            let mut unit_custody = line_tank(joiner, p_a, pose(4_500, 4_500));
            let mut stock = line_card(&reflex_cfg);
            stock.close_m = 50 + u32::try_from(i).expect("idx");
            let card_def = stock_definition(
                &[(SmolStr::new_static("line"), stock)],
                unit_custody.seal.class,
            )
            .expect("stock def");
            let sealed = SealedCards::prepare(
                card_def,
                &[SmolStr::new_static("line")],
                &arena.classes.row(unit_custody.seal.class),
                &unit_custody.seal.weapons,
                &unit_custody.seal.channels,
                unit_custody.seal.odds_floor_permille,
            )
            .expect("sealed cards");
            unit_custody.seal.card = sealed.opening_card.clone();
            unit_custody.seal.card_digest = sealed.opening_digest;
            unit_custody.seal.cards = sealed;

            arena
                .join(unit_custody, None, &host, 4)
                .unwrap_or_else(|e| panic!("join cycle {i} failed: {e:?}"));
            assert!(
                arena.cards.len() <= base_cards + 1,
                "catalog grew unbounded at cycle {i}: {}",
                arena.cards.len()
            );
            arena.withdraw(joiner, None).expect("withdraw joiner");
            assert_eq!(arena.cards.len(), base_cards);
            assert_eq!(arena.cards.encoded_bytes(), base_bytes);
        }

        // Populate stale references for `NEAR` across `designators`,
        // `fired_on_by`, `held_said`, `held_losses`, `held_loss_targets`, and
        // `lost_designators`, then remove `NEAR` and verify every leak is pruned.
        let ghost_id = ContactId(42);
        let _ = arena.designators.assign(NEAR, FAR);
        let _ = arena.designators.assign(FAR, NEAR);
        arena.designators.retire_track(NEAR, FAR);
        arena.unit_mut(FAR).expect("far").fired_on_by.insert(NEAR);
        arena.held_said.insert((FAR, 1, ghost_id), 100);
        arena.held_losses.insert(
            (NEAR, ghost_id),
            (0, crate::arena::state::Designator::Provisional(1)),
        );
        arena.held_loss_targets.insert((NEAR, ghost_id), FAR);
        arena.lost_designators.insert(
            (NEAR, ghost_id),
            crate::arena::state::Designator::Provisional(1),
        );
        // Simulate a stale `held_said` and a pending loss hysteresis entry on `FAR` targeting `NEAR`.
        let orphan_id = ContactId(77);
        arena.held_said.insert((FAR, 2, orphan_id), 200);
        arena.held_losses.insert(
            (FAR, orphan_id),
            (500, crate::arena::state::Designator::Provisional(9)),
        );
        arena.held_loss_targets.insert((FAR, orphan_id), NEAR);
        arena.lost_designators.insert(
            (FAR, orphan_id),
            crate::arena::state::Designator::Provisional(9),
        );

        arena.remove_unit(NEAR);

        assert!(
            !arena.unit(FAR).expect("far").fired_on_by.contains(&NEAR),
            "withdrawn shooter must be pruned from surviving unit's fired_on_by"
        );
        assert_eq!(arena.designators.provisional(NEAR, FAR), None);
        assert_eq!(arena.designators.epoch(NEAR, FAR), 0);
        assert!(arena.held_losses.keys().all(|(v, _)| *v != NEAR));
        assert!(arena.held_loss_targets.keys().all(|(v, _)| *v != NEAR));
        assert!(arena.lost_designators.keys().all(|(v, _)| *v != NEAR));
        assert!(!arena.held_said.contains_key(&(FAR, 2, orphan_id)));
        assert!(!arena.held_said.contains_key(&(FAR, 1, ghost_id)));
        // Once `FAR`'s pending loss on `NEAR` settles, `(FAR, NEAR)` is forgotten completely.
        arena.settle_held_loss(FAR, orphan_id, 500);
        assert_eq!(arena.designators.provisional(FAR, NEAR), None);
        assert_eq!(arena.designators.epoch(FAR, NEAR), 0);
        assert!(!arena.held_losses.contains_key(&(FAR, orphan_id)));
        assert!(!arena.held_loss_targets.contains_key(&(FAR, orphan_id)));
        // Sanity check that `FAR`'s frame still renders cleanly with no panics.
        let _ = arena.frame_for(FAR, Tempo::Engaged);

        // `DesignatorBook::assign` wraps past `u16::MAX` skipping in-use numbers.
        let mut book = DesignatorBook::default();
        let v = ObjId(1);
        let t1 = ObjId(10);
        let t2 = ObjId(20);
        let t3 = ObjId(30);
        assert_eq!(book.assign(v, t1), 1);
        let mut val = serde_json::to_value(&book).expect("to_value");
        val["next"]["1"] = serde_json::json!(u16::MAX);
        let mut wrapped: DesignatorBook = serde_json::from_value(val).expect("from_value");
        assert_eq!(wrapped.assign(v, t2), u16::MAX);
        // Wrapping past `u16::MAX` skips `1` (still in use by `t1`) and assigns `2`.
        assert_eq!(wrapped.assign(v, t3), 2);
    }

    #[test]
    fn p2_tsc_15_withdrawn_target_keeps_stale_plot_until_ttl_without_presence_oracle() {
        use crate::arena::Tempo;
        use crate::arena::ledger::EventKind;
        use crate::arena::perception::{Notice, PerceptionCounters};
        use crate::arena::snapshot::Snapshot;
        use crate::arena::state::{Classification, SeenPose, ShadowContact};
        use crate::arena::weapons::FiredBy;

        let mut near_custody = line_tank(NEAR, PartyId::RED, pose(1_500, 1_500));
        near_custody.controller = crate::arena::state::Controller::Mind;
        let mut arena = arena_of(vec![
            near_custody,
            line_tank(FAR, PartyId::BLUE, pose(5_500, 5_500)),
        ]);
        for id in [NEAR, FAR] {
            if let Some(state) = arena.unit_mut(id) {
                state.power = Power::Online;
            }
        }
        let far_callsign = arena.unit(FAR).expect("far").callsign;
        let far_plot = SeenPose::of(arena.unit(FAR).expect("far"));
        let contact = crate::arena::rng::draw_contact_id(arena.seed, NEAR, FAR);
        arena.contacts.insert(
            NEAR,
            contact,
            ShadowContact::sighted(
                FAR,
                Classification::Classified {
                    party: PartyId::BLUE,
                },
                0,
                far_plot,
            ),
        );
        assert_eq!(arena.resolve_lock(NEAR, contact, 0), Ok(()));
        // Record a pre-withdraw frame baseline to compare against post-withdraw rendering.
        let before = arena.frame_for(NEAR, Tempo::Engaged).expect("before frame");
        assert_eq!(before.lock, Some(contact));
        assert!(before.contacts.iter().any(|row| row.id == contact));
        assert!(!before.odds.is_empty());

        // Withdraw `FAR` mid-fight while `NEAR` holds a classified, locked contact on it.
        arena.withdraw(FAR, None).expect("FAR withdraws");
        assert!(arena.unit(FAR).is_none());
        assert!(
            arena.withdrawn_plots.contains_key(&FAR),
            "withdrawn target must leave a tombstone while a viewer still holds a plot"
        );

        // Immediately after withdrawal, `NEAR`'s frame still renders the stale
        // plot identically: `Frame::lock ∈ Frame::contacts`, callsign/chassis/odds intact.
        let after = arena.frame_for(NEAR, Tempo::Engaged).expect("after frame");
        assert_eq!(
            after.lock,
            Some(contact),
            "lock must not drop immediately on target withdrawal (presence oracle)"
        );
        let row = after
            .contacts
            .iter()
            .find(|c| c.id == contact)
            .expect("stale plot must remain in Frame::contacts until contact_ttl_ms");
        assert_eq!(row.designator, far_callsign);
        assert!(
            !after.odds.is_empty(),
            "Frame::odds must still resolve from tombstone metadata"
        );

        // Aiming, weapon-range/alert checks, and deliberation signals work without `self.unit(FAR)`.
        let aim = arena
            .aim_at(NEAR, contact)
            .expect("aim_at must resolve stale plot");
        let seen = arena.resolve_seen(NEAR, &aim, "cannon");
        assert!(seen.presented_width_cm > 0);
        assert!(
            arena.hostile_in_weapons_range(),
            "stale armed hostile plot stays in weapons range until TTL"
        );
        assert!(
            arena.contact_within_alert_range(),
            "stale hostile plot stays in alert range until TTL"
        );
        let signals = arena.take_deliberation_signals();
        assert!(
            !signals.is_empty(),
            "squad signal should be emitted for NEAR"
        );
        for sig in &signals {
            for c in &sig.contacts {
                assert!(
                    c.range_cm < u64::MAX,
                    "deliberation signal must use stale plot range, not u64::MAX sentinel: {c:?}"
                );
            }
        }

        // Firing at the locked stale contact is accepted at trigger time (no
        // withdrawal oracle before flight) and resolves `TARGET_GONE` at impact.
        let shot_seq = arena
            .resolve_fire_by(NEAR, "cannon", 100, FiredBy::Seat)
            .expect("fire at stale plot must be accepted at trigger time");
        let mut journal: Vec<Notice> = Vec::new();
        arena.resolve_impact(shot_seq, 500, &mut journal);
        assert!(
            journal.iter().any(|(_, kind, _)| matches!(
                kind,
                EventKind::Miss { reason, .. } if reason == crate::arena::damage::TARGET_GONE
            )),
            "shot fired at withdrawn target's stale plot must resolve TARGET_GONE at impact: {journal:?}"
        );

        // Snapshot encode + decode preserves the tombstone and stale plot cleanly.
        let snap = Snapshot::encode(&arena).expect("encode with tombstone");
        let mut decoded =
            Snapshot::decode(&snap.document, (*arena.definition).clone()).expect("decode");
        assert!(decoded.withdrawn_plots.contains_key(&FAR));
        let decoded_frame = decoded
            .frame_for(NEAR, Tempo::Engaged)
            .expect("decoded frame");
        assert!(decoded_frame.contacts.iter().any(|c| c.id == contact));

        // Once contact TTL expires (`pass_ms * lost_after_passes`), the stale
        // plot ages out naturally and `withdrawn_plots` prunes the tombstone.
        let ttl = i64::from(decoded.sensors_cfg.pass_ms)
            * i64::from(decoded.sensors_cfg.lost_after_passes);
        let expire_at = ttl + 2_000;
        let mut counters = PerceptionCounters::default();
        let mut pass_journal: Vec<Notice> = Vec::new();
        let _ = decoded.sensor_pass(NEAR, 99, expire_at, &mut counters, &mut pass_journal);
        assert!(
            decoded.contacts.get(NEAR, contact).is_none(),
            "stale plot must expire naturally past contact TTL"
        );
        assert!(
            !decoded.withdrawn_plots.contains_key(&FAR),
            "unreferenced tombstone must be pruned once no viewer holds the plot"
        );
    }
}
