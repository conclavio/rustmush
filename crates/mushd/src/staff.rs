//! Server-side staff commands (M2): `@siteban`, `@dump`, and `@shutdown`.
//!
//! These need the store, which `mush-core` must never see. The bridge is
//! the treaty-ruling-4 shape: the commands are registered into the normal
//! [`Registry`] (so dispatch, `min_role: Wizard` gating, switch validation,
//! and `help` all work natively), and their handlers — which run
//! synchronously inside the world task's `Engine::step` — capture a shared
//! [`StaffQueue`] and push typed [`StaffOp`]s onto it. The world task
//! drains the queue right after the step and performs the async store work
//! on detached tasks, delivering the confirmation to the actor when the
//! store has acknowledged (ack-gated output, like account creation).

use std::net::IpAddr;
use std::sync::{Arc, Mutex};

use mush_core::{ArgShape, CmdError, CommandSpec, Ctx, ObjId, Registry, RegistryError, Role};
use mush_proto::{OutputEvent, RichText};

/// One queued staff operation, tagged with the requesting actor (who gets
/// the confirmation once the store acks).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StaffOp {
    /// The wizard who ran the command.
    pub actor: ObjId,
    /// What to do.
    pub kind: StaffOpKind,
}

/// The staff operations the world task executes against the store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StaffOpKind {
    /// `@siteban <ip>[=<duration> <reason>]` — persist (and mirror) a ban.
    SetBan {
        /// The address to ban.
        ip: IpAddr,
        /// Expiry (unix seconds); `None` = permanent.
        until_unix: Option<i64>,
        /// Staff-entered reason.
        reason: String,
    },
    /// `@siteban/clear <ip>` — lift a persisted ban.
    ClearBan {
        /// The address to unban.
        ip: IpAddr,
    },
    /// Bare `@siteban` — list the active persisted bans.
    ListBans,
    /// `@dump` — BGSAVE, ordered behind every pending write batch.
    Dump,
    /// `@shutdown` — graceful stop: the world task steps
    /// `ControlMsg::Shutdown` (announce to all) and exits; main then runs
    /// the normal drain → BGSAVE → close sequence, exactly like Ctrl-C.
    Shutdown,
}

/// The sync→async hand-off queue between command handlers (inside
/// `Engine::step`) and the world task (which owns the store handles).
/// Handlers push; the world task drains immediately after the step, so ops
/// never sit across loop iterations.
#[derive(Debug, Default)]
pub struct StaffQueue {
    ops: Mutex<Vec<StaffOp>>,
}

impl StaffQueue {
    /// An empty queue.
    #[must_use]
    pub fn new() -> StaffQueue {
        StaffQueue::default()
    }

    /// Queue one op (called from a sync command handler).
    pub fn push(&self, op: StaffOp) {
        self.ops.lock().expect("staff queue poisoned").push(op);
    }

    /// Drain everything queued, in push order.
    #[must_use]
    pub fn drain(&self) -> Vec<StaffOp> {
        std::mem::take(&mut *self.ops.lock().expect("staff queue poisoned"))
    }
}

/// Parse a ban duration token: `<n>` seconds, or `<n>s|m|h|d`. Returns the
/// length in seconds (`> 0`).
#[must_use]
pub fn parse_duration_s(tok: &str) -> Option<i64> {
    let tok = tok.trim();
    let (digits, mult) = match tok.chars().last()? {
        's' => (&tok[..tok.len() - 1], 1),
        'm' => (&tok[..tok.len() - 1], 60),
        'h' => (&tok[..tok.len() - 1], 3600),
        'd' => (&tok[..tok.len() - 1], 86_400),
        c if c.is_ascii_digit() => (tok, 1),
        _ => return None,
    };
    let n: i64 = digits.parse().ok()?;
    (n > 0).then(|| n.saturating_mul(mult))
}

const SITEBAN_USAGE: &str = "Usage: @siteban <ip>[=<duration> <reason>] (duration: 30m, 12h, 7d, or seconds; \
     no '=' part means permanent), @siteban/clear <ip>, or bare @siteban to list.";

fn parse_ip(text: &str) -> Result<IpAddr, CmdError> {
    text.trim()
        .parse::<IpAddr>()
        .map_err(|_| CmdError::msg(format!("{text:?} is not a valid IP address.")))
}

