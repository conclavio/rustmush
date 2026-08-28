//! The [`Engine`]: the synchronous heart the tokio world task drives.
//!
//! One loop iteration is: receive an [`Inbound`], call [`Engine::step`],
//! deliver the [`Outbox`], spawn everything in [`Effects`], sleep until
//! [`Scheduler::next_due`], and on wake call [`Engine::fire_due`]. The
//! engine itself is plain sync data — no I/O, no tokio — so every path
//! here is unit-testable with a manual clock.
//!
//! Async work follows treaty ruling 3, the **two-phase pattern**:
//!
//! 1. A handler snapshots what it needs, pushes an [`Effect`] (a
//!    future-free *description* of the async work) via
//!    [`Outbox::defer`], and returns immediately.
//! 2. The driving loop drains [`Effects`], spawns the real future, and
//!    when it completes sends [`Inbound::Apply`] with a typed
//!    [`WorldCommand`] carrying the correlation id. The engine's handler
//!    re-validates against **current** world state before applying.
//!
//! `@slowtest` is the M1 prototype of this seam: a fake 3-second "LLM
//! call" proving typed re-entry without ever blocking the game loop.

use std::collections::HashMap;
use std::sync::Arc;

use mush_proto::{OutputEvent, RichText};

use crate::cmd::{ArgShape, CmdError, CommandSpec, Ctx, Registry, RegistryError, dispatch};
use crate::commands::narrative_emit;
use crate::flags::Role;
use crate::obj::ObjId;
use crate::output::{ConnId, Outbox, name_span};
use crate::sched::{ScheduledKind, Scheduler};
use crate::world::World;

/// Cap on `@slowtest` delays (seconds) — it is a debug tool, not a timer.
const SLOWTEST_MAX_SECONDS: u64 = 60;

/// Cap on `@wait` delays: one year, keeping `due_ms` far from overflow.
const WAIT_MAX_SECONDS: u64 = 365 * 24 * 60 * 60;

/// Cap on *pending* `@wait` entries per actor (classic MUSH queue limit) —
/// without it any player could grow the scheduler heap without bound.
const WAIT_QUEUE_MAX: u32 = 100;

/// Why a connection went down — drives the engine's announcement and
/// whether the actor fully disconnects at once.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DisconnectKind {
    /// Player-initiated clean close (`QUIT`): on the last connection the
    /// actor fully disconnects immediately ("has disconnected.").
    Quit,
    /// Server-initiated clean close (`@boot`, defensive cleanup): same
    /// immediate full disconnect as [`DisconnectKind::Quit`].
    Boot,
    /// The transport dropped unexpectedly: on the last connection the actor
    /// goes link-dead and stays connected until [`Inbound::SessionGone`].
    Lost,
}

impl DisconnectKind {
    /// Clean closes end the session at once; `Lost` leaves it link-dead.
    #[must_use]
    pub fn is_clean(self) -> bool {
        matches!(self, DisconnectKind::Quit | DisconnectKind::Boot)
    }
}

impl std::fmt::Display for DisconnectKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            DisconnectKind::Quit => "quit",
            DisconnectKind::Boot => "boot",
            DisconnectKind::Lost => "lost",
        })
    }
}

/// Operator/driver control messages (SIGTERM, `@shutdown`, `@dump`).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ControlMsg {
    /// Announce and stop the world loop.
    Shutdown,
    /// Request a database dump. M1 has no store; the engine only announces
    /// (the M2 persistence layer will act on it in the driving loop).
    Dump,
}

/// Typed async re-entry (treaty ruling 3): every cross-crate async
/// completion comes back to the world as one of these, carried by
/// [`Inbound::Apply`]. Handlers **re-validate against current world
/// state** — the world may have changed while the async work ran.
///
/// Extensible by design: later milestones add arms here — reserved names
/// include `NpcAct { .. }` (M8 agent actions), `WorldgenDraftReady { .. }`
/// (M7 `@gen` drafts), and `AuthResult { .. }` (M2 login flows). Do not
/// repurpose those names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorldCommand {
    /// Completion of an [`Effect::SlowTest`]: the fake LLM "answered".
    SlowTestDone {
        /// The actor who ran `@slowtest`.
        actor: ObjId,
        /// Correlation id the effect carried out.
        correlation: u64,
        /// The produced text, emitted to the actor.
        text: String,
    },
}

/// A future-free description of deferred async work. `mush-core` holds no
/// futures (no tokio here); the driving loop drains these from [`Effects`]
/// and spawns the real futures, each completing as an [`Inbound::Apply`].
///
/// Later milestones add variants (LLM calls, argon2 verification, store
/// queries); M1 has only the seam prototype.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// The fake 3-second "LLM call": sleep `seconds`, then re-enter with
    /// [`WorldCommand::SlowTestDone`] for `actor`/`correlation`.
    SlowTest {
        /// The actor to answer.
        actor: ObjId,
        /// Correlation id to echo back in the completion.
        correlation: u64,
        /// How long the fake call takes.
        seconds: u64,
    },
}

