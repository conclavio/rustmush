//! Server-side staff commands: `@siteban`, `@dump`, `@shutdown` (M2),
//! `@boot`, `@ps`, `@stats` (M3), and `@newpassword` (M4).
//!
//! These need server-side state (the store, the session layer, the
//! scheduler), which `mush-core` must never see. The bridge is the
//! treaty-ruling-4 shape: the commands are registered into the normal
//! [`Registry`] (so dispatch, `min_role`/power gating, switch validation,
//! and `help` all work natively), and their handlers — which run
//! synchronously inside the world task's `Engine::step` — capture a shared
//! [`StaffQueue`] and push typed [`StaffOp`]s onto it. The world task
//! drains the queue right after the step: store-touching ops
//! (siteban/dump) run on async executor tasks with ack-gated confirmations,
//! while **local** ops ([`StaffOpKind::is_local`]: boot/ps/stats) are
//! handled inline in the loop where the `Engine`, the scheduler, and the
//! session manager are all at hand.

use std::fmt;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};

use mush_core::{
    ArgShape, CmdError, CommandSpec, Ctx, ObjId, ObjKind, Powers, Registry, RegistryError, Role,
    perm,
};
use mush_proto::{OutputEvent, RichText};
use ssh_key::{HashAlg, PublicKey};

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
    /// `@boot <player>[=<message>]` — disconnect every one of the target's
    /// connections. The handler already queued the target an
    /// [`OutputEvent::Disconnect`] carrying the message (delivered before
    /// the world task detaches the session); this op does the detach and
    /// feeds the resulting `ConnDown(Boot)`s back into the engine.
    Boot {
        /// The actor being booted.
        target: ObjId,
        /// Staff-entered message (empty when none was given) — audit log.
        message: String,
    },
    /// `@newpassword <player>=<newpass>` — re-hash (argon2 on the blocking
    /// pool), update the owning account's stored hash, and revoke every one
    /// of the account's web session tokens (logout-everywhere). The
    /// confirmation gates on the store acks.
    NewPassword {
        /// The target character (the account is resolved store-side).
        target: ObjId,
        /// The target's display name at command time (messages/audit).
        name: String,
        /// The requesting wizard's display name (target notification).
        by: String,
        /// The new password — redacted from `Debug` so no log can leak it.
        password: Secret,
    },
    /// `@sshkey/add <openssh public line>` — register an SSH public key on
    /// the CALLER's account (self-service). The line is validated and
    /// normalized at command time, so only the parsed public key and its
    /// fingerprint (both public data) ride the op: a mistakenly-pasted
    /// PRIVATE key can never reach the queue, a log, or the store.
    AddSshKey {
        /// The requesting character; the owning account is resolved
        /// store-side by scanning for the account that lists it.
        actor: ObjId,
        /// The normalized `"type base64 comment"` public-key line.
        openssh: String,
        /// The `SHA256:...` fingerprint (public identity, safe to log).
        fingerprint: String,
    },
    /// `@sshkey/list` — list the caller's account's registered keys.
    ListSshKeys {
        /// The requesting character (account resolved store-side).
        actor: ObjId,
    },
    /// `@sshkey/del <index-or-fingerprint>` — remove one key from the
    /// caller's account by 1-based list index or by `SHA256:...`
    /// fingerprint.
    DelSshKey {
        /// The requesting character (account resolved store-side).
        actor: ObjId,
        /// The user's selector: a 1-based index or a fingerprint string.
        /// Public (never a secret), so it may appear in `Debug`/logs.
        which: String,
    },
    /// `@newchar <Name>` — create one MORE character on the CALLER's own
    /// account (self-service; M20). The whole flow runs store-side/off-loop
    /// so the world loop never blocks on a store read: the owning account is
    /// resolved by scanning for the one that lists `actor` (so it can only
    /// ever touch the caller's own account — invariant N1), the cap and the
    /// world name index are checked BEFORE anything is created (N2), then the
    /// Player is created through the established durable, ack-gated server
    /// create path and linked into the account doc (N3). The confirmation
    /// gates on the store link write.
    NewChar {
        /// The requesting character; the owning account is resolved
        /// store-side by scanning for the account that lists it.
        actor: ObjId,
        /// The validated (command-time) name for the new character. The
        /// world create re-checks it atomically for a duplicate.
        name: String,
        /// The `[game].max_characters` per-account cap, enforced off-loop
        /// before the new Player is created.
        max_characters: usize,
    },
    /// `@ps` — render the scheduler (pending `@wait`s, module ticks,
    /// counts) to the requesting wizard.
    Ps,
    /// `@stats` (wizard form) — render uptime, object counts, connected
    /// count, loop lag, and store health to the requesting wizard. The
    /// mortal form is answered directly by the command handler.
    Stats,
}

