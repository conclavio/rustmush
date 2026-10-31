//! One private mutable timeline. Its reference never enters this owner.
use super::{BulkError, invalid};
use crate::arena::{
    Arena, ArenaDefinition, Snapshot,
    clock::{Clock, ReplayClock},
    intents::{
        Control, ControlState, Intent, IntentKind, IntentOutcome, IntentSource, MotorGoal,
        apply_control, apply_intent,
    },
    mind::MindRunner,
    step::OwnedPulse,
};
use mush_core::ObjId;
use serde::Serialize;
use std::{collections::BTreeMap, sync::Arc};
const CHECKPOINT_BYTES: u64 = 16 * 1024 * 1024;
pub(super) const RESERVATION_BYTES: u64 = 2 * CHECKPOINT_BYTES + 1024 * 1024;
const INPUT_BYTES: usize = 1024 * 1024;
const INPUT_COUNT: usize = 256;
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Timeline {
    Reference,
    Fork {
        serial: String,
        parent_event: usize,
        from_ms: i64,
    },
}
#[derive(Clone, Debug, Serialize)]
struct Input {
    at: i64,
    controls: Vec<Control>,
    intent: Option<Intent>,
}
pub(super) struct Branch {
    pub timeline: Timeline,
    pub memory_allowed: bool,
    pub unit: ObjId,
    actor: ObjId,
    opening: Snapshot,
    clock: ReplayClock,
    ground: Arc<ArenaDefinition>,
    runners: BTreeMap<ObjId, Arc<dyn MindRunner>>,
    default_runner: Arc<dyn MindRunner>,
    arena: Option<Arena>,
    pulse: Option<OwnedPulse>,
    wall: i64,
    inputs: Vec<Input>,
    cursor: usize,
    possessed: bool,
    pub refusals: Vec<String>,
}
impl Branch {
    pub fn preflight(arena: &Arena) -> Result<(), BulkError> {
        let budget = crate::arena::snapshot_capacity::admit(arena)
            .map_err(|_| invalid("fork state capacity"))?;
        if budget.total > CHECKPOINT_BYTES {
            return Err(invalid("fork state exceeds the reserved capacity"));
        }
        Snapshot::encode_with_limit(arena, CHECKPOINT_BYTES)
            .map_err(|_| invalid("fork requires a bounded settled checkpoint"))?;
        Ok(())
    }
    /// Caller reserves RESERVATION_BYTES before cloning; every possible later
    /// state must fit the snapshot admission envelope, not only its opening.
    pub fn new(
        mut arena: Arena,
        actor: ObjId,
        unit: ObjId,
        serial: u64,
        parent_event: usize,
        memory_allowed: bool,
    ) -> Result<Box<Self>, BulkError> {
        arena.arena_cfg.snapshot_max_bytes = CHECKPOINT_BYTES;
        crate::arena::snapshot_capacity::admit(&arena)
            .map_err(|_| invalid("fork state exceeds the reserved capacity"))?;
        let opening = Snapshot::encode_with_limit(&arena, CHECKPOINT_BYTES)
            .map_err(|_| invalid("fork requires a bounded settled checkpoint"))?;
        arena.tape_witnesses.disable();
        let clock = arena.clock.replay_state();
        let wall = arena.clock.replay_wall_ms().unwrap_or(0);
        Ok(Box::new(Self {
            timeline: Timeline::Fork {
                serial: serial.to_string(),
                parent_event,
                from_ms: arena.now_ms,
            },
            memory_allowed,
            unit,
            actor,
            ground: Arc::clone(&arena.definition),
            runners: arena.runners.clone(),
            default_runner: Arc::clone(&arena.mind_runner),
            opening,
            clock,
            arena: Some(arena),
            pulse: None,
            wall,
            inputs: Vec::new(),
            cursor: 0,
            possessed: false,
            refusals: Vec::new(),
        }))
    }
    pub fn arena(&self) -> Option<&Arena> {
        self.arena.as_ref()
    }
    pub fn opening_ms(&self) -> i64 {
        self.opening.at_ms
    }
    pub fn possessed(&self) -> bool {
        self.possessed
    }
    pub fn next_simulation_ms(&self) -> u64 {
        self.pulse
            .as_ref()
            .map_or(0, |p| u64::from(p.next_simulation_ms()))
    }
    pub fn rewind(&mut self) -> Result<(), BulkError> {
        if self.pulse.is_some() {
            return Err(BulkError::Busy);
        }
        let mut arena = Snapshot::decode(&self.opening.document, (*self.ground).clone())
            .map_err(|_| invalid("fork checkpoint refused"))?;
        // Preserve the exact immutable compiled source set at this checkpoint,
        // including a swapped candidate. No original-header runner can replace it.
        arena.runners = self.runners.clone();
        arena.mind_runner = Arc::clone(&self.default_runner);
        arena.clock = Clock::from_replay(self.clock);
        arena.tape_witnesses.disable();
        self.wall = arena.clock.replay_wall_ms().unwrap_or(0);
        self.arena = Some(arena);
        self.cursor = 0;
        self.possessed = false;
        self.refusals.clear();
        Ok(())
    }
    fn apply(&mut self, input: &Input) -> Result<(), BulkError> {
        let arena = self.arena.as_mut().ok_or(BulkError::Busy)?;
        for control in &input.controls {
            apply_control(arena, control);
            self.possessed = control.state == ControlState::Live;
        }
        if let Some(intent) = &input.intent
            && let IntentOutcome::Refused(reason) = apply_intent(arena, intent, IntentSource::Seat)
        {
            self.refusals
                .push(format!("{} refused: {}.", intent.kind.as_str(), reason));
            if self.refusals.len() > 8 {
                self.refusals.remove(0);
            }
        }
        Ok(())
    }
    fn append(&mut self, input: Input) -> Result<(), BulkError> {
        if self.pulse.is_some() {
            return Err(BulkError::Busy);
        }
        if self.cursor >= INPUT_COUNT {
            return Err(invalid("fork command history is full; create a new fork"));
        }
        let mut rows = self
            .inputs
            .get(..self.cursor)
            .ok_or_else(|| invalid("fork command cursor"))?
            .to_vec();
        rows.push(input.clone());
        super::encoding::encode(&rows, INPUT_BYTES)?;
        self.apply(&input)?;
        self.inputs = rows;
        self.cursor += 1;
        Ok(())
    }
    pub fn possess(&mut self, on: bool) -> Result<(), BulkError> {
        let arena = self.arena().ok_or(BulkError::Busy)?;
        let own = arena.unit(self.unit).ok_or(BulkError::Missing)?;
        if self.possessed == on {
            return Err(invalid(if on {
                "fork is already possessed"
            } else {
                "fork is not possessed"
            }));
        }
        let controls = own
            .seal
            .channels
            .iter()
            .map(|channel| {
                Ok(Control {
                    unit: self.unit,
                    channel: *channel,
                    epoch: arena
                        .epochs
                        .current(self.unit, *channel)
                        .checked_add(1)
                        .ok_or_else(|| invalid("station epoch exhausted"))?,
                    state: if on {
                        ControlState::Live
                    } else {
                        ControlState::Released
                    },
                    pilot: Some(self.actor),
                })
            })
            .collect::<Result<Vec<_>, BulkError>>()?;
        self.append(Input {
            at: arena.now_ms,
            controls,
            intent: None,
        })
    }
    pub fn command(&mut self, text: &str) -> Result<(), BulkError> {
        if !self.possessed() {
            return Err(invalid("BENCH POSSESS before station commands"));
        }
        let arena = self.arena().ok_or(BulkError::Busy)?;
        let kind = parse_station(arena, self.unit, text)?;
        let channel = kind.channel_required();
        if !arena
            .unit(self.unit)
            .ok_or(BulkError::Missing)?
            .seal
            .channels
            .contains(&channel)
        {
            return Err(invalid("this hull has no such station"));
        }
        let input = Input {
            at: arena.now_ms,
            controls: Vec::new(),
            intent: Some(Intent {
                unit: self.unit,
                channel,
                epoch: arena.epochs.current(self.unit, channel),
                kind,
                seq: self.cursor as u64 + 1,
            }),
        };
        self.append(input)
    }
    pub fn advance(&mut self, maximum: u64) -> Result<u64, BulkError> {
        if let Some(mut pulse) = self.pulse.take() {
            if u64::from(pulse.next_simulation_ms()) > maximum {
                self.pulse = Some(pulse);
                return Err(invalid("fork quantum differs from lease"));
            }
            let slice = pulse.advance();
            if !slice.done {
                self.pulse = Some(pulse);
                return Ok(u64::from(slice.simulated_ms));
            }
            self.land(pulse)?;
            return Ok(u64::from(slice.simulated_ms));
        }
        let at = self.arena().ok_or(BulkError::Busy)?.now_ms;
        while let Some(input) = self
            .inputs
            .get(self.cursor)
            .filter(|row| row.at <= at)
            .cloned()
        {
            self.apply(&input)?;
            self.cursor += 1;
        }
        let arena = self.arena.take().ok_or(BulkError::Busy)?;
        self.wall = self
            .wall
            .checked_add(arena.substep_len_ms())
            .ok_or_else(|| invalid("fork clock exhausted"))?;
        self.pulse = Some(OwnedPulse::new(arena, self.wall, &[], &[]));
        Ok(0)
    }

