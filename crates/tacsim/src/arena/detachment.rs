//! Fenced detachment commands at the machine boundary. The host admits common
//! roster authority; the arena fences queued messages and resolves every hint
//! in the recipient's own classified observations. Squad ids are execution
//! metadata and never confer command authority on this lane.

use std::collections::BTreeMap;

use mush_core::ObjId;
use serde::{Deserialize, Deserializer, Serialize};

use super::intents::{IntentOutcome, IntentSource, Refusal};
use super::state::{Arena, Controller};
use crate::deliberation::SquadId;
use crate::deliberation::order::{Constraints, Objective};

fn required_option<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(deserializer)
}

/// Common roster authority admitted by the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetachmentAuthority {
    pub detachment: u64,
    pub revision: u64,
    pub term: u64,
}

/// Per-hull command ownership. Epochs increase independently of helm epochs and
/// survive removal as tombstones. `None` removes ownership; a leaderless term
/// retains `Some` so an already delivered standing objective continues.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetachmentControl {
    pub unit: ObjId,
    pub epoch: u64,
    #[serde(deserialize_with = "required_option")]
    pub authority: Option<DetachmentAuthority>,
}

/// A doctrine objective, with no mandatory legacy TTL.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetachmentObjective {
    pub objective: Objective,
    pub constraints: Constraints,
}

/// An already admitted, delivered command. `None` is cancellation. The deadline
/// is absolute arena time; no deadline is stored as `i64::MAX` in the mind's
/// existing execution row. No periodic renewal is needed for standing orders.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetachmentCommand {
    pub authority: DetachmentAuthority,
    pub authority_epoch: u64,
    pub sequence: u64,
    pub issuer: ObjId,
    pub issuer_unit: ObjId,
    pub squad: SquadId,
    #[serde(deserialize_with = "required_option")]
    pub directive: Option<DetachmentObjective>,
    #[serde(deserialize_with = "required_option")]
    pub expires_at_ms: Option<i64>,
}

/// One fixed-size row per arena hull, including revoked epoch tombstones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DetachmentState {
    epoch: u64,
    #[serde(deserialize_with = "required_option")]
    authority: Option<DetachmentAuthority>,
    sequence: u64,
}

pub(crate) type DetachmentStates = BTreeMap<ObjId, DetachmentState>;

impl Arena {
    /// Apply the host's independently monotonic authority fence. Same-epoch
    /// duplicates are harmless; conflicting or old messages never roll it back.
    /// A succession within the same assignment revision retains its delivered
    /// objective; a new revision starts with no objective or accepted sequence.
    pub fn apply_detachment_control(&mut self, control: DetachmentControl) -> bool {
        if control.epoch == 0 || self.unit(control.unit).is_none() {
            return false;
        }
        let previous = self.detachment_commands.get(&control.unit).copied();
        if let Some(previous) = previous
            && control.epoch <= previous.epoch
        {
            return control.epoch == previous.epoch && control.authority == previous.authority;
        }
        let same_assignment = previous
            .and_then(|state| state.authority)
            .zip(control.authority)
            .is_some_and(|(old, new)| {
                old.detachment == new.detachment && old.revision == new.revision
            });
        if !same_assignment
            && (control.authority.is_some()
                || previous.is_some_and(|state| state.authority.is_some()))
        {
            // Claiming a hull also removes its legacy instruction. The two
            // authorities never share ownership of a machine's standing order.
            self.clear_detachment_order(control.unit);
        }
        self.detachment_commands.insert(
            control.unit,
            DetachmentState {
                epoch: control.epoch,
                authority: control.authority,
                sequence: if same_assignment {
                    previous.map_or(0, |state| state.sequence)
                } else {
                    0
                },
            },
        );
        true
    }

    /// OOC practice restart policy: the ephemeral host's detachment state ends
    /// with its process. Clear its owned objectives and fences before admitting
    /// a fresh practice group. This is not the durable IC resumption policy.
    pub fn reset_detachment_commands(&mut self) {
        let owned: Vec<_> = self
            .detachment_commands
            .iter()
            .filter_map(|(unit, state)| state.authority.map(|_| *unit))
            .collect();
        for unit in owned {
            self.clear_detachment_order(unit);
        }
        self.detachment_commands.clear();
    }

    pub(crate) fn detachment_owns(&self, unit: ObjId) -> bool {
        self.detachment_commands
            .get(&unit)
            .is_some_and(|state| state.authority.is_some())
    }

