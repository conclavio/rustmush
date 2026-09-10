//! A local harness dispatch debugger. It holds phase boundaries, not the game,
//! SSH readers, individual goals, or failure monitors.
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{
    sync::{mpsc, watch},
    time::Instant,
};

use crate::{Result, experiment::load_json};

fn version() -> u32 {
    1
}
fn held() -> bool {
    true
}
fn hold_limit() -> u64 {
    300_000
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    #[serde(default = "version")]
    pub version: u32,
    #[serde(default = "held")]
    pub start_held: bool,
    #[serde(default)]
    pub before_phases: Vec<usize>,
    #[serde(default = "hold_limit")]
    pub max_hold_ms: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            start_held: true,
            before_phases: vec![],
            max_hold_ms: hold_limit(),
        }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Result<Self> {
        let settings: Self = load_json(path, 32_000)?;
        settings.validate()?;
        Ok(settings)
    }

    pub fn validate(&self) -> Result<()> {
        if self.version != 1
            || !(100..=3_600_000).contains(&self.max_hold_ms)
            || self.before_phases.len() > 1000
        {
            return Err("cast debugger needs version 1, at most 1000 breakpoints, and max_hold_ms in 100..3600000".into());
        }
        let mut seen = BTreeSet::new();
        if self
            .before_phases
            .iter()
            .any(|n| !(1..=1000).contains(n) || !seen.insert(*n))
        {
            return Err(
                "cast debugger before_phases must be unique phase numbers in 1..1000".into(),
            );
        }
        Ok(())
    }

    pub fn validate_phases(&self, phase_count: usize) -> Result<()> {
        self.validate()?;
        if self.before_phases.iter().any(|n| *n > phase_count) {
            return Err("cast debugger breakpoint names a phase absent from the manifest".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Hold,
    Continue,
    Step,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub run: usize,
    pub revision: u64,
    pub action: Action,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Running,
    HoldRequested,
    Held,
    Stepping,
    Stopping,
    #[default]
    Finished,
}

#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub run: usize,
    pub revision: u64,
    pub mode: Mode,
    /// The next phase to dispatch; None means the final cleanup boundary.
    pub next_phase: Option<usize>,
    pub phase_count: usize,
    /// Time in the current boundary hold. Reports separately retain total held time.
    pub held_ms: u64,
    pub max_hold_ms: u64,
    pub notice: &'static str,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            run: 0,
            revision: 0,
            mode: Mode::Finished,
            next_phase: None,
            phase_count: 0,
            held_ms: 0,
            max_hold_ms: hold_limit(),
            notice: "idle",
        }
    }
}

pub struct Controller {
    pub settings: Settings,
    pub commands: mpsc::Receiver<Command>,
    pub state: watch::Sender<Snapshot>,
    /// Library hosts can explicitly name the bot executable; normal CLI/TUI use
    /// the running executable. This never selects a server or privileged API.
    pub bot_program: Option<PathBuf>,
}

pub(crate) struct Session {
    settings: Settings,
    state: watch::Sender<Snapshot>,
    snapshot: Snapshot,
    hold_requested: bool,
    step_armed: bool,
    boundary_open: Option<usize>,
    boundary_held: Option<usize>,
    held_since: Option<Instant>,
    held_total: Duration,
    holds: u64,
    accepted_commands: u64,
    rejected_commands: u64,
    accepting_commands: bool,
}

impl Session {
    pub(crate) fn new(
        settings: &Settings,
        state: &watch::Sender<Snapshot>,
        run: usize,
        phase_count: usize,
    ) -> Result<Self> {
        settings.validate_phases(phase_count)?;
        let revision = state.borrow().revision;
        let mut session = Self {
            settings: settings.clone(),
            state: state.clone(),
            snapshot: Snapshot {
                run,
                revision,
                mode: if settings.start_held {
                    Mode::HoldRequested
                } else {
                    Mode::Running
                },
                next_phase: (phase_count > 0).then_some(1),
                phase_count,
                held_ms: 0,
                max_hold_ms: settings.max_hold_ms,
                notice: "run_start",
            },
            hold_requested: settings.start_held,
            step_armed: false,
            boundary_open: None,
            boundary_held: None,
            held_since: None,
            held_total: Duration::ZERO,
            holds: 0,
            accepted_commands: 0,
            rejected_commands: 0,
            accepting_commands: true,
        };
        session.publish("run_start")?;
        Ok(session)
    }

