//! Diagnostic joins only. These methods never alter execution or persistent state.

use mush_core::ObjId;

use super::author_trace::{Cancelled, TraceUpdate};
use super::conditions::{Arming, Rule, RulePredicate, SlotTarget};
use super::intents::{IntentKind, MotorGoal};
use super::mind::{CollectedThink, MIND_INTENTS_MAX, MindView, ThinkResult};
use super::state::{Arena, Controller};

impl Arena {
    pub(crate) fn trace_cancel_pending(&mut self, unit: ObjId, reason: Cancelled) {
        let pending = self
            .minds
            .get(&unit)
            .and_then(|mind| mind.pending.as_ref())
            .map(|pending| pending.think_seq);
        let sequences = self
            .collected
            .iter()
            .chain(&self.unsettled)
            .filter(|think| think.unit == unit)
            .map(|think| think.think_seq)
            .chain(pending)
            .collect::<Vec<_>>();
        for seq in sequences {
            self.traces.cancel(unit, seq, self.now_ms, reason);
        }
    }

    pub(crate) fn trace_cancel_all(&mut self, reason: Cancelled) {
        for (unit, seq) in self.traces.pending() {
            self.traces.cancel(unit, seq, self.now_ms, reason);
        }
    }

    pub(crate) fn trace_complete(&mut self, think: &CollectedThink, result: &ThinkResult) {
        let stale = !self.awaiting.contains(&think.unit)
            || !self
                .profiles
                .get(&think.unit)
                .is_some_and(|runtime| runtime.accepts(think.profile_token))
            || !self
                .minds
                .get(&think.unit)
                .and_then(|mind| mind.pending.as_ref())
                .is_some_and(|pending| pending.think_seq == think.think_seq);
        if self.ended.is_some() || stale {
            let reason = if self.ended.is_some() {
                Cancelled::Ended
            } else {
                self.unit(think.unit).map_or(Cancelled::Removed, |own| {
                    if own.limits.wreck {
                        Cancelled::Wreck
                    } else if own.controller != Controller::Mind {
                        Cancelled::Controller
                    } else {
                        Cancelled::Superseded
                    }
                })
            };
            self.traces
                .cancel(think.unit, think.think_seq, self.now_ms, reason);
        }
        let accepted = self.traces.complete(
            think.unit,
            think.think_seq,
            result.diagnostics.clone(),
            &result.trace,
            &result.intents,
            result.fault.as_ref().map(|fault| fault.cause.as_str()),
        );
        if accepted {
            for (index, intent) in result.intents.iter().take(MIND_INTENTS_MAX).enumerate() {
                let detail = intent_words(intent, &think.view);
                self.traces
                    .intent_detail(think.unit, think.think_seq, index, &detail);
            }
        }
    }

    /// A cancelled job may no longer have its captured view. Retain closed action
    /// names and coarse faults, without inventing or resolving argument detail.
    pub(crate) fn trace_complete_late(&mut self, unit: ObjId, seq: u64, result: &ThinkResult) {
        let accepted = self.traces.complete(
            unit,
            seq,
            result.diagnostics.clone(),
            &result.trace,
            &result.intents,
            result.fault.as_ref().map(|fault| fault.cause.as_str()),
        );
        if accepted && result.fault.is_none() && result.intents.len() > MIND_INTENTS_MAX {
            self.traces.mark_fault(unit, seq, "intents");
        }
    }

    /// One normal update per sim second; missing/ended hulls lose pending outcomes
    /// here too, covering future removal paths without retaining another arena map.
    pub(crate) fn trace_updates(&mut self) -> Vec<TraceUpdate> {
        for (unit, seq) in self.traces.pending() {
            let reason = if self.ended.is_some() {
                Some(Cancelled::Ended)
            } else if self.unit(unit).is_none() {
                Some(Cancelled::Removed)
            } else {
                None
            };
            if let Some(reason) = reason {
                self.traces.cancel(unit, seq, self.now_ms, reason);
            }
        }
        self.traces.take_updates(self.now_ms)
    }
}

fn known_weapon(view: &MindView, weapon: &str) -> String {
    view.weapons
        .iter()
        .find(|row| row.id == weapon)
        .map_or_else(|| "unavailable weapon".to_owned(), |row| row.id.to_string())
}