fn siteban_cmd(queue: &StaffQueue, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("clear") {
        let raw = ctx.args.raw();
        if raw.is_empty() {
            return Err(CmdError::msg("Usage: @siteban/clear <ip>."));
        }
        let ip = parse_ip(raw)?;
        queue.push(StaffOp {
            actor: ctx.actor,
            kind: StaffOpKind::ClearBan { ip },
        });
        return Ok(());
    }
    if ctx.args.raw().is_empty() {
        queue.push(StaffOp {
            actor: ctx.actor,
            kind: StaffOpKind::ListBans,
        });
        return Ok(());
    }
    let ip = parse_ip(ctx.args.lhs())?;
    let (until_unix, reason) = match ctx.args.rhs() {
        None => (None, String::new()),
        Some(rhs) => {
            let rhs = rhs.trim();
            let (dur_tok, reason) = match rhs.split_once(char::is_whitespace) {
                Some((d, r)) => (d, r.trim().to_string()),
                None => (rhs, String::new()),
            };
            let secs = parse_duration_s(dur_tok).ok_or_else(|| CmdError::msg(SITEBAN_USAGE))?;
            (Some(ctx.w.now().saturating_add(secs)), reason)
        }
    };
    queue.push(StaffOp {
        actor: ctx.actor,
        kind: StaffOpKind::SetBan {
            ip,
            until_unix,
            reason,
        },
    });
    Ok(())
}

fn shutdown_cmd(queue: &StaffQueue, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    // No announcement here: the world task steps `ControlMsg::Shutdown`,
    // whose engine handling broadcasts "GAME: Shutdown in progress." to
    // every connection before the loop exits.
    queue.push(StaffOp {
        actor: ctx.actor,
        kind: StaffOpKind::Shutdown,
    });
    Ok(())
}

fn dump_cmd(queue: &StaffQueue, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    // The classic announcement goes out immediately; the actor's private
    // confirmation is ack-gated behind the snapshot itself.
    ctx.out.to_all(OutputEvent::Sys(RichText::tag(
        "system",
        "GAME: Dumping database.",
    )));
    queue.push(StaffOp {
        actor: ctx.actor,
        kind: StaffOpKind::Dump,
    });
    Ok(())
}