    fn clear_detachment_order(&mut self, unit: ObjId) {
        if let Some(row) = self.minds.get_mut(&unit) {
            row.order = None;
            row.order_issuer = None;
            row.order_charge = None;
        }
        self.clear_order_watch_member(unit);
    }

    pub(crate) fn validate_detachment_commands(&self) -> Result<(), super::SnapshotError> {
        if self
            .minds
            .values()
            .filter_map(|row| row.order.as_ref())
            .filter_map(|order| order.identity.as_deref())
            .any(|identity| !super::perception::valid_order_identity(identity))
        {
            return Err(super::SnapshotError::Malformed(
                "invalid detachment order identity".into(),
            ));
        }
        if self.detachment_commands.len() > self.units.len()
            || self.detachment_commands.iter().any(|(unit, state)| {
                self.unit(*unit).is_none()
                    || state.epoch == 0
                    || (state.authority.is_none() && state.sequence != 0)
            })
        {
            return Err(super::SnapshotError::Malformed(
                "invalid detachment authority state".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn apply_detachment_command(
        &mut self,
        unit: ObjId,
        command: &DetachmentCommand,
        source: IntentSource,
    ) -> IntentOutcome {
        if source != IntentSource::Seat {
            return IntentOutcome::Refused(Refusal::Channel);
        }
        let Some(recipient) = self.unit(unit) else {
            return IntentOutcome::Refused(Refusal::Unknown);
        };
        if recipient.controller == Controller::Pilot {
            return IntentOutcome::Refused(Refusal::NotMachine);
        }
        let authorised = self.detachment_commands.get(&unit).is_some_and(|state| {
            state.epoch == command.authority_epoch
                && state.authority == Some(command.authority)
                && command.sequence > state.sequence
        });
        if !authorised
            || command.squad.team != recipient.team
            || !self
                .unit(command.issuer_unit)
                .is_some_and(|issuer| issuer.team == recipient.team && !issuer.limits.wreck)
        {
            return IntentOutcome::Refused(Refusal::NoCommander);
        }
        if command
            .expires_at_ms
            .is_some_and(|deadline| deadline <= self.now_ms)
        {
            return IntentOutcome::Refused(Refusal::NoCommander);
        }
        if let Some(directive) = command.directive {
            let valid = match directive.objective {
                Objective::Advance { to: (x, y) } => {
                    x < self.definition.terrain.width() && y < self.definition.terrain.height()
                }
                Objective::Screen { facing_mdeg } => facing_mdeg < 360_000,
                Objective::Support { of } => self
                    .unit(of)
                    .is_some_and(|charge| charge.team == recipient.team),
                _ => true,
            };
            if !valid {
                return IntentOutcome::Refused(Refusal::NoCommander);
            }
        }
        let hash = self.event_hash();
        self.clear_detachment_order(unit);
        if let Some(directive) = command.directive {
            let mut rekeyed = self.rekey_objective(
                unit,
                Some(command.issuer_unit),
                command.squad,
                directive,
                command.expires_at_ms.unwrap_or(i64::MAX),
            );
            // Exact admitted source identity, at most 53 printable ASCII bytes.
            // The OOC host gives each activation a fresh roster revision, so a
            // resumed mind cannot confuse a new order with its former host's.
            rekeyed.identity = Some(smol_str::SmolStr::new(format!(
                "dt:{:x}:{:x}:{:x}",
                command.authority.detachment, command.authority.revision, command.sequence
            )));
            if !self.minds.contains_key(&unit) {
                let seal_digest = self.unit(unit).map_or([0; 32], |state| state.seal.digest);
                self.minds.insert(
                    unit,
                    super::mind::MindState {
                        seal_digest,
                        ..super::mind::MindState::fresh(&self.arena_cfg.reflex)
                    },
                );
            }
            if let Some(row) = self.minds.get_mut(&unit) {
                row.order = Some(rekeyed);
                row.order_issuer = Some(command.issuer_unit);
                row.order_charge = match directive.objective {
                    Objective::Support { of } if of != unit => Some(of),
                    _ => None,
                };
            }
        }
        if let Some(state) = self.detachment_commands.get_mut(&unit) {
            state.sequence = command.sequence;
        }
        let _ = self.ledger.append(
            self.now_ms,
            super::ledger::EventKind::DetachmentCommanded {
                unit,
                command: Box::new(command.clone()),
            },
            Vec::new(),
            hash,
        );
        IntentOutcome::AppliedContinuous
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::perception::MindObjective;
    use crate::arena::state::{Classification, ContactId, SeenPose, ShadowContact};
    use crate::arena::test_support::{arena_at_epoch, pose, tank};
    use crate::arena::{
        Channel, Intent, IntentKind, MachineState, PartyId, Snapshot, apply_intent, state_hash,
    };

    const LEADER: ObjId = ObjId(1);
    const MEMBER: ObjId = ObjId(2);
    const ENEMY: ObjId = ObjId(3);

    fn field() -> Arena {
        let mut arena = arena_at_epoch(
            vec![
                tank(LEADER, PartyId::RED, pose(1500, 1500)),
                tank(MEMBER, PartyId::RED, pose(2500, 1500)),
                tank(ENEMY, PartyId::BLUE, pose(6500, 6500)),
            ],
            1,
        );
        arena.set_controller(MEMBER, Controller::Mind, MachineState::Patrol);
        arena
    }
    fn authority(term: u64) -> DetachmentAuthority {
        DetachmentAuthority {
            detachment: 4,
            revision: 7,
            term,
        }
    }
    fn control(epoch: u64, term: u64) -> DetachmentControl {
        DetachmentControl {
            unit: MEMBER,
            epoch,
            authority: Some(authority(term)),
        }
    }
    fn command(epoch: u64, term: u64, sequence: u64) -> DetachmentCommand {
        DetachmentCommand {
            authority: authority(term),
            authority_epoch: epoch,
            sequence,
            issuer: ObjId(99),
            issuer_unit: LEADER,
            // Deliberately no legacy squad roster: this id is metadata alone.
            squad: SquadId {
                team: PartyId::RED,
                index: 1,
            },
            directive: Some(DetachmentObjective {
                objective: Objective::Hold,
                constraints: Constraints::default(),
            }),
            expires_at_ms: None,
        }
    }
    fn intent(command: DetachmentCommand) -> Intent {
        Intent {
            unit: MEMBER,
            channel: Channel::Helm,
            epoch: 1,
            seq: 1,
            kind: IntentKind::DetachmentCommand(Box::new(command)),
        }
    }
    fn issue(arena: &mut Arena, command: DetachmentCommand) -> IntentOutcome {
        apply_intent(arena, &intent(command), IntentSource::Seat)
    }
    fn held(arena: &Arena) -> Option<crate::arena::perception::FrameOrder> {
        arena
            .minds
            .get(&MEMBER)
            .and_then(|state| state.order.clone())
    }

    #[test]
    fn dt_standing_succession_sequences_and_cancel_are_fenced() {
        let mut arena = field();
        assert_eq!(
            issue(&mut arena, command(1, 1, 1)),
            IntentOutcome::Refused(Refusal::NoCommander)
        );
        assert!(arena.apply_detachment_control(control(1, 1)));
        assert_eq!(
            issue(&mut arena, command(1, 1, 1)),
            IntentOutcome::AppliedContinuous
        );
        let standing = held(&arena).unwrap();
        assert_eq!(standing.expires_ms, i64::MAX);
        assert!(arena.apply_detachment_control(control(2, 2)));
        assert_eq!(
            held(&arena),
            Some(standing.clone()),
            "succession retains the delivered objective"
        );
        let mut old_cancel = command(1, 1, 2);
        old_cancel.directive = None;
        assert_eq!(
            issue(&mut arena, old_cancel),
            IntentOutcome::Refused(Refusal::NoCommander)
        );
        assert!(!arena.apply_detachment_control(control(1, 1)));
        assert!(!arena.apply_detachment_control(control(2, 99)));
        assert_eq!(held(&arena), Some(standing));
        assert_eq!(
            issue(&mut arena, command(2, 2, 1)),
            IntentOutcome::Refused(Refusal::NoCommander)
        );
        let mut cancel = command(2, 2, 2);
        cancel.directive = None;
        assert_eq!(
            issue(&mut arena, cancel.clone()),
            IntentOutcome::AppliedContinuous
        );
        assert!(held(&arena).is_none());
        assert!(
            arena.detachment_owns(MEMBER),
            "cancel gives doctrine, not legacy authority"
        );
        assert_eq!(
            issue(&mut arena, cancel),
            IntentOutcome::Refused(Refusal::NoCommander)
        );
        assert_eq!(
            issue(&mut arena, command(2, 2, 3)),
            IntentOutcome::AppliedContinuous
        );
    }

    #[test]
    fn dt_assignment_revision_clears_old_order_and_source_sequence() {
        let mut arena = field();
        arena.apply_detachment_control(control(1, 1));
        assert_eq!(
            issue(&mut arena, command(1, 1, 90)),
            IntentOutcome::AppliedContinuous
        );
        let mut reassigned = control(2, 1);
        reassigned.authority.as_mut().unwrap().revision = 8;
        assert!(arena.apply_detachment_control(reassigned));
        assert!(held(&arena).is_none());
        let mut fresh = command(2, 1, 1);
        fresh.authority.revision = 8;
        assert_eq!(issue(&mut arena, fresh), IntentOutcome::AppliedContinuous);
        assert_eq!(held(&arena).unwrap().identity.as_deref(), Some("dt:4:8:1"));
        let mut obsolete = command(2, 1, 91);
        obsolete.directive = None;
        assert_eq!(
            issue(&mut arena, obsolete),
            IntentOutcome::Refused(Refusal::NoCommander)
        );
        assert!(held(&arena).is_some());
    }

    #[test]
    fn dt_removal_tombstones_and_ooc_reset_keep_legacy_compatibility() {
        let mut arena = field();
        let legacy = Intent {
            unit: MEMBER,
            channel: Channel::Helm,
            epoch: 1,
            seq: 1,
            kind: IntentKind::Order(Box::new(crate::deliberation::order::Order {
                squad: command(1, 1, 1).squad,
                objective: Objective::Hold,
                target: None,
                horizon_ms: 1000,
                constraints: Constraints::default(),
                ttl_ms: 1000,
                issued_ms: 0,
                issuer: Some(LEADER),
                by: crate::deliberation::CharacterRef::Character { id: ObjId(99) },
                spot: None,
            })),
        };
        assert_eq!(
            apply_intent(&mut arena, &legacy, IntentSource::Seat),
            IntentOutcome::AppliedContinuous
        );
        assert!(arena.apply_detachment_control(control(1, 1)));
        assert!(
            held(&arena).is_none(),
            "claiming DT clears the legacy objective"
        );
        assert_eq!(
            issue(&mut arena, command(1, 1, 1)),
            IntentOutcome::AppliedContinuous
        );
        assert_eq!(
            apply_intent(&mut arena, &legacy, IntentSource::Seat),
            IntentOutcome::Refused(Refusal::NoCommander)
        );
        let mut release = legacy.clone();
        release.kind = IntentKind::ReleaseOrder {
            squad: command(1, 1, 1).squad,
            issuer: Some(LEADER),
        };
        assert_eq!(
            apply_intent(&mut arena, &release, IntentSource::Seat),
            IntentOutcome::Refused(Refusal::NoCommander)
        );
        assert!(arena.apply_detachment_control(DetachmentControl {
            unit: MEMBER,
            epoch: 2,
            authority: None
        }));
        assert!(held(&arena).is_none());
        assert!(!arena.apply_detachment_control(control(1, 1)));
        assert_eq!(
            apply_intent(&mut arena, &legacy, IntentSource::Seat),
            IntentOutcome::AppliedContinuous
        );
        assert!(arena.apply_detachment_control(DetachmentControl {
            unit: MEMBER,
            epoch: 3,
            authority: None
        }));
        assert!(
            held(&arena).is_some(),
            "another revocation cannot erase unowned legacy doctrine"
        );
        arena.reset_detachment_commands();
        assert!(
            held(&arena).is_some(),
            "reset leaves an unowned legacy objective intact"
        );
        assert!(arena.apply_detachment_control(control(1, 1)));
        assert_eq!(
            issue(&mut arena, command(1, 1, 1)),
            IntentOutcome::AppliedContinuous
        );
        arena.reset_detachment_commands();
        assert!(held(&arena).is_none());
        assert!(arena.detachment_commands.is_empty());
    }

    #[test]
    fn dt_helm_epoch_machine_and_deadline_gates_do_not_advance_sequence() {
        let mut arena = field();
        assert!(arena.apply_detachment_control(control(1, 1)));
        let mut message = intent(command(1, 1, 1));
        message.epoch = 0;
        assert_eq!(
            apply_intent(&mut arena, &message, IntentSource::Seat),
            IntentOutcome::Refused(Refusal::Epoch)
        );
        message.epoch = 1;
        message.channel = Channel::Fire;
        assert_eq!(
            apply_intent(&mut arena, &message, IntentSource::Seat),
            IntentOutcome::Refused(Refusal::Channel)
        );
        message.channel = Channel::Helm;
        for source in [
            IntentSource::Mind,
            IntentSource::Machine,
            IntentSource::Cond,
        ] {
            assert_eq!(
                apply_intent(&mut arena, &message, source),
                IntentOutcome::Refused(Refusal::Channel)
            );
        }
        arena.set_controller(MEMBER, Controller::Pilot, MachineState::Patrol);
        assert_eq!(
            apply_intent(&mut arena, &message, IntentSource::Seat),
            IntentOutcome::Refused(Refusal::NotMachine)
        );
        arena.set_controller(MEMBER, Controller::Mind, MachineState::Patrol);
        let mut expired = command(1, 1, 1);
        expired.expires_at_ms = Some(arena.now_ms);
        assert_eq!(
            issue(&mut arena, expired),
            IntentOutcome::Refused(Refusal::NoCommander)
        );
        let mut timed = command(1, 1, 1);
        timed.expires_at_ms = Some(1000);
        assert_eq!(issue(&mut arena, timed), IntentOutcome::AppliedContinuous);
        assert_eq!(held(&arena).unwrap().expires_ms, 1000);
    }

    #[test]
    fn dt_hint_is_rekeyed_only_into_recipient_classified_observations() {
        let mut arena = field();
        arena.apply_detachment_control(control(1, 1));
        let seen = SeenPose::of(arena.unit(ENEMY).unwrap());
        arena.contacts.insert(
            LEADER,
            ContactId(70),
            ShadowContact::sighted(
                ENEMY,
                Classification::Classified {
                    party: PartyId::BLUE,
                },
                0,
                seen,
            ),
        );
        let mut tell = command(1, 1, 1);
        tell.directive.as_mut().unwrap().objective = Objective::Engage {
            target_hint: Some(ContactId(70)),
        };
        assert_eq!(
            issue(&mut arena, tell.clone()),
            IntentOutcome::AppliedContinuous
        );
        assert_eq!(
            held(&arena).unwrap().objective,
            MindObjective::Engage { target: None }
        );
        let seen = SeenPose::of(arena.unit(ENEMY).unwrap());
        arena.contacts.insert(
            MEMBER,
            ContactId(30),
            ShadowContact::sighted(ENEMY, Classification::Unknown, 0, seen),
        );
        tell.sequence = 2;
        assert_eq!(
            issue(&mut arena, tell.clone()),
            IntentOutcome::AppliedContinuous
        );
        assert_eq!(
            held(&arena).unwrap().objective,
            MindObjective::Engage { target: None }
        );
        let seen = SeenPose::of(arena.unit(ENEMY).unwrap());
        arena.contacts.insert(
            MEMBER,
            ContactId(30),
            ShadowContact::sighted(
                ENEMY,
                Classification::Classified {
                    party: PartyId::BLUE,
                },
                0,
                seen,
            ),
        );
        tell.sequence = 3;
        assert_eq!(
            issue(&mut arena, tell.clone()),
            IntentOutcome::AppliedContinuous
        );
        assert_eq!(
            held(&arena).unwrap().objective,
            MindObjective::Engage {
                target: Some(ContactId(30))
            }
        );
        // Changing the leader's scope never treats the foreign key as the
        // recipient's, even if the recipient happens to hold that numeric id.
        tell.sequence = 4;
        tell.issuer_unit = MEMBER;
        assert_eq!(issue(&mut arena, tell), IntentOutcome::AppliedContinuous);
        assert_eq!(
            held(&arena).unwrap().objective,
            MindObjective::Engage { target: None }
        );
    }

    #[test]
    fn dt_snapshot_hash_and_tombstones_preserve_fences() {
        let mut arena = field();
        let initial = state_hash(&arena);
        arena.apply_detachment_control(control(1, 1));
        assert_ne!(state_hash(&arena), initial);
        issue(&mut arena, command(1, 1, 8));
        let snapshot = Snapshot::encode(&arena).unwrap();
        let mut again =
            Snapshot::decode(&snapshot.document, arena.definition.as_ref().clone()).unwrap();
        assert_eq!(state_hash(&again), state_hash(&arena));
        assert_eq!(held(&again), held(&arena));
        assert_eq!(
            issue(&mut again, command(1, 1, 8)),
            IntentOutcome::Refused(Refusal::NoCommander)
        );
        again.apply_detachment_control(DetachmentControl {
            unit: MEMBER,
            epoch: 4,
            authority: None,
        });
        let snapshot = Snapshot::encode(&again).unwrap();
        let mut removed =
            Snapshot::decode(&snapshot.document, again.definition.as_ref().clone()).unwrap();
        assert!(!removed.apply_detachment_control(control(3, 1)));
        assert!(held(&removed).is_none());
        assert!(removed.apply_detachment_control(control(5, 2)));
        let order = held(&arena).unwrap();
        for invalid in [
            "dt:00:1:1".to_owned(),
            "dt:1:2:3:4".to_owned(),
            "x".repeat(54),
        ] {
            let mut malformed = serde_json::to_value(&order).unwrap();
            malformed["identity"] = serde_json::json!(invalid);
            assert!(
                serde_json::from_value::<super::super::perception::FrameOrder>(malformed).is_err()
            );
        }
        let mut malformed = serde_json::to_value(command(1, 1, 1)).unwrap();
        malformed.as_object_mut().unwrap().remove("directive");
        assert!(
            serde_json::from_value::<DetachmentCommand>(malformed).is_err(),
            "missing directive cannot become cancellation"
        );
        let mut malformed = serde_json::to_value(control(1, 1)).unwrap();
        malformed.as_object_mut().unwrap().remove("authority");
        assert!(
            serde_json::from_value::<DetachmentControl>(malformed).is_err(),
            "missing authority cannot become revocation"
        );
    }

    /// **Doc 32 cut CF.6** (Q-CF-41): a delivered `HOLD FIRE` binds the mind's own shots from
    /// the moment the command lands — the pulse head the host's message is applied at — over
    /// the seat's gate, and a later order without it frees them again.
    #[test]
    fn cf6_a_delivered_hold_fire_binds_the_minds_shots_from_the_pulse_it_lands() {
        let mut arena = field();
        assert!(arena.apply_detachment_control(control(1, 1)));
        let mut engage = command(1, 1, 1);
        engage.directive.as_mut().unwrap().objective = Objective::Engage { target_hint: None };
        assert_eq!(
            issue(&mut arena, engage.clone()),
            IntentOutcome::AppliedContinuous
        );
        assert!(!arena.holds_fire(MEMBER), "an ENGAGE leaves the mind free");
        let mut hold = engage.clone();
        hold.sequence = 2;
        hold.directive.as_mut().unwrap().constraints.hold_fire = true;
        assert_eq!(issue(&mut arena, hold), IntentOutcome::AppliedContinuous);
        assert!(arena.holds_fire(MEMBER), "HOLD FIRE binds at once");
        assert!(held(&arena).is_some_and(|order| order.hold_fire));
        let mut free = engage;
        free.sequence = 3;
        assert_eq!(issue(&mut arena, free), IntentOutcome::AppliedContinuous);
        assert!(!arena.holds_fire(MEMBER), "a fresh ENGAGE frees it");
    }

    #[test]
    fn dt_native_mind_executes_standing_objective_and_expires_finite_order() {
        let mut arena = field();
        arena.unit_mut(MEMBER).unwrap().power = super::super::state::Power::Online;
        let start = arena.unit(MEMBER).unwrap().pose.clone();
        arena.apply_detachment_control(control(1, 1));
        let mut advance = command(1, 1, 1);
        advance.directive.as_mut().unwrap().objective = Objective::Advance { to: (5, 1) };
        assert_eq!(issue(&mut arena, advance), IntentOutcome::AppliedContinuous);
        for wall in (100..=6000).step_by(100) {
            arena.step_settled(wall, &[], &[]);
        }
        assert!(arena.minds[&MEMBER].think_seq > 0);
        assert_ne!(
            arena.unit(MEMBER).unwrap().pose,
            start,
            "the production mind steers the motor"
        );
        assert_eq!(held(&arena).unwrap().expires_ms, i64::MAX);
        let mut finite = command(1, 1, 2);
        finite.expires_at_ms = Some(arena.now_ms + 1000);
        assert_eq!(issue(&mut arena, finite), IntentOutcome::AppliedContinuous);
        for wall in (6100..=12000).step_by(100) {
            arena.step_settled(wall, &[], &[]);
        }
        assert!(held(&arena).is_none());
        assert!(
            arena.detachment_owns(MEMBER),
            "expiry cannot reactivate legacy authority"
        );
    }
}