impl StaffOpKind {
    /// Whether the op is handled inline in the world loop (needs the
    /// `Engine`/sessions, no store round-trip) rather than by the async
    /// store executors.
    #[must_use]
    pub fn is_local(&self) -> bool {
        matches!(
            self,
            StaffOpKind::Boot { .. } | StaffOpKind::Ps | StaffOpKind::Stats
        )
    }
}

/// A staff-supplied secret riding the op queue (`@newpassword`'s new
/// password). `Debug` is hand-written to redact, so the derived `Debug` on
/// [`StaffOpKind`] (and any future `tracing` of whole ops) can never leak
/// it into a log.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(pub String);

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
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

/// Resolve a `@boot` target: player-index name (optionally `*`-prefixed) or
/// a `#dbref` naming an actor.
fn resolve_actor(w: &mush_core::World, raw: &str) -> Result<ObjId, CmdError> {
    let q = raw.trim();
    if let Ok(id) = q.parse::<ObjId>() {
        return if w.get(id).is_some_and(|o| o.kind.is_actor()) {
            Ok(id)
        } else {
            Err(CmdError::msg("I don't recognize that player."))
        };
    }
    let name = q.strip_prefix('*').unwrap_or(q);
    w.resolve_player(name)
        .ok_or_else(|| CmdError::msg("I don't recognize that player."))
}

fn boot_cmd(queue: &StaffQueue, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let who = ctx.args.lhs().trim().to_string();
    if who.is_empty() {
        return Err(CmdError::msg("Usage: @boot <player>[=<message>]."));
    }
    let target = resolve_actor(ctx.w, &who)?;
    if target == ctx.actor {
        return Err(CmdError::msg("Use QUIT to disconnect yourself."));
    }
    // The BOOT power (or Wizard, which implies it) got us past dispatch;
    // rank still matters: nobody boots someone above their own role.
    if perm::role_of(ctx.w, target) > perm::role_of(ctx.w, ctx.actor) {
        return Err(CmdError::msg("Permission denied."));
    }
    if !ctx.w.is_connected(target) {
        return Err(CmdError::msg("That player is not connected."));
    }
    let message = ctx
        .args
        .rhs()
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .unwrap_or_default()
        .to_string();
    let booter = ctx
        .w
        .get(ctx.actor)
        .map_or_else(|| ctx.actor.to_string(), |o| o.name.clone());
    let reason = if message.is_empty() {
        format!("You have been booted by {booter}.")
    } else {
        format!("You have been booted by {booter}: {message}")
    };
    // The Disconnect event rides the normal outbox and is DELIVERED before
    // the world task processes the queued op (delivery happens right after
    // the step, local staff ops after that) — so the message reaches the
    // target's connections before their session is closed.
    ctx.out.to_actor(
        target,
        OutputEvent::Disconnect {
            reason: RichText::tag("system", reason),
        },
    );
    queue.push(StaffOp {
        actor: ctx.actor,
        kind: StaffOpKind::Boot { target, message },
    });
    Ok(())
}