/// What a command handler deferred through [`Outbox::defer`], routed by
/// [`Engine::step`]: async work goes to [`Effects`], timers go to the
/// [`Scheduler`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Deferred {
    /// Spawn this async work (via the [`Effects`] queue).
    Effect(Effect),
    /// Schedule `kind` to fire `delay_ms` after the world's current time.
    ScheduleIn {
        /// Delay relative to [`World::now`], in milliseconds.
        delay_ms: i64,
        /// What fires.
        kind: ScheduledKind,
    },
}

/// The queue of [`Effect`]s one loop iteration produced. The driving loop
/// drains it after every [`Engine::step`] / [`Engine::fire_due`] and spawns
/// the described futures.
#[derive(Debug, Default)]
pub struct Effects {
    queue: Vec<Effect>,
}

impl Effects {
    /// An empty queue.
    #[must_use]
    pub fn new() -> Effects {
        Effects::default()
    }

    /// Queue one effect.
    pub fn push(&mut self, effect: Effect) {
        self.queue.push(effect);
    }

    /// Drain the queue in push order, leaving it empty.
    pub fn drain(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.queue)
    }

    /// The queued effects, in push order (for tests/inspection).
    #[must_use]
    pub fn queued(&self) -> &[Effect] {
        &self.queue
    }

    /// Number of queued effects.
    #[must_use]
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Whether nothing is queued.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

/// Everything the world task can receive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inbound {
    /// One input line from a live connection.
    Cmd {
        /// The connection it arrived on.
        conn: ConnId,
        /// The actor it runs as.
        actor: ObjId,
        /// The raw line.
        line: String,
    },
    /// A connection finished auth and attached to an actor.
    ConnUp {
        /// The new connection.
        conn: ConnId,
        /// The actor it attached to.
        actor: ObjId,
        /// Whether the client understands structured (JSON) events —
        /// recorded per connection; the session layer renders accordingly.
        caps_structured: bool,
        /// Whether this attach revived a link-dead session (the session
        /// layer's `AttachOutcome::Rejoined { from_linkdead }`); drives the
        /// classic "has reconnected." announce.
        from_linkdead: bool,
    },
    /// A connection dropped (network close, QUIT, boot). A clean
    /// [`DisconnectKind`] on the actor's last connection fully disconnects
    /// them at once; `Lost` leaves them link-dead until
    /// [`Inbound::SessionGone`].
    ConnDown {
        /// The connection that went away.
        conn: ConnId,
        /// The actor it was attached to.
        actor: ObjId,
        /// Why it went down.
        reason: DisconnectKind,
    },
    /// The session layer gave up on the actor entirely (linkdead grace
    /// expired). The actor fully disconnects — unless the engine currently
    /// tracks live connections for them, in which case the message is stale
    /// (a reconnect raced the reaper) and is ignored (treaty ruling 3:
    /// re-validate against current state).
    SessionGone {
        /// The actor whose session ended.
        actor: ObjId,
    },
    /// Typed completion of deferred async work (treaty ruling 3).
    Apply(WorldCommand),
    /// Operator/driver control.
    Control(ControlMsg),
}

/// What the driving loop should do after a step.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StepOutcome {
    /// Keep looping.
    Continue,
    /// Stop the world loop (shutdown was requested and announced).
    Shutdown,
}

/// Per-connection bookkeeping.
#[derive(Copy, Clone, Debug)]
struct ConnInfo {
    actor: ObjId,
    #[allow(dead_code)] // read by the session layer via caps negotiation later
    caps_structured: bool,
}

/// The sync world engine: world + command registry + scheduler + connection
/// bookkeeping. The tokio loop in `mushd` owns exactly one of these.
pub struct Engine {
    /// The game state.
    pub world: World,
    /// The command table.
    pub registry: Registry,
    /// Pending timers (`@wait`, module ticks).
    pub sched: Scheduler,
    /// Live connections by id.
    conns: HashMap<ConnId, ConnInfo>,
    /// Live connection count per actor (invariant: sums `conns` rows).
    conn_count: HashMap<ObjId, u32>,
    /// Pending `@wait` entries per actor (capped at [`WAIT_QUEUE_MAX`]).
    pending_waits: HashMap<ObjId, u32>,
}

impl Engine {
    /// Wrap a world and a fully registered command table. The caller
    /// registers commands first (typically
    /// [`register_core_commands`](crate::register_core_commands) followed by
    /// [`register_engine_commands`]).
    #[must_use]
    pub fn new(world: World, registry: Registry) -> Engine {
        Engine {
            world,
            registry,
            sched: Scheduler::new(),
            conns: HashMap::new(),
            conn_count: HashMap::new(),
            pending_waits: HashMap::new(),
        }
    }

    /// Feed wall time in (unix seconds) — forwarded to [`World::set_now`].
    /// The driving loop calls this before each step; core logic never reads
    /// the system clock.
    pub fn set_now(&mut self, unix_seconds: i64) {
        self.world.set_now(unix_seconds);
    }

    /// The actor a live connection is attached to.
    #[must_use]
    pub fn actor_of(&self, conn: ConnId) -> Option<ObjId> {
        self.conns.get(&conn).map(|c| c.actor)
    }

    /// Number of live connections attached to an actor.
    #[must_use]
    pub fn connections_of(&self, actor: ObjId) -> u32 {
        self.conn_count.get(&actor).copied().unwrap_or(0)
    }