    fn publish(&mut self, notice: &'static str) -> Result<()> {
        self.snapshot.revision = self
            .snapshot
            .revision
            .checked_add(1)
            .ok_or("cast debugger revision capacity exceeded")?;
        self.snapshot.notice = notice;
        self.state.send_replace(self.snapshot.clone());
        Ok(())
    }

    pub(crate) fn event(
        &self,
        transition: &'static str,
        accepted: Option<bool>,
        action: Option<Action>,
        reason: &'static str,
    ) -> Value {
        json!({"kind":"cast-debug","data":{"transition":transition,"accepted":accepted,"action":action,"reason":reason,"state":self.snapshot}})
    }

    pub(crate) fn refresh(&mut self, now: Instant) {
        let elapsed = self
            .held_since
            .map_or(0, |at| now.saturating_duration_since(at).as_millis() as u64);
        if elapsed != self.snapshot.held_ms {
            self.snapshot.held_ms = elapsed;
            self.state.send_replace(self.snapshot.clone());
        }
    }

    pub(crate) fn held_duration(&self, now: Instant) -> Duration {
        self.held_total
            + self
                .held_since
                .map_or(Duration::ZERO, |at| now.saturating_duration_since(at))
    }

    pub(crate) fn hold_expired(&self, now: Instant) -> bool {
        self.held_since.is_some_and(|at| {
            now.saturating_duration_since(at) >= Duration::from_millis(self.settings.max_hold_ms)
        })
    }

    fn release(&mut self, now: Instant) {
        if let Some(at) = self.held_since.take() {
            self.held_total += now.saturating_duration_since(at);
        }
        self.snapshot.held_ms = 0;
        if let Some(boundary) = self.boundary_held.take() {
            self.boundary_open = Some(boundary);
        }
    }

    /// Every accepted control changes revision. A repeated queued click carrying
    /// the previous revision cannot release a later gate or another matrix run.
    pub(crate) fn command(&mut self, command: Command, now: Instant) -> Result<Value> {
        self.refresh(now);
        let rejection = if command.run != self.snapshot.run {
            Some("stale_run")
        } else if command.revision != self.snapshot.revision {
            Some("stale_revision")
        } else if !self.accepting_commands {
            Some(if self.snapshot.mode == Mode::Stopping {
                "cleaning_up"
            } else {
                "finished"
            })
        } else if self.hold_expired(now) {
            Some("hold_expired")
        } else {
            match (command.action, self.snapshot.mode) {
                (_, Mode::Stopping) => Some("cleaning_up"),
                (_, Mode::Finished) => Some("finished"),
                (Action::Hold, Mode::Held) => Some("already_held"),
                (Action::Hold, Mode::HoldRequested) => Some("hold_already_requested"),
                (Action::Continue, Mode::Running) => Some("already_running"),
                (Action::Step, mode) if mode != Mode::Held => Some("step_requires_held"),
                (Action::Step, _) if self.snapshot.next_phase.is_none() => {
                    Some("final_boundary_requires_continue")
                }
                _ => None,
            }
        };
        if let Some(reason) = rejection {
            self.rejected_commands += 1;
            self.snapshot.notice = reason;
            self.state.send_replace(self.snapshot.clone());
            let mut event = self.event("command", Some(false), Some(command.action), reason);
            event["data"]["command"] = serde_json::to_value(command)?;
            return Ok(event);
        }
        let notice = match command.action {
            Action::Hold => {
                self.hold_requested = true;
                self.step_armed = false;
                self.snapshot.mode = Mode::HoldRequested;
                "hold_requested"
            }
            Action::Continue => {
                self.release(now);
                self.hold_requested = false;
                self.step_armed = false;
                self.snapshot.mode = Mode::Running;
                "continued"
            }
            Action::Step => {
                self.release(now);
                self.hold_requested = false;
                self.step_armed = true;
                self.snapshot.mode = Mode::Stepping;
                "stepping"
            }
        };
        self.accepted_commands += 1;
        self.publish(notice)?;
        let mut event = self.event("command", Some(true), Some(command.action), notice);
        event["data"]["command"] = serde_json::to_value(command)?;
        Ok(event)
    }