fn newpassword_cmd(queue: &StaffQueue, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @newpassword <player>=<new password>.";
    let who = ctx.args.lhs().trim().to_string();
    let Some(password) = ctx.args.rhs().map(str::trim).filter(|p| !p.is_empty()) else {
        return Err(CmdError::msg(USAGE));
    };
    if who.is_empty() {
        return Err(CmdError::msg(USAGE));
    }
    let target = resolve_actor(ctx.w, &who)?;
    if ctx.w.get(target).is_none_or(|o| o.kind != ObjKind::Player) {
        return Err(CmdError::msg("Only players have passwords."));
    }
    // Rank still matters: nobody resets a password above their own role.
    if perm::role_of(ctx.w, target) > perm::role_of(ctx.w, ctx.actor) {
        return Err(CmdError::msg("Permission denied."));
    }
    mush_config::accounts::validate_password(password).map_err(|e| CmdError::msg(format!("{e}.")))?;
    let name = ctx
        .w
        .get(target)
        .map_or_else(|| target.to_string(), |o| o.name.clone());
    let by = ctx
        .w
        .get(ctx.actor)
        .map_or_else(|| ctx.actor.to_string(), |o| o.name.clone());
    // No output here: the confirmation gates on the store acks (hash
    // update + token revocation), delivered by the sequential executor.
    queue.push(StaffOp {
        actor: ctx.actor,
        kind: StaffOpKind::NewPassword {
            target,
            name,
            by,
            password: Secret(password.to_string()),
        },
    });
    Ok(())
}

/// Does `s` look like a PRIVATE key someone pasted by mistake? Detects PEM
/// / OpenSSH private-key markers (`-----BEGIN ... PRIVATE KEY-----`, covering
/// `OPENSSH`/`RSA`/`EC`/generic) and the PuTTY `.ppk` header. Content-free:
/// only the presence of a marker matters, never the bytes around it.
fn looks_like_private_key(s: &str) -> bool {
    const MARKERS: &[&str] = &["PRIVATE KEY-----", "PuTTY-User-Key-File"];
    MARKERS.iter().any(|m| s.contains(m))
}

/// `@sshkey/add|list|del` — self-service management of the CALLER's account
/// SSH public keys. `add` validates + normalizes the pasted line HERE (before
/// anything is queued, logged, or stored) so a private-key paste is caught and
/// the raw input never leaves this function; only the parsed public key and its
/// fingerprint ride the op. The account is resolved store-side from `ctx.actor`
/// (invariant S5: only ever the caller's own account).
fn sshkey_cmd(queue: &StaffQueue, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str =
        "Usage: @sshkey/add <ssh public key line>, @sshkey/list, or @sshkey/del <index-or-fingerprint>.";
    if ctx.switches.has("add") {
        let line = ctx.args.raw().trim();
        if line.is_empty() {
            return Err(CmdError::msg(
                "Usage: @sshkey/add <ssh public key line> -- paste the one line from your \
                 .pub file (it starts with ssh-ed25519 / ssh-rsa / ecdsa-...).",
            ));
        }
        // Private-key guard FIRST, before any parse: never echo the input.
        if looks_like_private_key(line) {
            // Log a generic reason only — never the pasted bytes (S7).
            tracing::warn!(actor = %ctx.actor, "@sshkey/add rejected: private-key paste");
            return Err(CmdError::msg(
                "That looks like a PRIVATE key. Keep it secret -- never paste or share it. \
                 I need your PUBLIC key: the .pub file (it starts with ssh-ed25519 / ssh-rsa / \
                 ecdsa-...), e.g. the output of `cat ~/.ssh/id_ed25519.pub`.",
            ));
        }
        // Parse + normalize here; a failure is reported without echoing input.
        let key = PublicKey::from_openssh(line).map_err(|_| {
            tracing::warn!(actor = %ctx.actor, "@sshkey/add rejected: not a public key");
            CmdError::msg(
                "That is not a valid OpenSSH public key. Paste the single line from your \
                 .pub file (it starts with ssh-ed25519 / ssh-rsa / ecdsa-...).",
            )
        })?;
        let openssh = key.to_openssh().map_err(|_| {
            tracing::warn!(actor = %ctx.actor, "@sshkey/add rejected: not a public key");
            CmdError::msg("That is not a valid OpenSSH public key.")
        })?;
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
        // Only the parsed public key + fingerprint (public data) are queued.
        queue.push(StaffOp {
            actor: ctx.actor,
            kind: StaffOpKind::AddSshKey {
                actor: ctx.actor,
                openssh,
                fingerprint,
            },
        });
        return Ok(());
    }
    if ctx.switches.has("del") {
        let which = ctx.args.raw().trim().to_string();
        if which.is_empty() {
            return Err(CmdError::msg(
                "Usage: @sshkey/del <index-or-fingerprint> -- see @sshkey/list for both.",
            ));
        }
        queue.push(StaffOp {
            actor: ctx.actor,
            kind: StaffOpKind::DelSshKey {
                actor: ctx.actor,
                which,
            },
        });
        return Ok(());
    }
    // `/list`, or a bare `@sshkey`, lists; anything else is a usage error.
    if ctx.switches.has("list") || ctx.args.raw().trim().is_empty() {
        queue.push(StaffOp {
            actor: ctx.actor,
            kind: StaffOpKind::ListSshKeys { actor: ctx.actor },
        });
        return Ok(());
    }
    Err(CmdError::msg(USAGE))
}