    /// Handle one inbound message. Output lands in `out`; deferred async
    /// work lands in `fx` (and timers in [`Engine::sched`]). Returns
    /// whether the loop should keep running.
    pub fn step(&mut self, msg: Inbound, out: &mut Outbox, fx: &mut Effects) -> StepOutcome {
        let outcome = match msg {
            Inbound::Cmd {
                conn: _,
                actor,
                line,
            } => {
                if self.world.get(actor).is_some() {
                    self.world.session_input(actor);
                    dispatch(&self.registry, &mut self.world, actor, &line, out);
                } else {
                    tracing::warn!(%actor, "Cmd for missing actor dropped");
                }
                StepOutcome::Continue
            }
            Inbound::ConnUp {
                conn,
                actor,
                caps_structured,
                from_linkdead,
            } => {
                self.conn_up(conn, actor, caps_structured, from_linkdead, out);
                StepOutcome::Continue
            }
            Inbound::ConnDown {
                conn,
                actor,
                reason,
            } => {
                self.conn_down(conn, actor, reason, out);
                StepOutcome::Continue
            }
            Inbound::SessionGone { actor } => {
                self.session_gone(actor, out);
                StepOutcome::Continue
            }
            Inbound::Apply(cmd) => {
                self.apply(cmd, out);
                StepOutcome::Continue
            }
            Inbound::Control(ControlMsg::Shutdown) => {
                out.to_all(OutputEvent::Sys(RichText::tag(
                    "system",
                    "GAME: Shutdown in progress.",
                )));
                StepOutcome::Shutdown
            }
            Inbound::Control(ControlMsg::Dump) => {
                out.to_all(OutputEvent::Sys(RichText::tag(
                    "system",
                    "GAME: Dumping database.",
                )));
                StepOutcome::Continue
            }
        };
        self.drain_deferred(out, fx);
        outcome
    }

    /// Fire every scheduled entry due at `now_ms` (unix milliseconds):
    /// `WaitCmd` dispatches its line as its actor; module ticks and control
    /// timers have no consumers in M1 and are logged. The driving loop
    /// calls this when the [`Scheduler::next_due`] sleep elapses.
    pub fn fire_due(&mut self, now_ms: i64, out: &mut Outbox, fx: &mut Effects) {
        for kind in self.sched.pop_due(now_ms) {
            match kind {
                ScheduledKind::WaitCmd { actor, line } => {
                    self.dec_pending_waits(actor);
                    // Re-validate: the actor may be gone by the time the
                    // timer fires.
                    if self.world.get(actor).is_some() {
                        dispatch(&self.registry, &mut self.world, actor, &line, out);
                    } else {
                        tracing::debug!(%actor, "WaitCmd for missing actor dropped");
                    }
                }
                ScheduledKind::ModuleTick { module, tag } => {
                    tracing::debug!(%module, %tag, "module tick with no consumer (M1)");
                }
                ScheduledKind::Control(name) => {
                    tracing::debug!(%name, "control timer with no consumer (M1)");
                }
            }
        }
        self.drain_deferred(out, fx);
    }

    // ------------------------------------------------------------------
    // Inbound handlers
    // ------------------------------------------------------------------

    fn conn_up(
        &mut self,
        conn: ConnId,
        actor: ObjId,
        caps_structured: bool,
        from_linkdead: bool,
        out: &mut Outbox,
    ) {
        if self.world.get(actor).is_none() {
            tracing::warn!(%actor, conn, "ConnUp for missing actor ignored");
            return;
        }
        let prev = self.conns.insert(
            conn,
            ConnInfo {
                actor,
                caps_structured,
            },
        );
        if let Some(p) = prev {
            if p.actor == actor {
                // Duplicate ConnUp for a tracked connection: caps updated,
                // nothing else to do.
                return;
            }
            // A conn id re-used for a different actor should not happen;
            // recover by releasing the old attachment.
            tracing::warn!(conn, old = %p.actor, new = %actor, "conn id re-attached");
            self.dec_conn_count(p.actor);
        }
        let n = self.conn_count.entry(actor).or_insert(0);
        *n += 1;
        if *n == 1 {
            // set_connected/session_start are idempotent, so reviving a
            // link-dead session keeps the original connect time.
            self.world.set_connected(actor, true);
            self.world.session_start(actor);
            let suffix = if from_linkdead {
                " has reconnected."
            } else {
                " has connected."
            };
            announce_room(&self.world, actor, suffix, out);
        } else {
            announce_room(&self.world, actor, " has reconnected.", out);
        }
    }

    fn conn_down(&mut self, conn: ConnId, actor: ObjId, reason: DisconnectKind, out: &mut Outbox) {
        let Some(info) = self.conns.remove(&conn) else {
            tracing::debug!(conn, %actor, "ConnDown for untracked connection ignored");
            return;
        };
        debug_assert_eq!(info.actor, actor, "ConnDown actor mismatch");
        let actor = info.actor; // trust our own bookkeeping over the message
        let last = self.dec_conn_count(actor);
        tracing::info!(conn, %actor, %reason, last, "connection down");
        if last && self.world.is_connected(actor) {
            if reason.is_clean() {
                // Clean close of the last connection: full disconnect now.
                announce_room(&self.world, actor, " has disconnected.", out);
                self.world.set_connected(actor, false);
                self.world.session_end(actor);
            } else {
                // The character stays in the room (and connected, for WHO)
                // — linkdead until the session layer sends SessionGone.
                announce_room(&self.world, actor, " has gone link-dead.", out);
            }
        }
    }