    /// Called only at a ready dispatch boundary. Repeated calls at an already
    /// released boundary do not retrigger a configured breakpoint.
    pub(crate) fn gate(
        &mut self,
        next_phase: Option<usize>,
        now: Instant,
    ) -> Result<(bool, Option<Value>)> {
        self.refresh(now);
        let boundary = next_phase.unwrap_or(self.snapshot.phase_count + 1);
        if self.boundary_held == Some(boundary) {
            return Ok((false, None));
        }
        if self.boundary_open == Some(boundary) {
            return Ok((true, None));
        }
        self.snapshot.next_phase = next_phase;
        if self.hold_requested || self.step_armed || self.settings.before_phases.contains(&boundary)
        {
            self.hold_requested = false;
            self.step_armed = false;
            self.boundary_held = Some(boundary);
            self.held_since = Some(now);
            self.holds += 1;
            self.snapshot.mode = Mode::Held;
            self.publish("boundary_held")?;
            Ok((
                false,
                Some(self.event("gate-held", None, None, "boundary_held")),
            ))
        } else {
            self.boundary_open = Some(boundary);
            self.snapshot.mode = Mode::Running;
            self.publish("boundary_open")?;
            Ok((
                true,
                Some(self.event("gate-open", None, None, "boundary_open")),
            ))
        }
    }

    pub(crate) fn phase_started(&mut self, phase: usize) -> Result<Value> {
        self.snapshot.next_phase = (phase < self.snapshot.phase_count).then_some(phase + 1);
        self.publish("phase_started")?;
        Ok(self.event("phase-start", None, None, "phase_started"))
    }

    pub(crate) fn finish(&mut self, now: Instant, failed: bool) -> Result<Value> {
        self.release(now);
        self.hold_requested = false;
        self.step_armed = false;
        self.accepting_commands = false;
        self.snapshot.mode = Mode::Finished;
        self.publish(if failed { "run_failed" } else { "run_finished" })?;
        Ok(self.event("finished", None, None, self.snapshot.notice))
    }

    pub(crate) fn begin_cleanup(&mut self, now: Instant) -> Result<Value> {
        self.release(now);
        self.hold_requested = false;
        self.step_armed = false;
        self.accepting_commands = false;
        self.snapshot.mode = Mode::Stopping;
        self.publish("cleaning_up")?;
        Ok(self.event("cleanup", None, None, "cleaning_up"))
    }