/// `@newchar <Name>` — create one more character on the CALLER's OWN account
/// (self-service; M20). The syntactic name check runs here for a fast, friendly
/// refusal, with an advisory in-loop duplicate check against the world's name
/// index (the world is at hand); the authoritative account resolution, cap
/// enforcement, durable create, and account link all run off-loop in the staff
/// executor (a store read must never happen on the world loop), so the command
/// handler only validates and queues. No output on success — the confirmation
/// is ack-gated on the store link write (N3), like `@newpassword`/`@sshkey`.
fn newchar_cmd(
    queue: &StaffQueue,
    max_characters: usize,
    ctx: &mut Ctx<'_>,
) -> Result<(), CmdError> {
    const USAGE: &str =
        "Usage: @newchar <Name>  --  create one more character on your own account.";
    let name = ctx.args.raw().trim().to_string();
    if name.is_empty() {
        return Err(CmdError::msg(USAGE));
    }
    // Validate the name up front for a fast, friendly refusal (N2). The
    // executor's world create re-checks the name atomically before creating,
    // so a duplicate can never produce a second Player.
    mush_config::accounts::validate_character_name(&name)
        .map_err(|e| CmdError::msg(format!("{e}.")))?;
    // Advisory in-loop duplicate check: the world holds the global actor-name
    // index, so a taken name gets a sharper message without a store round-trip.
    // (Authoritative rejection is still the executor's atomic create.)
    if ctx.w.resolve_player(&name).is_some() {
        return Err(CmdError::msg(format!(
            "{name:?} is already taken. Pick another name."
        )));
    }
    queue.push(StaffOp {
        actor: ctx.actor,
        kind: StaffOpKind::NewChar {
            actor: ctx.actor,
            name,
            max_characters,
        },
    });
    Ok(())
}

fn ps_cmd(queue: &StaffQueue, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    queue.push(StaffOp {
        actor: ctx.actor,
        kind: StaffOpKind::Ps,
    });
    Ok(())
}