    fn session_gone(&mut self, actor: ObjId, out: &mut Outbox) {
        // Re-validate (treaty ruling 3): the reaper's SessionGone is not
        // ordered against a concurrent reconnect's ConnUp. If the engine
        // currently tracks live connections for the actor, the reconnect
        // won the race and this message is stale — applying it would leave
        // a live session world-side "disconnected" (deaf to its room).
        if self.connections_of(actor) > 0 {
            tracing::info!(%actor, "stale SessionGone ignored (live connections present)");
            return;
        }
        // Defensive: forget any connections still attached to the actor.
        self.conns.retain(|_, i| i.actor != actor);
        self.conn_count.remove(&actor);
        if self.world.is_connected(actor) {
            announce_room(&self.world, actor, " has disconnected.", out);
            self.world.set_connected(actor, false);
        }
        self.world.session_end(actor);
    }

    fn apply(&mut self, cmd: WorldCommand, out: &mut Outbox) {
        match cmd {
            WorldCommand::SlowTestDone {
                actor,
                correlation,
                text,
            } => {
                // Re-validate (treaty ruling 3): the actor may have been
                // destroyed while the fake call ran.
                if self.world.get(actor).is_none() {
                    tracing::debug!(%actor, correlation, "SlowTestDone for missing actor dropped");
                    return;
                }
                tracing::debug!(%actor, correlation, "slowtest completed");
                out.info(actor, text);
            }
        }
    }

    // ------------------------------------------------------------------
    // Internals
    // ------------------------------------------------------------------

    /// Route everything handlers deferred: effects to `fx`, timers to the
    /// scheduler (due = [`World::now`] seconds + delay). `@wait` entries are
    /// capped per actor ([`WAIT_QUEUE_MAX`]) so no player can grow the heap
    /// without bound.
    fn drain_deferred(&mut self, out: &mut Outbox, fx: &mut Effects) {
        for d in out.take_deferred() {
            match d {
                Deferred::Effect(e) => fx.push(e),
                Deferred::ScheduleIn { delay_ms, kind } => {
                    if let ScheduledKind::WaitCmd { actor, .. } = &kind {
                        let actor = *actor;
                        let n = self.pending_waits.entry(actor).or_insert(0);
                        if *n >= WAIT_QUEUE_MAX {
                            out.error(actor, "Too many commands queued.");
                            continue;
                        }
                        *n += 1;
                    }
                    let due = self
                        .world
                        .now()
                        .saturating_mul(1000)
                        .saturating_add(delay_ms);
                    self.sched.insert(due, kind);
                }
            }
        }
    }

    /// Release one pending-`@wait` slot for an actor (its timer fired).
    fn dec_pending_waits(&mut self, actor: ObjId) {
        match self.pending_waits.get_mut(&actor) {
            Some(n) if *n > 1 => *n -= 1,
            Some(_) => {
                self.pending_waits.remove(&actor);
            }
            None => {}
        }
    }

    /// Decrement an actor's live-connection count; `true` if it reached 0.
    fn dec_conn_count(&mut self, actor: ObjId) -> bool {
        match self.conn_count.get_mut(&actor) {
            Some(n) if *n > 1 => {
                *n -= 1;
                false
            }
            Some(_) => {
                self.conn_count.remove(&actor);
                true
            }
            None => false,
        }
    }
}

/// Queue an unattributed narrative line "<Name><suffix>" to the actor's
/// room, excluding the actor (connect/disconnect/linkdead announcements).
fn announce_room(w: &World, actor: ObjId, suffix: &str, out: &mut Outbox) {
    if let Some(loc) = w.get(actor).and_then(|o| o.location) {
        let text = RichText::new().span(name_span(w, actor)).text(suffix);
        out.to_room(loc, vec![actor], narrative_emit(text));
    }
}

// ----------------------------------------------------------------------
// Engine commands: @slowtest and @wait
// ----------------------------------------------------------------------

fn slowtest_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let raw = ctx.args.raw();
    let seconds: u64 = if raw.is_empty() {
        3
    } else {
        raw.parse().map_err(|_| {
            CmdError::msg(format!(
                "Usage: @slowtest [<seconds>] (1-{SLOWTEST_MAX_SECONDS})."
            ))
        })?
    };
    if !(1..=SLOWTEST_MAX_SECONDS).contains(&seconds) {
        return Err(CmdError::msg(format!(
            "Usage: @slowtest [<seconds>] (1-{SLOWTEST_MAX_SECONDS})."
        )));
    }
    let correlation = ctx.w.next_correlation_id();
    ctx.out.defer(Deferred::Effect(Effect::SlowTest {
        actor: ctx.actor,
        correlation,
        seconds,
    }));
    ctx.out.info(ctx.actor, "Thinking...");
    Ok(())
}