/// Register the server-owned staff commands into the world's registry.
/// Call after `register_core_commands` / `register_engine_commands`; pass
/// the same `queue` to the world task so it can drain what the handlers
/// push.
pub fn register_server_commands(
    reg: &mut Registry,
    queue: &Arc<StaffQueue>,
) -> Result<(), RegistryError> {
    let q = Arc::clone(queue);
    reg.register(
        CommandSpec {
            name: "@siteban".into(),
            switches: vec!["clear".into()],
            shape: ArgShape::EqPair,
            min_role: Role::Wizard,
            help: "@siteban <ip>[=<duration> <reason>]  --  Ban an IP address from \
                   connecting (persisted; survives restarts). Duration is like 30m, \
                   12h, 7d, or plain seconds; without '=' the ban is permanent. \
                   @siteban/clear <ip> lifts a ban; bare @siteban lists active bans. \
                   Wizards only."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(move |ctx: &mut Ctx<'_>| siteban_cmd(&q, ctx)),
    )?;
    let q = Arc::clone(queue);
    reg.register(
        CommandSpec {
            name: "@dump".into(),
            shape: ArgShape::None,
            min_role: Role::Wizard,
            help: "@dump  --  Trigger a background database snapshot (Redis BGSAVE), \
                   ordered behind every write already queued. You are told when the \
                   store has acknowledged it. Wizards only."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(move |ctx: &mut Ctx<'_>| dump_cmd(&q, ctx)),
    )?;
    let q = Arc::clone(queue);
    reg.register(
        CommandSpec {
            name: "@shutdown".into(),
            shape: ArgShape::None,
            min_role: Role::Wizard,
            help: "@shutdown  --  Gracefully stop the server: announce the shutdown, \
                   stop the world, drain every pending store write, trigger a BGSAVE \
                   snapshot, and exit. Same sequence as Ctrl-C/SIGTERM at the console. \
                   Wizards only."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(move |ctx: &mut Ctx<'_>| shutdown_cmd(&q, ctx)),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::{
        Effects, Engine, GOD, Inbound, LIMBO, ObjKind, Outbox, World, register_core_commands,
    };

    fn engine_with_staff() -> (Engine, Arc<StaffQueue>) {
        let mut world = World::new_minimal();
        world.set_now(1_000);
        let mut reg = Registry::new();
        register_core_commands(&mut reg).unwrap();
        let queue = Arc::new(StaffQueue::new());
        register_server_commands(&mut reg, &queue).unwrap();
        (Engine::new(world, reg), queue)
    }

    fn run(engine: &mut Engine, actor: mush_core::ObjId, line: &str) -> Outbox {
        let mut out = Outbox::new();
        let mut fx = Effects::new();
        engine.step(
            Inbound::Cmd {
                conn: 1,
                actor,
                line: line.into(),
            },
            &mut out,
            &mut fx,
        );
        out
    }

    #[test]
    fn duration_parsing() {
        assert_eq!(parse_duration_s("90"), Some(90));
        assert_eq!(parse_duration_s("30s"), Some(30));
        assert_eq!(parse_duration_s("30m"), Some(1_800));
        assert_eq!(parse_duration_s("12h"), Some(43_200));
        assert_eq!(parse_duration_s("7d"), Some(604_800));
        for bad in ["", "0", "-5", "x", "5w", "d", "1.5h"] {
            assert_eq!(parse_duration_s(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn siteban_parses_and_queues() {
        let (mut engine, queue) = engine_with_staff();
        run(&mut engine, GOD, "@siteban 10.1.2.3=12h being a menace");
        run(&mut engine, GOD, "@siteban 10.1.2.4");
        run(&mut engine, GOD, "@siteban/clear 10.1.2.3");
        run(&mut engine, GOD, "@siteban");
        let ops = queue.drain();
        assert_eq!(ops.len(), 4);
        assert_eq!(
            ops[0].kind,
            StaffOpKind::SetBan {
                ip: "10.1.2.3".parse().unwrap(),
                until_unix: Some(1_000 + 43_200),
                reason: "being a menace".to_string(),
            }
        );
        assert_eq!(
            ops[1].kind,
            StaffOpKind::SetBan {
                ip: "10.1.2.4".parse().unwrap(),
                until_unix: None,
                reason: String::new(),
            }
        );
        assert_eq!(
            ops[2].kind,
            StaffOpKind::ClearBan {
                ip: "10.1.2.3".parse().unwrap()
            }
        );
        assert_eq!(ops[3].kind, StaffOpKind::ListBans);
        assert!(queue.drain().is_empty(), "drain empties the queue");
    }

    #[test]
    fn siteban_rejects_garbage_and_non_wizards() {
        let (mut engine, queue) = engine_with_staff();
        // Bad IP and bad duration queue nothing and error.
        let out = run(&mut engine, GOD, "@siteban not-an-ip");
        assert!(!out.is_empty());
        let out = run(&mut engine, GOD, "@siteban 10.0.0.1=eternity because");
        assert!(!out.is_empty());
        assert!(queue.drain().is_empty());

        // A mortal is refused by dispatch's min_role gate.
        let nick = engine
            .world
            .create(ObjKind::Player, "Nick", GOD, Some(LIMBO))
            .unwrap();
        let out = run(&mut engine, nick, "@siteban 10.0.0.1");
        let denied = out.events().iter().any(|(_, e)| match e {
            OutputEvent::Sys(t) => t.to_plain_text() == "Permission denied.",
            _ => false,
        });
        assert!(denied, "{:?}", out.events());
        assert!(queue.drain().is_empty());
    }

    #[test]
    fn shutdown_queues_for_wizards_only() {
        let (mut engine, queue) = engine_with_staff();
        run(&mut engine, GOD, "@shutdown");
        assert_eq!(
            queue.drain(),
            vec![StaffOp {
                actor: GOD,
                kind: StaffOpKind::Shutdown
            }]
        );

        // A mortal is refused by dispatch's min_role gate.
        let nick = engine
            .world
            .create(ObjKind::Player, "Nick", GOD, Some(LIMBO))
            .unwrap();
        let out = run(&mut engine, nick, "@shutdown");
        let denied = out.events().iter().any(|(_, e)| match e {
            OutputEvent::Sys(t) => t.to_plain_text() == "Permission denied.",
            _ => false,
        });
        assert!(denied, "{:?}", out.events());
        assert!(queue.drain().is_empty());
    }

    #[test]
    fn dump_announces_and_queues() {
        let (mut engine, queue) = engine_with_staff();
        let out = run(&mut engine, GOD, "@dump");
        let announced = out.events().iter().any(|(aud, e)| {
            matches!(aud, mush_core::Audience::All)
                && matches!(e, OutputEvent::Sys(t) if t.to_plain_text().contains("Dumping"))
        });
        assert!(announced);
        assert_eq!(
            queue.drain(),
            vec![StaffOp {
                actor: GOD,
                kind: StaffOpKind::Dump
            }]
        );
    }
}