fn stats_cmd(queue: &StaffQueue, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if perm::has_role(ctx.w, ctx.actor, Role::Wizard) {
        queue.push(StaffOp {
            actor: ctx.actor,
            kind: StaffOpKind::Stats,
        });
    } else {
        // The classic brief mortal form.
        let n = ctx.w.iter().count();
        ctx.out
            .info(ctx.actor, format!("The universe contains {n} objects."));
    }
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

/// The per-account character cap used when a caller registers the staff
/// commands without threading `[game].max_characters` explicitly (tests and
/// compat wiring). Mirrors `mush_config::GameCfg::default().max_characters`;
/// the real binaries call [`register_server_commands_with_cap`] with the
/// configured value instead.
const DEFAULT_MAX_CHARACTERS: usize = 3;

/// Register the server-owned staff commands into the world's registry with the
/// default per-account character cap ([`DEFAULT_MAX_CHARACTERS`]). Convenience
/// wrapper over [`register_server_commands_with_cap`] for tests and compat
/// wiring; the real binaries pass the configured `[game].max_characters`.
pub fn register_server_commands(
    reg: &mut Registry,
    queue: &Arc<StaffQueue>,
) -> Result<(), RegistryError> {
    register_server_commands_with_cap(reg, queue, DEFAULT_MAX_CHARACTERS)
}

/// Register the server-owned staff commands into the world's registry.
/// Call after `register_core_commands` / `register_engine_commands`; pass
/// the same `queue` to the world task so it can drain what the handlers
/// push. `max_characters` is the `[game].max_characters` per-account cap the
/// `@newchar` command enforces.
pub fn register_server_commands_with_cap(
    reg: &mut Registry,
    queue: &Arc<StaffQueue>,
    max_characters: usize,
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
    let q = Arc::clone(queue);
    reg.register(
        CommandSpec {
            name: "@boot".into(),
            shape: ArgShape::EqPair,
            // min_role stays Player so a mortal GRANTED the BOOT power can
            // use it; Wizard+ passes the power check implicitly.
            power: Some(Powers::BOOT),
            help: "@boot <player>[=<message>]  --  Disconnect every one of a \
                   player's connections at once. They see \"You have been \
                   booted by <you>\" (plus your message, if given) before the \
                   line drops; the room sees the normal disconnect. You cannot \
                   boot someone of higher rank than yourself. Requires the \
                   BOOT power (Wizards have it implicitly)."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(move |ctx: &mut Ctx<'_>| boot_cmd(&q, ctx)),
    )?;
    let q = Arc::clone(queue);
    reg.register(
        CommandSpec {
            name: "@newpassword".into(),
            shape: ArgShape::EqPair,
            min_role: Role::Wizard,
            help: "@newpassword <player>=<new password>  --  Set a player's account \
                   password (they forgot it, or it leaked). The change is written \
                   through to the database, and every one of the account's web \
                   sessions is logged out (their tokens are revoked); active SSH \
                   connections stay up. You cannot change the password of someone \
                   of higher rank than yourself. Wizards only."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(move |ctx: &mut Ctx<'_>| newpassword_cmd(&q, ctx)),
    )?;
    let q = Arc::clone(queue);
    reg.register(
        CommandSpec {
            name: "@sshkey".into(),
            switches: vec!["add".into(), "list".into(), "del".into()],
            // Raw so a base64 key line with '=' padding is not split as EqPair.
            shape: ArgShape::Raw,
            // Self-service: any player manages the keys on their OWN account.
            min_role: Role::Player,
            help: "@sshkey/add <ssh public key line>  --  Authorize an SSH public key on \
                   your account, so `ssh -i <key> <account>@game` logs you in with no \
                   password. Paste the ONE line from your .pub file (starts with \
                   ssh-ed25519 / ssh-rsa / ecdsa-...); never paste your private key. \
                   @sshkey/list shows your registered keys (index, fingerprint, comment, \
                   date); @sshkey/del <index-or-fingerprint> removes one. Acts only on \
                   your own account. See also `account+character@game` to log a key \
                   straight into one character."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(move |ctx: &mut Ctx<'_>| sshkey_cmd(&q, ctx)),
    )?;
    let q = Arc::clone(queue);
    let cap = max_characters;
    reg.register(
        CommandSpec {
            name: "@newchar".into(),
            // A single character name; Raw so it is taken verbatim (trimmed).
            shape: ArgShape::Raw,
            // Self-service: any player may create another character on their
            // OWN account (invariant N1 is enforced by the store-side account
            // scan in the executor).
            min_role: Role::Player,
            help: "@newchar <Name>  --  Create one MORE character on your own account, up \
                   to the per-account limit. The new character starts in Limbo, unapproved \
                   (chargen runs the first time you play it); you keep playing your current \
                   character. To play the new one, reconnect and pick it at the \
                   character-select screen, or `ssh <account>+<Name>@game` to jump straight \
                   in. Acts only on your own account."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(move |ctx: &mut Ctx<'_>| newchar_cmd(&q, cap, ctx)),
    )?;
    let q = Arc::clone(queue);
    reg.register(
        CommandSpec {
            name: "@ps".into(),
            shape: ArgShape::None,
            min_role: Role::Wizard,
            help: "@ps  --  Inspect the scheduler: pending @wait entries (who \
                   queued them, seconds remaining, a preview of the command), \
                   registered module ticks with their cadences and next fire, \
                   and totals. Wizards only."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(move |ctx: &mut Ctx<'_>| ps_cmd(&q, ctx)),
    )?;
    let q = Arc::clone(queue);
    reg.register(
        CommandSpec {
            name: "@stats".into(),
            shape: ArgShape::None,
            help: "@stats  --  Server statistics. Everyone gets the classic \
                   \"The universe contains N objects.\"; Wizards get the full \
                   report: uptime, object counts by kind, connected players, \
                   world-loop lag (last/max), store health (read-only mode and \
                   the age of the last write), and database size."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(move |ctx: &mut Ctx<'_>| stats_cmd(&q, ctx)),
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
    fn boot_validates_and_queues_with_disconnect_event() {
        let (mut engine, queue) = engine_with_staff();
        let nick = engine
            .world
            .create(ObjKind::Player, "Nick", GOD, Some(LIMBO))
            .unwrap();

        // Not connected yet: refused, nothing queued.
        let out = run(&mut engine, GOD, "@boot Nick");
        assert!(
            out.events().iter().any(|(_, e)| matches!(e,
                OutputEvent::Sys(t) if t.to_plain_text().contains("not connected"))),
            "{:?}",
            out.events()
        );
        assert!(queue.drain().is_empty());

        // Connect Nick, then boot with a message.
        let mut fx = mush_core::Effects::new();
        let mut o = Outbox::new();
        engine.step(
            mush_core::Inbound::ConnUp {
                conn: 7,
                actor: nick,
                caps_structured: false,
                from_linkdead: false,
            },
            &mut o,
            &mut fx,
        );
        let out = run(&mut engine, GOD, "@boot Nick=go away");
        // The engine-side Disconnect event carries the message to the actor.
        let disc = out.events().iter().any(|(aud, e)| {
            matches!(aud, mush_core::Audience::Actor(a) if *a == nick)
                && matches!(e, OutputEvent::Disconnect { reason }
                    if reason.to_plain_text() == "You have been booted by God: go away")
        });
        assert!(disc, "{:?}", out.events());
        assert_eq!(
            queue.drain(),
            vec![StaffOp {
                actor: GOD,
                kind: StaffOpKind::Boot {
                    target: nick,
                    message: "go away".into()
                }
            }]
        );

        // Booting yourself is refused.
        let out = run(&mut engine, GOD, "@boot God");
        assert!(out.events().iter().any(|(_, e)| matches!(e,
                OutputEvent::Sys(t) if t.to_plain_text().contains("Use QUIT"))),);
        assert!(queue.drain().is_empty());

        // A mortal without the BOOT power is refused by dispatch.
        let out = run(&mut engine, nick, "@boot God");
        assert!(out.events().iter().any(|(_, e)| matches!(e,
            OutputEvent::Sys(t) if t.to_plain_text() == "Permission denied.")));
        assert!(queue.drain().is_empty());

        // Granting BOOT lets the mortal use the command — but never up-rank.
        engine.world.get_mut(nick).unwrap().powers = mush_core::Powers::BOOT;
        let out = run(&mut engine, nick, "@boot God=usurped");
        assert!(out.events().iter().any(|(_, e)| matches!(e,
            OutputEvent::Sys(t) if t.to_plain_text() == "Permission denied.")));
        assert!(queue.drain().is_empty());
    }

    #[test]
    fn ps_is_wizard_gated_and_queues() {
        let (mut engine, queue) = engine_with_staff();
        run(&mut engine, GOD, "@ps");
        assert_eq!(
            queue.drain(),
            vec![StaffOp {
                actor: GOD,
                kind: StaffOpKind::Ps
            }]
        );
        let nick = engine
            .world
            .create(ObjKind::Player, "Nick", GOD, Some(LIMBO))
            .unwrap();
        let out = run(&mut engine, nick, "@ps");
        assert!(out.events().iter().any(|(_, e)| matches!(e,
            OutputEvent::Sys(t) if t.to_plain_text() == "Permission denied.")));
        assert!(queue.drain().is_empty());
    }

    #[test]
    fn stats_brief_for_mortals_full_op_for_wizards() {
        let (mut engine, queue) = engine_with_staff();
        let nick = engine
            .world
            .create(ObjKind::Player, "Nick", GOD, Some(LIMBO))
            .unwrap();
        // Mortal: the classic one-liner, answered synchronously, no op.
        let out = run(&mut engine, nick, "@stats");
        // Limbo + God + Nick = 3 objects.
        assert!(
            out.events().iter().any(|(_, e)| matches!(e,
                OutputEvent::Sys(t) if t.to_plain_text() == "The universe contains 3 objects.")),
            "{:?}",
            out.events()
        );
        assert!(queue.drain().is_empty());
        // Wizard: the full report is rendered by the world task.
        run(&mut engine, GOD, "@stats");
        assert_eq!(
            queue.drain(),
            vec![StaffOp {
                actor: GOD,
                kind: StaffOpKind::Stats
            }]
        );
    }

    // A fixed ed25519 public line (shared with the gateway/loopback tests).
    const PUB_LINE: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICKZsuYnz5vDjbi8gtQSL5djhsj1/8olA+yHkPMLp/Ki loopback-test@rustmush";

    fn sys_texts(out: &Outbox) -> Vec<String> {
        out.events()
            .iter()
            .filter_map(|(_, e)| match e {
                OutputEvent::Sys(t) => Some(t.to_plain_text()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn sshkey_add_validates_and_queues_only_the_public_key() {
        let (mut engine, queue) = engine_with_staff();
        // A mortal (self-service): min_role Player lets anyone manage their own.
        let nick = engine
            .world
            .create(ObjKind::Player, "Nick", GOD, Some(LIMBO))
            .unwrap();

        let out = run(&mut engine, nick, &format!("@sshkey/add {PUB_LINE}"));
        assert!(sys_texts(&out).is_empty(), "no echo on success: {out:?}", out = sys_texts(&out));
        let ops = queue.drain();
        assert_eq!(ops.len(), 1);
        let want_fp = PublicKey::from_openssh(PUB_LINE)
            .unwrap()
            .fingerprint(HashAlg::Sha256)
            .to_string();
        match &ops[0].kind {
            StaffOpKind::AddSshKey {
                actor,
                openssh,
                fingerprint,
            } => {
                assert_eq!(*actor, nick);
                assert_eq!(fingerprint, &want_fp);
                // The queued line is the NORMALIZED public key, not raw input.
                assert!(openssh.starts_with("ssh-ed25519 "));
                assert!(!openssh.contains("PRIVATE"));
            }
            other => panic!("expected AddSshKey, got {other:?}"),
        }
    }

    #[test]
    fn sshkey_add_rejects_private_key_paste_without_echoing_it() {
        let (mut engine, queue) = engine_with_staff();
        // A realistic private-key blob (its bytes must never surface).
        let privkey = "-----BEGIN OPENSSH PRIVATE KEY-----\n\
                       b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAAB-secret-material\n\
                       -----END OPENSSH PRIVATE KEY-----";
        let out = run(&mut engine, GOD, &format!("@sshkey/add {privkey}"));
        let texts = sys_texts(&out);
        assert!(
            texts.iter().any(|t| t.contains("PRIVATE key") && t.contains("PUBLIC key")),
            "specific private-key guidance expected: {texts:?}"
        );
        // The raw secret bytes must appear in NO reply.
        assert!(
            texts.iter().all(|t| !t.contains("secret-material")),
            "the pasted private key must never be echoed: {texts:?}"
        );
        assert!(queue.drain().is_empty(), "a private-key paste queues nothing");
    }

    #[test]
    fn sshkey_add_rejects_garbage() {
        let (mut engine, queue) = engine_with_staff();
        let out = run(&mut engine, GOD, "@sshkey/add this is not a key");
        assert!(
            sys_texts(&out)
                .iter()
                .any(|t| t.contains("not a valid OpenSSH public key")),
            "{:?}",
            sys_texts(&out)
        );
        assert!(queue.drain().is_empty());
    }

    #[test]
    fn sshkey_list_and_del_queue() {
        let (mut engine, queue) = engine_with_staff();
        run(&mut engine, GOD, "@sshkey/list");
        run(&mut engine, GOD, "@sshkey"); // bare == list
        run(&mut engine, GOD, "@sshkey/del 1");
        run(&mut engine, GOD, "@sshkey/del SHA256:abc");
        let ops = queue.drain();
        assert_eq!(ops.len(), 4);
        assert_eq!(ops[0].kind, StaffOpKind::ListSshKeys { actor: GOD });
        assert_eq!(ops[1].kind, StaffOpKind::ListSshKeys { actor: GOD });
        assert_eq!(
            ops[2].kind,
            StaffOpKind::DelSshKey {
                actor: GOD,
                which: "1".to_string()
            }
        );
        assert_eq!(
            ops[3].kind,
            StaffOpKind::DelSshKey {
                actor: GOD,
                which: "SHA256:abc".to_string()
            }
        );
    }

    #[test]
    fn sshkey_del_requires_an_argument() {
        let (mut engine, queue) = engine_with_staff();
        let out = run(&mut engine, GOD, "@sshkey/del");
        assert!(sys_texts(&out).iter().any(|t| t.contains("Usage")));
        assert!(queue.drain().is_empty());
    }

    #[test]
    fn newchar_validates_and_queues_for_the_caller() {
        let (mut engine, queue) = engine_with_staff();
        // Self-service: a mortal may create another character on their account.
        let nick = engine
            .world
            .create(ObjKind::Player, "Nick", GOD, Some(LIMBO))
            .unwrap();

        // A valid, free name queues a NewChar carrying the caller + the cap.
        let out = run(&mut engine, nick, "@newchar Casey");
        assert!(sys_texts(&out).is_empty(), "no echo on success: {:?}", sys_texts(&out));
        let ops = queue.drain();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].actor, nick);
        assert_eq!(
            ops[0].kind,
            StaffOpKind::NewChar {
                actor: nick,
                name: "Casey".to_string(),
                // engine_with_staff registers with the default cap.
                max_characters: DEFAULT_MAX_CHARACTERS,
            }
        );

        // An invalid name is refused up front — nothing is queued (N2).
        let out = run(&mut engine, nick, "@newchar 9bad");
        assert!(
            sys_texts(&out)
                .iter()
                .any(|t| t.contains("character names start with a letter")),
            "{:?}",
            sys_texts(&out)
        );
        assert!(queue.drain().is_empty());

        // A name already held by an actor is refused up front (advisory dup
        // check), so no op is queued (N2). `Nick` exists.
        let out = run(&mut engine, nick, "@newchar Nick");
        assert!(
            sys_texts(&out).iter().any(|t| t.contains("already taken")),
            "{:?}",
            sys_texts(&out)
        );
        assert!(queue.drain().is_empty());

        // Bare @newchar is a usage error, nothing queued.
        let out = run(&mut engine, nick, "@newchar");
        assert!(sys_texts(&out).iter().any(|t| t.contains("Usage")));
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