fn wait_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @wait <seconds>=<command>.";
    let seconds: u64 = ctx.args.lhs().parse().map_err(|_| CmdError::msg(USAGE))?;
    if seconds > WAIT_MAX_SECONDS {
        return Err(CmdError::msg("That's too long to wait."));
    }
    let line = ctx.args.rhs().map(str::trim).unwrap_or_default();
    if line.is_empty() {
        return Err(CmdError::msg(USAGE));
    }
    // i64 is safe: seconds <= WAIT_MAX_SECONDS.
    let delay_ms = i64::try_from(seconds).expect("bounded above") * 1000;
    ctx.out.defer(Deferred::ScheduleIn {
        delay_ms,
        kind: ScheduledKind::WaitCmd {
            actor: ctx.actor,
            line: line.to_string(),
        },
    });
    // Classic MUX @wait queues silently.
    Ok(())
}

/// Register the engine's commands (`@slowtest`, `@wait`). Called at boot
/// after [`register_core_commands`](crate::register_core_commands).
pub fn register_engine_commands(reg: &mut Registry) -> Result<(), RegistryError> {
    reg.register(
        CommandSpec {
            name: "@slowtest".into(),
            shape: ArgShape::Raw,
            min_role: Role::Wizard,
            help: "@slowtest [<seconds>]  --  Wizard debug tool: start a fake slow \
                   async call (default 3 seconds, max 60) that answers you when it \
                   completes. You see \"Thinking...\" immediately; the game never \
                   blocks while it runs. This prototypes the LLM re-entry path."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(slowtest_cmd),
    )?;
    reg.register(
        CommandSpec {
            name: "@wait".into(),
            shape: ArgShape::EqPair,
            help: "@wait <seconds>=<command>  --  Queue <command> to run as you \
                   after <seconds> seconds (whole seconds, up to one year). The \
                   command is dispatched exactly as if you typed it then. Queues \
                   silently, classic-style."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(wait_cmd),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::register_core_commands;
    use crate::flags::Flags;
    use crate::obj::{GOD, LIMBO, ObjKind};
    use crate::output::Audience;
    use mush_proto::NarrativeKind;

    /// Engine with core + engine commands, God connected via conn 1, and a
    /// player Nick (not yet connected) in Limbo.
    fn setup() -> (Engine, ObjId) {
        let mut world = World::new_minimal();
        world.set_now(1_000);
        let nick = world
            .create(ObjKind::Player, "Nick", GOD, Some(LIMBO))
            .unwrap();
        let mut registry = Registry::new();
        register_core_commands(&mut registry).unwrap();
        register_engine_commands(&mut registry).unwrap();
        let mut engine = Engine::new(world, registry);
        let mut out = Outbox::new();
        let mut fx = Effects::new();
        engine.step(
            Inbound::ConnUp {
                conn: 1,
                actor: GOD,
                caps_structured: false,
                from_linkdead: false,
            },
            &mut out,
            &mut fx,
        );
        (engine, nick)
    }

    fn texts(out: &Outbox) -> Vec<String> {
        out.events()
            .iter()
            .map(|(_, e)| match e {
                OutputEvent::Sys(t) => t.to_plain_text(),
                OutputEvent::Narrative { text, .. } => text.to_plain_text(),
                other => format!("{other:?}"),
            })
            .collect()
    }

    fn room_saw(out: &Outbox, room: ObjId, needle: &str) -> bool {
        out.events().iter().any(|(aud, e)| {
            matches!(aud, Audience::Room { room: r, .. } if *r == room)
                && matches!(e, OutputEvent::Narrative { text, .. } if text.to_plain_text().contains(needle))
        })
    }

    #[test]
    fn conn_up_announces_connect_then_reconnect() {
        let (mut engine, nick) = setup();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnUp {
                conn: 2,
                actor: nick,
                caps_structured: true,
                from_linkdead: false,
            },
            &mut out,
            &mut fx,
        );
        assert!(room_saw(&out, LIMBO, "Nick has connected."));
        assert!(engine.world.is_connected(nick));
        assert_eq!(engine.world.session_stat(nick).unwrap().connect_time, 1_000);
        assert_eq!(engine.actor_of(2), Some(nick));
        assert_eq!(engine.connections_of(nick), 1);

        // Second connection: reconnect announce, connect_time untouched.
        engine.set_now(1_050);
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnUp {
                conn: 3,
                actor: nick,
                caps_structured: false,
                from_linkdead: false,
            },
            &mut out,
            &mut fx,
        );
        assert!(room_saw(&out, LIMBO, "Nick has reconnected."));
        assert_eq!(engine.connections_of(nick), 2);
        assert_eq!(engine.world.session_stat(nick).unwrap().connect_time, 1_000);

        // The announcement excludes the actor themselves.
        assert!(out.events().iter().all(
            |(aud, _)| matches!(aud, Audience::Room { exclude, .. } if exclude.contains(&nick))
        ));
    }

    #[test]
    fn conn_up_for_missing_actor_is_ignored() {
        let (mut engine, _) = setup();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnUp {
                conn: 9,
                actor: ObjId(99),
                caps_structured: false,
                from_linkdead: false,
            },
            &mut out,
            &mut fx,
        );
        assert!(out.is_empty());
        assert_eq!(engine.actor_of(9), None);
    }

    #[test]
    fn conn_down_announces_linkdead_only_on_last_connection() {
        let (mut engine, nick) = setup();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        for conn in [2, 3] {
            engine.step(
                Inbound::ConnUp {
                    conn,
                    actor: nick,
                    caps_structured: false,
                    from_linkdead: false,
                },
                &mut out,
                &mut fx,
            );
        }
        // Dropping one of two connections: silent.
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnDown {
                conn: 2,
                actor: nick,
                reason: DisconnectKind::Lost,
            },
            &mut out,
            &mut fx,
        );
        assert!(out.is_empty());
        assert_eq!(engine.connections_of(nick), 1);

        // Dropping the last: link-dead announce, still connected for WHO.
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnDown {
                conn: 3,
                actor: nick,
                reason: DisconnectKind::Lost,
            },
            &mut out,
            &mut fx,
        );
        assert!(room_saw(&out, LIMBO, "Nick has gone link-dead."));
        assert!(engine.world.is_connected(nick));
        assert_eq!(engine.connections_of(nick), 0);

        // A duplicate ConnDown is a no-op.
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnDown {
                conn: 3,
                actor: nick,
                reason: DisconnectKind::Lost,
            },
            &mut out,
            &mut fx,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn session_gone_disconnects_and_announces() {
        let (mut engine, nick) = setup();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnUp {
                conn: 2,
                actor: nick,
                caps_structured: false,
                from_linkdead: false,
            },
            &mut out,
            &mut fx,
        );
        // Link goes down (link-dead), then the grace expires: SessionGone.
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnDown {
                conn: 2,
                actor: nick,
                reason: DisconnectKind::Lost,
            },
            &mut out,
            &mut fx,
        );
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(Inbound::SessionGone { actor: nick }, &mut out, &mut fx);
        assert!(room_saw(&out, LIMBO, "Nick has disconnected."));
        assert!(!engine.world.is_connected(nick));
        assert_eq!(engine.world.session_stat(nick), None);
        assert_eq!(engine.actor_of(2), None, "lingering conns are forgotten");

        // A second SessionGone announces nothing.
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(Inbound::SessionGone { actor: nick }, &mut out, &mut fx);
        assert!(out.is_empty());
    }

    /// The reaper's `SessionGone` is not ordered against a concurrent
    /// reconnect's `ConnUp`: when the ConnUp wins the race, the stale
    /// SessionGone must be ignored, not applied (it would leave a live
    /// session world-side "disconnected" — deaf to its room, absent from
    /// WHO, with no recovery).
    #[test]
    fn stale_session_gone_after_reconnect_is_ignored() {
        let (mut engine, nick) = setup();
        // Connect, lose the link (link-dead), reconnect out of linkdead.
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnUp {
                conn: 2,
                actor: nick,
                caps_structured: false,
                from_linkdead: false,
            },
            &mut out,
            &mut fx,
        );
        engine.step(
            Inbound::ConnDown {
                conn: 2,
                actor: nick,
                reason: DisconnectKind::Lost,
            },
            &mut out,
            &mut fx,
        );
        engine.step(
            Inbound::ConnUp {
                conn: 3,
                actor: nick,
                caps_structured: false,
                from_linkdead: false,
            },
            &mut out,
            &mut fx,
        );
        // The reaper's SessionGone arrives late: ignored outright.
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(Inbound::SessionGone { actor: nick }, &mut out, &mut fx);
        assert!(out.is_empty(), "stale SessionGone must not announce");
        assert!(engine.world.is_connected(nick));
        assert_eq!(engine.actor_of(3), Some(nick));
        assert_eq!(engine.connections_of(nick), 1);
        assert!(engine.world.session_stat(nick).is_some());
    }

    /// A clean last-connection ConnDown (QUIT) fully disconnects at once:
    /// "has disconnected.", connected flag cleared, session stats ended —
    /// no link-dead phase and no SessionGone needed.
    #[test]
    fn clean_conn_down_disconnects_immediately() {
        let (mut engine, nick) = setup();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnUp {
                conn: 2,
                actor: nick,
                caps_structured: false,
                from_linkdead: false,
            },
            &mut out,
            &mut fx,
        );
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnDown {
                conn: 2,
                actor: nick,
                reason: DisconnectKind::Quit,
            },
            &mut out,
            &mut fx,
        );
        assert!(room_saw(&out, LIMBO, "Nick has disconnected."));
        assert!(!room_saw(&out, LIMBO, "link-dead"));
        assert!(!engine.world.is_connected(nick));
        assert_eq!(engine.world.session_stat(nick), None);
        assert_eq!(engine.connections_of(nick), 0);
    }

    /// Reviving a link-dead session announces "has reconnected." (the
    /// driver forwards the session layer's `from_linkdead`), and keeps the
    /// original connect time.
    #[test]
    fn linkdead_revival_announces_reconnected() {
        let (mut engine, nick) = setup();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnUp {
                conn: 2,
                actor: nick,
                caps_structured: false,
                from_linkdead: false,
            },
            &mut out,
            &mut fx,
        );
        engine.step(
            Inbound::ConnDown {
                conn: 2,
                actor: nick,
                reason: DisconnectKind::Lost,
            },
            &mut out,
            &mut fx,
        );
        engine.set_now(1_040);
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnUp {
                conn: 3,
                actor: nick,
                caps_structured: false,
                from_linkdead: true,
            },
            &mut out,
            &mut fx,
        );
        assert!(room_saw(&out, LIMBO, "Nick has reconnected."));
        assert!(!room_saw(&out, LIMBO, "Nick has connected."));
        // Connect time survives the link-dead gap (session_start idempotent).
        assert_eq!(engine.world.session_stat(nick).unwrap().connect_time, 1_000);
    }

    #[test]
    fn cmd_dispatches_and_stamps_last_input() {
        let (mut engine, nick) = setup();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnUp {
                conn: 2,
                actor: nick,
                caps_structured: false,
                from_linkdead: false,
            },
            &mut out,
            &mut fx,
        );
        engine.set_now(1_030);
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Cmd {
                conn: 2,
                actor: nick,
                line: "say hello".into(),
            },
            &mut out,
            &mut fx,
        );
        assert!(room_saw(&out, LIMBO, "hello"));
        assert_eq!(engine.world.session_stat(nick).unwrap().last_input, 1_030);

        // A Cmd for a missing actor neither panics nor emits.
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Cmd {
                conn: 7,
                actor: ObjId(99),
                line: "say ghost".into(),
            },
            &mut out,
            &mut fx,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn who_reports_real_idle_and_conn_seconds() {
        let (mut engine, nick) = setup();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnUp {
                conn: 2,
                actor: nick,
                caps_structured: false,
                from_linkdead: false,
            },
            &mut out,
            &mut fx,
        );
        engine.set_now(1_010);
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Cmd {
                conn: 2,
                actor: nick,
                line: "think idling now".into(),
            },
            &mut out,
            &mut fx,
        );
        // 40s later, God runs WHO: Nick is idle 40s, connected 50s.
        engine.set_now(1_050);
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: "WHO".into(),
            },
            &mut out,
            &mut fx,
        );
        let entries = out
            .events()
            .iter()
            .find_map(|(_, e)| match e {
                OutputEvent::Who(entries) => Some(entries.clone()),
                _ => None,
            })
            .expect("WHO output");
        let nick_row = entries.iter().find(|e| e.name == "Nick").expect("Nick row");
        assert_eq!(nick_row.idle_s, 40);
        assert_eq!(nick_row.conn_s, 50);
        // God ran WHO just now: idle 0, connected since setup at t=1000.
        let god_row = entries.iter().find(|e| e.name == "God").expect("God row");
        assert_eq!(god_row.idle_s, 0);
        assert_eq!(god_row.conn_s, 50);
    }

    #[test]
    fn slowtest_defers_effect_and_round_trips_apply() {
        let (mut engine, nick) = setup();
        // Wizard-only: Nick is denied and no effect is queued.
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Cmd {
                conn: 2,
                actor: nick,
                line: "@slowtest".into(),
            },
            &mut out,
            &mut fx,
        );
        assert!(texts(&out).iter().any(|t| t == "Permission denied."));
        assert!(fx.is_empty());

        // God: "Thinking..." plus one deferred SlowTest effect.
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: "@slowtest".into(),
            },
            &mut out,
            &mut fx,
        );
        assert!(texts(&out).iter().any(|t| t == "Thinking..."));
        let effects = fx.drain();
        let Effect::SlowTest {
            actor,
            correlation,
            seconds,
        } = effects.first().cloned().expect("one effect queued");
        assert_eq!((actor, seconds), (GOD, 3));

        // The loop would sleep, then re-enter with the typed completion.
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Apply(WorldCommand::SlowTestDone {
                actor,
                correlation,
                text: "Deep thought complete.".into(),
            }),
            &mut out,
            &mut fx,
        );
        assert!(
            out.events().iter().any(|(aud, e)| matches!(aud, Audience::Actor(a) if *a == GOD)
                && matches!(e, OutputEvent::Sys(t) if t.to_plain_text() == "Deep thought complete."))
        );

        // Explicit seconds and validation.
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: "@slowtest 10".into(),
            },
            &mut out,
            &mut fx,
        );
        assert!(matches!(
            fx.queued(),
            [Effect::SlowTest { seconds: 10, .. }]
        ));
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: "@slowtest bogus".into(),
            },
            &mut out,
            &mut fx,
        );
        assert!(fx.is_empty());
        assert!(texts(&out).iter().any(|t| t.contains("Usage: @slowtest")));
    }

    #[test]
    fn slowtest_done_for_missing_actor_is_dropped() {
        let (mut engine, _) = setup();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Apply(WorldCommand::SlowTestDone {
                actor: ObjId(99),
                correlation: 7,
                text: "ghost".into(),
            }),
            &mut out,
            &mut fx,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn wait_schedules_and_fires_through_engine() {
        let (mut engine, nick) = setup();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnUp {
                conn: 2,
                actor: nick,
                caps_structured: false,
                from_linkdead: false,
            },
            &mut out,
            &mut fx,
        );
        // now = 1000s. Queue a say for 3s out.
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Cmd {
                conn: 2,
                actor: nick,
                line: "@wait 3=say Delayed hello".into(),
            },
            &mut out,
            &mut fx,
        );
        assert!(out.events().is_empty(), "@wait queues silently");
        assert!(fx.is_empty(), "timers are not effects");
        assert_eq!(engine.sched.next_due(), Some(1_003_000));

        // Not due yet: nothing fires.
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.fire_due(1_002_999, &mut out, &mut fx);
        assert!(out.is_empty());
        assert_eq!(engine.sched.len(), 1);

        // Due: the line dispatches as Nick.
        engine.set_now(1_003);
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.fire_due(1_003_000, &mut out, &mut fx);
        assert!(out.events().iter().any(
            |(aud, e)| matches!(aud, Audience::Room { room, .. } if *room == LIMBO)
                && matches!(
                    e,
                    OutputEvent::Narrative { kind: NarrativeKind::Say, actor: Some(a), text }
                        if a.name == "Nick" && text.to_plain_text() == "Delayed hello"
                )
        ));
        assert!(engine.sched.is_empty());
    }

    #[test]
    fn wait_validates_arguments() {
        let (mut engine, _) = setup();
        for bad in [
            "@wait",
            "@wait 3",
            "@wait 3=",
            "@wait x=say hi",
            "@wait 99999999999=say hi",
        ] {
            let (mut out, mut fx) = (Outbox::new(), Effects::new());
            engine.step(
                Inbound::Cmd {
                    conn: 1,
                    actor: GOD,
                    line: bad.into(),
                },
                &mut out,
                &mut fx,
            );
            assert!(
                engine.sched.is_empty(),
                "{bad:?} must not schedule anything"
            );
            assert!(
                texts(&out).iter().any(|t| t.contains("wait")),
                "{bad:?} must error"
            );
        }
    }

    #[test]
    fn wait_fired_for_destroyed_actor_is_dropped() {
        let (mut engine, nick) = setup();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Cmd {
                conn: 2,
                actor: nick,
                line: "@wait 1=say too late".into(),
            },
            &mut out,
            &mut fx,
        );
        engine.world.destroy(nick).unwrap();
        engine.world.sweep();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.fire_due(1_001_000, &mut out, &mut fx);
        assert!(out.is_empty());
    }

    #[test]
    fn wait_queue_is_capped_per_actor() {
        let (mut engine, nick) = setup();
        for _ in 0..WAIT_QUEUE_MAX {
            let (mut out, mut fx) = (Outbox::new(), Effects::new());
            engine.step(
                Inbound::Cmd {
                    conn: 2,
                    actor: nick,
                    line: "@wait 60=say tick".into(),
                },
                &mut out,
                &mut fx,
            );
            assert!(out.events().is_empty(), "under the cap queues silently");
        }
        assert_eq!(engine.sched.len() as u32, WAIT_QUEUE_MAX);

        // One past the cap: refused with an error, nothing scheduled.
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Cmd {
                conn: 2,
                actor: nick,
                line: "@wait 60=say overflow".into(),
            },
            &mut out,
            &mut fx,
        );
        assert!(
            texts(&out)
                .iter()
                .any(|t| t.contains("Too many commands queued.")),
            "over-cap @wait must error"
        );
        assert_eq!(engine.sched.len() as u32, WAIT_QUEUE_MAX);

        // Another actor is unaffected by Nick's full queue.
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: "@wait 60=say fine".into(),
            },
            &mut out,
            &mut fx,
        );
        assert!(out.events().is_empty());
        assert_eq!(engine.sched.len() as u32, WAIT_QUEUE_MAX + 1);

        // Firing releases slots: after the queue drains, @wait works again.
        engine.set_now(1_100);
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.fire_due(1_100_000, &mut out, &mut fx);
        assert!(engine.sched.is_empty());
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Cmd {
                conn: 2,
                actor: nick,
                line: "@wait 60=say again".into(),
            },
            &mut out,
            &mut fx,
        );
        assert!(out.events().is_empty(), "slots must free after firing");
        assert_eq!(engine.sched.len(), 1);
    }

    #[test]
    fn control_shutdown_announces_and_stops() {
        let (mut engine, _) = setup();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        let outcome = engine.step(Inbound::Control(ControlMsg::Shutdown), &mut out, &mut fx);
        assert_eq!(outcome, StepOutcome::Shutdown);
        assert!(
            out.events()
                .iter()
                .any(|(aud, e)| matches!(aud, Audience::All)
                    && matches!(e, OutputEvent::Sys(t) if t.to_plain_text().contains("Shutdown")))
        );
    }

    #[test]
    fn control_dump_announces_and_continues() {
        let (mut engine, _) = setup();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        let outcome = engine.step(Inbound::Control(ControlMsg::Dump), &mut out, &mut fx);
        assert_eq!(outcome, StepOutcome::Continue);
        assert!(
            out.events()
                .iter()
                .any(|(aud, e)| matches!(aud, Audience::All)
                    && matches!(e, OutputEvent::Sys(t) if t.to_plain_text().contains("Dumping")))
        );
    }

    #[test]
    fn connect_announcement_audience_excludes_only_the_actor() {
        // Regression guard on announcement audience shape. (DARK-aware
        // suppression is a later session-layer concern; a DARK player's
        // connect still queues the room event today.)
        let (mut engine, nick) = setup();
        engine.world.set_flag(nick, Flags::DARK, true).unwrap();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnUp {
                conn: 2,
                actor: nick,
                caps_structured: false,
                from_linkdead: false,
            },
            &mut out,
            &mut fx,
        );
        let (aud, _) = &out.events()[0];
        assert_eq!(
            aud,
            &Audience::Room {
                room: LIMBO,
                exclude: vec![nick]
            }
        );
    }
}