    pub(crate) fn report(&self, now: Instant, bot_program: Option<&Path>) -> Value {
        json!({"enabled":true,"held_ms":self.held_duration(now).as_millis(),
            "holds":self.holds,"accepted_commands":self.accepted_commands,"rejected_commands":self.rejected_commands,
            "settings":self.settings,"bot_program":bot_program})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(settings: Settings, phases: usize) -> Session {
        let (state, _) = watch::channel(Snapshot::default());
        Session::new(&settings, &state, 1, phases).unwrap()
    }

    fn command(session: &Session, action: Action) -> Command {
        Command {
            run: session.snapshot.run,
            revision: session.snapshot.revision,
            action,
        }
    }

    #[test]
    fn step_releases_one_whole_phase_and_holds_before_final_cleanup() {
        let mut session = session(Settings::default(), 2);
        let now = Instant::now();
        assert!(!session.gate(Some(1), now).unwrap().0);
        let step = command(&session, Action::Step);
        assert_eq!(
            session.command(step.clone(), now).unwrap()["data"]["accepted"],
            true
        );
        assert!(session.gate(Some(1), now).unwrap().0);
        session.phase_started(1).unwrap();
        assert!(session.gate(Some(1), now).unwrap().0);
        assert!(!session.gate(Some(2), now).unwrap().0);
        assert_eq!(
            session.command(step, now).unwrap()["data"]["reason"],
            "stale_revision"
        );
        assert!(!session.gate(Some(2), now).unwrap().0);
        session
            .command(command(&session, Action::Step), now)
            .unwrap();
        session.phase_started(2).unwrap();
        assert!(!session.gate(None, now).unwrap().0);
        assert_eq!(
            session
                .command(command(&session, Action::Step), now)
                .unwrap()["data"]["reason"],
            "final_boundary_requires_continue"
        );
        session
            .command(command(&session, Action::Continue), now)
            .unwrap();
        assert!(session.gate(None, now).unwrap().0);
        assert_eq!(session.holds, 3);
    }

    #[test]
    fn hold_is_deferred_and_continue_consumes_a_breakpoint_once() {
        let mut session = session(
            Settings {
                start_held: false,
                before_phases: vec![2],
                ..Settings::default()
            },
            3,
        );
        let now = Instant::now();
        assert!(session.gate(Some(1), now).unwrap().0);
        session.phase_started(1).unwrap();
        session
            .command(command(&session, Action::Hold), now)
            .unwrap();
        assert_eq!(session.snapshot.mode, Mode::HoldRequested);
        assert!(session.gate(Some(1), now).unwrap().0);
        assert!(!session.gate(Some(2), now).unwrap().0);
        session
            .command(command(&session, Action::Continue), now)
            .unwrap();
        assert!(session.gate(Some(2), now).unwrap().0);
        assert!(session.gate(Some(2), now).unwrap().0);
        session.phase_started(2).unwrap();
        assert!(session.gate(Some(3), now).unwrap().0);
    }

    #[test]
    fn hold_clock_is_separate_and_resets_only_between_boundaries() {
        let mut session = session(
            Settings {
                max_hold_ms: 100,
                ..Settings::default()
            },
            2,
        );
        let now = Instant::now();
        session.gate(Some(1), now).unwrap();
        let revision = session.snapshot.revision;
        session.refresh(now + Duration::from_millis(70));
        assert_eq!(session.snapshot.held_ms, 70);
        assert_eq!(session.snapshot.revision, revision);
        assert!(!session.hold_expired(now + Duration::from_millis(99)));
        assert!(session.hold_expired(now + Duration::from_millis(100)));
        session
            .command(
                command(&session, Action::Step),
                now + Duration::from_millis(70),
            )
            .unwrap();
        session.phase_started(1).unwrap();
        session
            .gate(Some(2), now + Duration::from_millis(200))
            .unwrap();
        assert_eq!(session.snapshot.held_ms, 0);
        assert_eq!(
            session.held_duration(now + Duration::from_millis(210)),
            Duration::from_millis(80)
        );
        assert!(!session.hold_expired(now + Duration::from_millis(210)));
        session
            .begin_cleanup(now + Duration::from_millis(210))
            .unwrap();
        assert_eq!(session.snapshot.mode, Mode::Stopping);
        assert_eq!(session.snapshot.notice, "cleaning_up");
        assert_eq!(
            session
                .command(
                    command(&session, Action::Hold),
                    now + Duration::from_millis(220)
                )
                .unwrap()["data"]["reason"],
            "cleaning_up"
        );
        assert_eq!(session.snapshot.mode, Mode::Stopping);
        session
            .finish(now + Duration::from_millis(230), true)
            .unwrap();
        assert_eq!(session.snapshot.mode, Mode::Finished);
        assert_eq!(
            session.held_duration(now + Duration::from_secs(1)),
            Duration::from_millis(80)
        );
    }

    #[test]
    fn expired_controls_cannot_release_a_hold_and_rejections_are_observable() {
        let (state, watched) = watch::channel(Snapshot::default());
        let mut session = Session::new(
            &Settings {
                max_hold_ms: 100,
                ..Settings::default()
            },
            &state,
            1,
            1,
        )
        .unwrap();
        let now = Instant::now();
        session.gate(Some(1), now).unwrap();
        let request = command(&session, Action::Continue);
        let revision = request.revision;
        let event = session
            .command(request, now + Duration::from_millis(100))
            .unwrap();
        assert_eq!(event["data"]["reason"], "hold_expired");
        assert_eq!(event["data"]["command"]["revision"], revision);
        assert_eq!(watched.borrow().notice, "hold_expired");
        assert_eq!(watched.borrow().revision, revision);
        assert_eq!(watched.borrow().mode, Mode::Held);
        assert!(
            !session
                .gate(Some(1), now + Duration::from_millis(101))
                .unwrap()
                .0
        );
    }

    #[test]
    fn stale_commands_do_not_cross_matrix_runs_and_invalid_settings_fail() {
        let (state, _) = watch::channel(Snapshot::default());
        let mut first = Session::new(&Settings::default(), &state, 1, 1).unwrap();
        first.gate(Some(1), Instant::now()).unwrap();
        let old = command(&first, Action::Step);
        let mut second = Session::new(&Settings::default(), &state, 2, 1).unwrap();
        second.gate(Some(1), Instant::now()).unwrap();
        assert_eq!(
            second.command(old, Instant::now()).unwrap()["data"]["reason"],
            "stale_run"
        );
        assert_eq!(second.snapshot.mode, Mode::Held);
        for phases in [vec![0], vec![1, 1], vec![1001]] {
            assert!(
                Settings {
                    before_phases: phases,
                    ..Settings::default()
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            Settings {
                before_phases: vec![2],
                ..Settings::default()
            }
            .validate_phases(1)
            .is_err()
        );
        assert!(
            serde_json::from_value::<Settings>(json!({"version":1,"freeze_world":true})).is_err()
        );
    }
}