fn safe_rule_words(rule: &Rule, view: &MindView) -> String {
    let terms = rule.predicates().iter().take(4).cloned().collect();
    let mut safe = match rule {
        Rule::Fire { condition, .. } => Rule::fire_rule(condition.clone(), terms),
        Rule::HoldFire {
            weapon, implicit, ..
        } => Rule::HoldFire {
            weapon: weapon.clone(),
            predicates: terms,
            implicit: *implicit,
        },
        Rule::Lock {
            selector,
            weapon,
            implicit,
            ..
        } => Rule::Lock {
            selector: *selector,
            weapon: weapon.clone(),
            predicates: terms,
            implicit: *implicit,
        },
    };
    let target = match rule {
        Rule::Fire { condition, .. } => match condition.target {
            SlotTarget::Contact { id } => view
                .contacts
                .iter()
                .find(|row| row.id == id)
                .map(|row| row.designator.to_string()),
            _ => None,
        },
        _ => None,
    };
    let predicates = match &mut safe {
        Rule::Fire {
            condition,
            predicates,
            ..
        } => {
            condition.weapon = known_weapon(view, &condition.weapon).into();
            predicates
        }
        Rule::HoldFire {
            weapon, predicates, ..
        } => {
            *weapon = known_weapon(view, weapon).into();
            predicates
        }
        Rule::Lock {
            weapon, predicates, ..
        } => {
            if let Some(id) = weapon {
                *id = known_weapon(view, id).into();
            }
            predicates
        }
    };
    predicates.truncate(4);
    for term in predicates {
        match term {
            RulePredicate::Odds {
                weapon: Some(id), ..
            }
            | RulePredicate::Rounds {
                weapon: Some(id), ..
            } => {
                *id = known_weapon(view, id).into();
            }
            RulePredicate::OwnSection { section, .. }
                if !view.sections.iter().any(|row| row.id == *section) =>
            {
                *section = "unavailable section".into();
            }
            RulePredicate::TargetSection { section, .. }
                if !view
                    .contacts
                    .iter()
                    .filter_map(|row| row.condition)
                    .filter_map(|reading| view.scan_tables.get(usize::from(reading.table)))
                    .any(|table| table.contains(section)) =>
            {
                *section = "unavailable section".into();
            }
            _ => {}
        }
    }
    super::rules::rule_words(&safe, target.as_deref())
}