    /// **Land a finished pulse** (split out of `BranchRun::advance` by doc 32
    /// cut CF.0): the finished arena and its output live in this frame alone,
    /// so `advance`'s frame never holds the in-flight pulse and the finished
    /// arena at once (`clippy::large_stack_frames`, 64 KiB).
    #[inline(never)]
    fn land(&mut self, pulse: OwnedPulse) -> Result<(), BulkError> {
        let (mut arena, output) = pulse
            .finish()
            .map_err(|_| invalid("fork pulse incomplete"))?;
        arena.settle_inline();
        if let Some(chunk) = output.ledger_chunks.last() {
            arena.ledger.mark_flushed(chunk.seq);
        }
        for (_, frame) in output
            .frames
            .into_iter()
            .filter(|(_, f)| f.unit == self.unit)
        {
            for notice in frame.notices {
                if let crate::arena::perception::FrameNotice::Refused { order, refusal, .. } =
                    notice
                {
                    self.refusals.push(format!("{order} refused: {refusal}."));
                }
            }
        }
        if self.refusals.len() > 8 {
            self.refusals.drain(..self.refusals.len() - 8);
        }
        self.arena = Some(arena);
        Ok(())
    }
}

fn parse_station(arena: &Arena, unit: ObjId, text: &str) -> Result<IntentKind, BulkError> {
    if text.is_empty() || text.len() > 1024 || text.chars().any(char::is_control) {
        return Err(invalid("bounded station command required"));
    }
    let (verb, tail) = text
        .split_once(char::is_whitespace)
        .map_or((text, ""), |(a, b)| (a, b.trim()));
    let frame = arena
        .frame_for(unit, arena.clock.tempo())
        .ok_or(BulkError::Missing)?;
    let words = tail.split_whitespace().collect::<Vec<_>>();
    let own = arena.unit(unit).ok_or(BulkError::Missing)?;
    match verb.to_ascii_lowercase().as_str() {
        "startup" if tail.is_empty() => Ok(IntentKind::Startup),
        "heading" => Ok(IntentKind::Heading {
            mdeg: tail
                .parse::<u32>()
                .ok()
                .filter(|n| *n < 360)
                .ok_or_else(|| invalid("HEADING <0-359>"))?
                * 1000,
        }),
        "speed" => {
            let kph = tail.parse::<i32>().map_err(|_| invalid("SPEED <kph>"))?;
            let max = i64::from(own.mobility.max_speed_cms) * 36 / 1000;
            let reverse = i64::from(own.mobility.reverse_ceiling_cms()) * 36 / 1000;
            if i64::from(kph) > max || i64::from(kph) < -reverse {
                return Err(invalid(
                    "speed exceeds this hull's forward or reverse ceiling",
                ));
            }
            Ok(IntentKind::Speed {
                cms: kph.saturating_mul(100_000) / 3600,
            })
        }
        "emcon" if tail.eq_ignore_ascii_case("on") || tail.eq_ignore_ascii_case("off") => {
            Ok(IntentKind::Emcon {
                on: tail.eq_ignore_ascii_case("on"),
            })
        }
        "drive" => {
            let goal = if tail.eq_ignore_ascii_case("hold") {
                MotorGoal::Hold
            } else if tail.eq_ignore_ascii_case("off") {
                MotorGoal::Off
            } else {
                let (x, y) = tail
                    .split_once(',')
                    .ok_or_else(|| invalid("DRIVE <x,y>|HOLD|OFF"))?;
                let x = x.trim().parse::<u16>().map_err(|_| invalid("DRIVE cell"))?;
                let y = y.trim().parse::<u16>().map_err(|_| invalid("DRIVE cell"))?;
                if x >= arena.definition.terrain.width() || y >= arena.definition.terrain.height() {
                    return Err(invalid("DRIVE cell is outside this window"));
                }
                MotorGoal::Cell { x, y }
            };
            Ok(IntentKind::Goal(goal))
        }
        "mind"
            if words
                .first()
                .is_some_and(|s| s.eq_ignore_ascii_case("profile")) =>
        {
            let name = words
                .get(1)
                .filter(|_| words.len() == 2)
                .ok_or_else(|| invalid("MIND PROFILE <name>"))?;
            let index = own
                .seal
                .profiles
                .names()
                .position(|n| n.eq_ignore_ascii_case(name))
                .ok_or(BulkError::Missing)?;
            Ok(IntentKind::Profile { index: index as u8 })
        }
        "lock" if words.len() == 1 => {
            let contact = frame
                .contacts
                .iter()
                .find(|c| {
                    c.designator.to_string().eq_ignore_ascii_case(tail)
                        || c.alias
                            .is_some_and(|n| format!("X{n}").eq_ignore_ascii_case(tail))
                })
                .ok_or_else(|| invalid("that contact is not on your plot"))?;
            Ok(IntentKind::Lock {
                contact: contact.id,
            })
        }
        "fire" if words.len() <= 1 && !tail.eq_ignore_ascii_case("off") => {
            let row = if tail.is_empty() {
                frame.weapons.first()
            } else {
                let matches = frame
                    .weapons
                    .iter()
                    .filter(|w| {
                        w.id.eq_ignore_ascii_case(tail)
                            || w.id
                                .to_ascii_lowercase()
                                .starts_with(&tail.to_ascii_lowercase())
                            || crate::match_::cockpit::weapon_initials(&w.id)
                                .is_some_and(|a| a.eq_ignore_ascii_case(tail))
                    })
                    .collect::<Vec<_>>();
                if matches.len() != 1 {
                    return Err(invalid("name one carried weapon"));
                }
                matches.first().copied()
            }
            .ok_or_else(|| invalid("this unit carries no weapon"))?;
            Ok(IntentKind::Fire {
                weapon: row.id.clone(),
                slot: None,
            })
        }
        "fire" | "lock" | "mind" => {
            let weapons = frame
                .weapons
                .iter()
                .map(|w| crate::arena::rules::RuleWeapon {
                    id: w.id.clone(),
                    rounds_max: w.rounds_max,
                })
                .collect::<Vec<_>>();
            let contacts = frame
                .contacts
                .iter()
                .flat_map(|c| {
                    let mut rows = vec![crate::arena::rules::RuleContact {
                        name: c.designator.to_string().into(),
                        id: c.id,
                        classified: c.class.party().is_some(),
                    }];
                    if let Some(n) = c.alias {
                        rows.push(crate::arena::rules::RuleContact {
                            name: format!("X{n}").into(),
                            id: c.id,
                            classified: c.class.party().is_some(),
                        });
                    }
                    rows
                })
                .collect::<Vec<_>>();
            let context = crate::arena::rules::RuleContext {
                weapons: &weapons,
                contacts: &contacts,
                floor_permille: u16::try_from(arena.odds_floor_permille)
                    .map_err(|_| invalid("Compact floor"))?,
            };
            let text = if verb.eq_ignore_ascii_case("mind") {
                tail
            } else {
                text
            };
            match crate::arena::rules::parse_command(text, &context)
                .map_err(|e| invalid(e.to_string()))?
            {
                crate::arena::rules::RuleCommand::Arm(arming) => Ok(IntentKind::Condition(arming)),
                crate::arena::rules::RuleCommand::Remove { slot } => {
                    let row = frame
                        .rules
                        .iter()
                        .find(|r| r.slot == slot)
                        .ok_or(BulkError::Missing)?;
                    Ok(IntentKind::Condition(
                        crate::arena::conditions::Arming::Remove {
                            slot,
                            channel: row.rule.channel(),
                        },
                    ))
                }
                _ => Err(invalid("use BENCH PIC to read the current program")),
            }
        }
        _ => Err(invalid(
            "station command is STARTUP, HEADING, SPEED, DRIVE, LOCK, FIRE, EMCON, or MIND PROFILE/RULE",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::{
        Channel, Controller, PartyId,
        test_support::{arena_of, pose, tank},
    };
    fn branch() -> Box<Branch> {
        let mut red = tank(ObjId(4), PartyId::RED, pose(1500, 4500));
        red.weapons.push(crate::arena::test_support::cannon());
        red.controller = Controller::Mind;
        red.reseal(&crate::match_::SealTerms::fixture()).unwrap();
        let arena = arena_of(vec![red, tank(ObjId(9), PartyId::BLUE, pose(6500, 4500))]);
        Branch::new(arena, ObjId(99), ObjId(4), 1, 0, true).unwrap()
    }
    #[test]
    fn possession_uses_current_epochs_and_old_orders_cannot_land() {
        let mut branch = branch();
        let old = branch
            .arena()
            .unwrap()
            .epochs
            .current(ObjId(4), Channel::Helm);
        assert!(crate::test_checks::is_refusal(branch.command("STARTUP")));
        branch.possess(true).unwrap();
        let arena = branch.arena.as_mut().unwrap();
        assert_eq!(arena.unit(ObjId(4)).unwrap().controller, Controller::Pilot);
        assert_eq!(arena.epochs.current(ObjId(4), Channel::Helm), old + 1);
        let stale = Intent {
            unit: ObjId(4),
            channel: Channel::Helm,
            epoch: old,
            seq: 1,
            kind: IntentKind::Startup,
        };
        assert_eq!(
            apply_intent(arena, &stale, IntentSource::Seat),
            IntentOutcome::Refused(crate::arena::intents::Refusal::Epoch)
        );
        branch.possess(false).unwrap();
        assert_eq!(
            branch.arena().unwrap().unit(ObjId(4)).unwrap().controller,
            Controller::Mind
        );
        assert!(!branch.possessed());
        assert!(crate::test_checks::is_refusal(branch.command("HEADING 90")));
    }
    #[test]
    fn station_parser_uses_only_own_carried_weapons_and_visible_contacts() {
        let branch = branch();
        let arena = branch.arena().unwrap();
        assert!(parse_station(arena, ObjId(4), "LOCK BA").is_err());
        assert!(parse_station(arena, ObjId(4), "FIRE imaginary").is_err());
        assert!(parse_station(arena, ObjId(4), "SPEED 99999").is_err());
        assert!(parse_station(arena, ObjId(4), "HEADING 360").is_err());
        assert!(parse_station(arena, ObjId(4), "DRIVE 999,999").is_err());
        assert!(parse_station(arena, ObjId(4), "@arena/place 3 4").is_err());
        assert_eq!(
            parse_station(arena, ObjId(4), "HEADING 90").unwrap(),
            IntentKind::Heading { mdeg: 90000 }
        );
        assert!(matches!(
            parse_station(arena, ObjId(4), "FIRE cannon WHEN odds >= 60").unwrap(),
            IntentKind::Condition(_)
        ));
    }
    #[test]
    fn command_history_is_bounded_and_a_refusal_preserves_it() {
        let mut branch = branch();
        branch.possess(true).unwrap();
        for _ in 1..INPUT_COUNT {
            branch.command("HEADING 90").unwrap();
        }
        let hash = crate::arena::state_hash(branch.arena().unwrap());
        assert!(crate::test_checks::is_refusal(
            branch.command("HEADING 180")
        ));
        assert_eq!(branch.inputs.len(), INPUT_COUNT);
        assert_eq!(crate::arena::state_hash(branch.arena().unwrap()), hash);
    }
}