fn intent_words(intent: &IntentKind, view: &MindView) -> String {
    match intent {
        IntentKind::Heading { mdeg } => format!("{} millidegrees", mdeg),
        IntentKind::Speed { cms } => format!("{cms} cm/s"),
        IntentKind::Startup => "power up".into(),
        IntentKind::Lock { contact } => view
            .contacts
            .iter()
            .find(|row| row.id == *contact)
            .map_or_else(
                || "unavailable contact".into(),
                |row| row.designator.to_string(),
            ),
        IntentKind::Fire { weapon, slot } => {
            let name = known_weapon(view, weapon);
            slot.map_or(name.clone(), |slot| format!("{name}, slot {slot}"))
        }
        IntentKind::Condition(Arming::Arm(condition)) => {
            safe_rule_words(&Rule::fire_rule(condition.clone(), Vec::new()), view)
        }
        IntentKind::Condition(Arming::Rules { slot, rules }) => {
            let words = rules
                .iter()
                .take(6)
                .map(|rule| safe_rule_words(rule, view))
                .collect::<Vec<_>>()
                .join("; ");
            slot.map_or(words.clone(), |slot| format!("slot {slot}: {words}"))
        }
        IntentKind::Condition(Arming::Disarm { weapon }) => weapon.as_ref().map_or_else(
            || "disarm mutable fire rules".into(),
            |weapon| format!("disarm {}", known_weapon(view, weapon)),
        ),
        IntentKind::Condition(Arming::Remove { slot, .. }) => format!("remove slot {slot}"),
        IntentKind::Goal(MotorGoal::Cell { x, y }) | IntentKind::Place { x, y } => {
            format!("cell {x},{y}")
        }
        IntentKind::Goal(MotorGoal::Hold) => "hold here".into(),
        IntentKind::Goal(MotorGoal::Off) => "release motor".into(),
        IntentKind::Emcon { on } => if *on { "on" } else { "off" }.into(),
        IntentKind::Profile { index } => format!("profile {}", u16::from(*index) + 1),
        // A mind cannot issue these. No command/private object payload is echoed.
        IntentKind::Order(_) => "standing order".into(),
        IntentKind::ReleaseOrder { .. } => "release standing order".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::author_trace::{Fault, Outcome, RomDrop};
    use super::super::mind::{ThinkAnswer, ThinkFault};
    use super::super::motor::MotorCounters;
    use super::super::state::{Channel, ContactId, Power, Team};
    use super::super::test_support::{arena_of, line_tank, pose};
    use super::*;

    const OWN: ObjId = ObjId(4);
    fn arena() -> Arena {
        let mut own = line_tank(OWN, Team::Red, pose(1500, 4500));
        own.controller = Controller::Mind;
        own.reseal(&crate::match_::SealTerms::fixture())
            .expect("fixture seal");
        arena_of(vec![own, line_tank(ObjId(9), Team::Blue, pose(6500, 4500))])
    }
    fn collect(arena: &mut Arena, t: i64) -> CollectedThink {
        arena.now_ms = t;
        let seq = arena.minds[&OWN].think_seq;
        arena.collect_think(OWN, seq, t, &mut MotorCounters::default());
        arena.collected.pop().expect("actual collection")
    }
    fn answer() -> ThinkResult {
        ThinkResult {
            intents: vec![
                IntentKind::Heading { mdeg: 90_000 },
                IntentKind::Fire {
                    weapon: "cannon".into(),
                    slot: None,
                },
            ],
            memory: b"[1]".to_vec(),
            trace: vec!["deliberate own line".into()],
            ..ThinkResult::default()
        }
    }

    #[test]
    fn real_landing_joins_applied_and_refused_intents_to_the_exact_think() {
        let mut arena = arena();
        arena.unit_mut(OWN).expect("own").power = Power::Online;
        let think = collect(&mut arena, 0);
        assert_eq!(arena.settle_collected(&think, answer()).written, 1);
        arena.now_ms = think.lands_ms;
        arena.mind_land(
            OWN,
            think.think_seq,
            think.lands_ms,
            &mut MotorCounters::default(),
        );
        let trace = arena
            .traces
            .recent(OWN, None)
            .pop()
            .expect("retained think");
        assert_eq!(trace.think_seq, think.think_seq);
        assert_eq!(trace.landed_ms, Some(think.lands_ms));
        assert_eq!(trace.outcome, Outcome::Applied);
        assert_eq!(trace.intents[0].outcome, Outcome::Applied);
        assert!(
            matches!(&trace.intents[1].outcome, Outcome::Refused { reason } if reason == "no_lock")
        );
        assert_eq!(arena.minds[&OWN].memory, b"[1]");
    }

    #[test]
    fn actual_epoch_loss_and_controller_cancel_survive_late_worker_faults() {
        let mut arena = arena();
        let think = collect(&mut arena, 0);
        arena.settle_collected(&think, answer());
        arena.epochs.raise(OWN, Channel::Helm, 1);
        arena.now_ms = think.lands_ms;
        arena.mind_land(
            OWN,
            think.think_seq,
            think.lands_ms,
            &mut MotorCounters::default(),
        );
        assert!(matches!(
            arena.traces.recent(OWN, None)[0].outcome,
            Outcome::Cancelled {
                reason: Cancelled::Epoch
            }
        ));
        let next = collect(&mut arena, 1000);
        arena.set_controller(
            OWN,
            Controller::Pilot,
            super::super::doctrine::MachineState::Startup,
        );
        let mut result = ThinkResult {
            fault: Some(ThinkFault {
                cause: "private source and memory".into(),
            }),
            ..answer()
        };
        result
            .intents
            .resize(MIND_INTENTS_MAX + 1, IntentKind::Startup);
        assert_eq!(
            arena
                .deliver_answers(vec![ThinkAnswer {
                    unit: OWN,
                    think_seq: next.think_seq,
                    result
                }])
                .dropped,
            1
        );
        let rows = arena.traces.recent(OWN, None);
        let late = rows.last().expect("late row");
        assert!(late.settled);
        assert_eq!(late.fault, Some(Fault::Unknown));
        assert!(matches!(
            late.outcome,
            Outcome::Cancelled {
                reason: Cancelled::Controller
            }
        ));
        let encoded = serde_json::to_string(late).expect("diagnostics JSON");
        assert!(!encoded.contains("private source and memory"));
    }

    #[test]
    fn third_real_fault_marks_only_the_triggering_think_as_rom_drop() {
        let mut arena = arena();
        for n in 0..3 {
            let think = collect(&mut arena, n * 1000);
            let result = ThinkResult {
                fault: Some(ThinkFault {
                    cause: "ops".into(),
                }),
                ..ThinkResult::default()
            };
            assert_eq!(arena.settle_collected(&think, result).faults, 1);
        }
        let rows = arena.traces.recent(OWN, None);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].rom_drop, None);
        assert_eq!(rows[1].rom_drop, None);
        assert_eq!(rows[2].rom_drop, Some(RomDrop::Strikes));
        assert!(
            rows.iter()
                .all(|row| matches!(row.outcome, Outcome::Faulted { fault: Fault::Ops }))
        );
        let next = collect(&mut arena, 3000);
        assert!(next.dropped);
        assert!(
            arena
                .traces
                .recent(OWN, None)
                .last()
                .expect("ROM think")
                .rom
        );
    }

    #[test]
    fn trace_on_off_has_identical_every_substep_hash_memory_and_snapshot_bytes() {
        let mut on = arena();
        let mut off = arena();
        off.traces.set_enabled(false);
        for wall in (0..=6000).step_by(250) {
            on.step_settled(wall, &[], &[]);
            off.step_settled(wall, &[], &[]);
            assert_eq!(on.substep_hashes, off.substep_hashes);
            assert_eq!(
                super::super::hash::state_hash(&on),
                super::super::hash::state_hash(&off)
            );
            assert_eq!(on.minds[&OWN].memory, off.minds[&OWN].memory);
            if on.ended.is_some() {
                break;
            }
        }
        assert!(!on.traces.recent(OWN, None).is_empty());
        assert!(off.traces.recent(OWN, None).is_empty());
        let on_snapshot = super::super::snapshot::Snapshot::encode(&on).expect("settled snapshot");
        let off_snapshot =
            super::super::snapshot::Snapshot::encode(&off).expect("settled snapshot");
        assert_eq!(on_snapshot.bytes(), off_snapshot.bytes());
        let restored = super::super::snapshot::Snapshot::decode(
            &on_snapshot.document,
            (*on.definition).clone(),
        )
        .expect("resume");
        assert!(restored.traces.recent(OWN, Some(64)).is_empty());
    }

    #[test]
    fn argument_projection_never_echoes_unknown_identifiers() {
        let mut arena = arena();
        let think = collect(&mut arena, 0);
        let view = &think.view;
        assert_eq!(
            intent_words(
                &IntentKind::Lock {
                    contact: ContactId(u64::MAX)
                },
                view
            ),
            "unavailable contact"
        );
        assert_eq!(
            intent_words(
                &IntentKind::Fire {
                    weapon: "private memory".into(),
                    slot: None
                },
                view
            ),
            "unavailable weapon"
        );
    }

    #[test]
    fn target_section_words_use_the_earned_target_table_not_own_chassis() {
        let mut arena = arena();
        let plot = super::super::state::SeenPose::of(arena.unit(ObjId(9)).expect("target"));
        arena.contacts.insert(
            OWN,
            ContactId(42),
            super::super::state::ShadowContact::sighted(
                ObjId(9),
                super::super::state::Classification::Classified { team: Team::Blue },
                0,
                plot,
            ),
        );
        let think = collect(&mut arena, 0);
        let mut view = think.view.as_ref().clone();
        assert!(!view.contacts.is_empty());
        assert!(!view.sections.iter().any(|row| row.id == "wing"));
        view.scan_tables = vec![vec!["wing".into()]];
        view.contacts[0].condition = Some(super::super::perception::FrameCondition {
            table: 0,
            bands: super::super::condition::Condition {
                bands: 2,
                sections: 1,
            },
            age_ms: 0,
        });
        let rule = Rule::HoldFire {
            weapon: "cannon".into(),
            predicates: vec![RulePredicate::TargetSection {
                section: "wing".into(),
                state: super::super::conditions::SectionState::Red,
            }],
            implicit: false,
        };
        let words = safe_rule_words(&rule, &view);
        assert!(words.contains("wing"));
        assert!(!words.contains("unavailable section"));
        view.contacts[0].condition = None;
        assert!(safe_rule_words(&rule, &view).contains("unavailable section"));
    }

    #[test]
    fn wreck_cancel_and_post_landing_core_drop_name_their_actual_think() {
        let mut wreck = arena();
        let think = collect(&mut wreck, 0);
        wreck.now_ms = 10;
        wreck.destroy_unit(OWN, 10, None, &mut Vec::new());
        wreck.settle_collected(&think, answer());
        assert!(matches!(
            wreck.traces.recent(OWN, None)[0].outcome,
            Outcome::Cancelled {
                reason: Cancelled::Wreck
            }
        ));

        let mut core = arena();
        let think = collect(&mut core, 0);
        core.settle_collected(&think, ThinkResult::default());
        let key = core
            .unit(OWN)
            .expect("own")
            .unit
            .capabilities()
            .find(|key| key.id.as_str() == crate::definitions::unit::CORE_CAPABILITY)
            .cloned()
            .expect("core");
        let plan = mush_tactics::UnitMutationPlan::new(
            vec![mush_tactics::UnitMutationOp::RevokeCapability { key }],
            None,
            None,
        )
        .expect("revoke plan");
        let own = core.unit_mut(OWN).expect("own");
        own.unit = mush_tactics::apply(&own.unit, &plan, 0).expect("revoke").0;
        core.now_ms = think.lands_ms;
        core.mind_land(
            OWN,
            think.think_seq,
            think.lands_ms,
            &mut MotorCounters::default(),
        );
        let rows = core.traces.recent(OWN, None);
        assert_eq!(rows[0].outcome, Outcome::Applied);
        assert_eq!(rows[0].rom_drop, Some(RomDrop::Core));
    }
}
