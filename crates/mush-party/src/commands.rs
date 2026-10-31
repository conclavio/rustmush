//! The `+party` command surface (E6a, 08f §3.4).
//!
//! ```text
//! +party                       the parties you are in and the invitations you hold
//! +party <id>                  one party, if you may see it
//! +party/create <name>         you become leader and sole member of a Forming party
//! +party/invite <who>          leader only
//! +party/uninvite <who>        leader only
//! +party/accept <id>           an invitee joins; the invitation is consumed
//! +party/decline <id>          an invitee declines
//! +party/leave                 a member leaves
//! +party/kick <who>            leader only; never the leader themselves
//! +party/role <who>=<label>    leader only; an empty value clears
//! +party/disband               leader only
//! ```
//!
//! ## Existence hiding (§3.4, 08a decision 13)
//!
//! A party the actor may not **view** — not a member, not an invitee, not staff
//! ([`may_view`]) — answers exactly [`NO_SUCH_PARTY`], the same text a party that
//! never existed gets. So does a malformed id, and so does "you lead no party at
//! all". Every refusal that names a record goes through [`resolve`] first, so a
//! refusal can never be used to enumerate who is running with whom.
//!
//! ## Which party? — the disambiguation rule
//!
//! `/invite`, `/uninvite`, `/kick`, `/role`, `/leave` and `/disband` name no party.
//! They resolve against the actor's own **live** parties: the ones they *lead* for
//! the five leader verbs, the ones they are *in* for `/leave`. `/accept` and
//! `/decline` resolve against the invitations the actor holds.
//!
//! - **exactly one candidate** → that one;
//! - **none** → [`NO_SUCH_PARTY`] (identical to "you may not see it");
//! - **more than one** → refused, with the candidate ids listed and the explicit form
//!   spelled out. Nothing is guessed.
//!
//! The explicit form puts the id **first**:
//!
//! - a verb with no target (`/leave`, `/disband`, `/accept`, `/decline`) reads its
//!   whole argument as the id: `+party/leave 3`;
//! - a verb with a target (`/invite`, `/uninvite`, `/kick`, `/role`) reads a leading
//!   id **only when two or more whitespace-separated words were given**:
//!   `+party/kick 3 Bob` names party 3, while `+party/kick #12` is a dbref target and
//!   not party 12. That one rule is what keeps a party id and a dbref from ever
//!   competing for the same word.
//!
//! An id may be written `3` or `#3`; a party renders bare (`3`), because printing it
//! `#3` beside an object's `#12` is the confusion this rule exists to remove.
//!
//! ## Shape
//!
//! One [`CommandSpec`] with [`ArgShape::EqPair`], because `/role <who>=<label>` needs
//! the split and `lhs()` is the whole (trimmed) argument for every other switch. Only
//! `/create` reads `raw()`, so a party may be called `The A=PartyId`.

use std::sync::Arc;

use mush_core::module::{ModError, Registrar};
use mush_core::{ArgShape, CmdError, CommandSpec, Ctx, ObjId, SwitchSet, World, match_object};
use smol_str::SmolStr;

use crate::api::{NO_SUCH_PARTY, may_view};
use crate::error::PartyError;
use crate::ids::{PartyId, parse_party_id};
use crate::module::{NS, PartyStore, is_character, persist_meta, put_party, render_actor};
use crate::record::{
    DisbandCause, Invitation, MAX_PARTIES, MAX_PARTIES_PER_LEADER, MAX_PARTY_INVITES,
    MAX_PARTY_NAME_BYTES, MAX_PARTY_ROLE_BYTES, MAX_PARTY_SIZE, Party, PartyStatus, check_label,
};

/// The most parties one `+party` render lists per section. Bounded like every other
/// list in the E-series; a player cannot exceed it anyway
/// ([`MAX_PARTIES_PER_LEADER`] parties led, plus their invitations).
pub const LIST_CAP: usize = 20;

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: "+party".into(),
            shape: ArgShape::EqPair,
            switches: [
                "create", "invite", "uninvite", "accept", "decline", "leave", "kick", "role",
                "disband",
            ]
            .iter()
            .copied()
            .map(SmolStr::new)
            .collect(),
            help: "+party                                  the parties you are in and every\n\
                   \x20                                      invitation you hold\n\
                   +party <id>                             one party, if you may see it\n\
                   +party/create <name>                    you lead it, and you are its only member\n\
                   +party/invite <who>                     leader only\n\
                   +party/uninvite <who>                   leader only\n\
                   +party/accept <id>                      join a party you were invited to\n\
                   +party/decline <id>                     turn an invitation down\n\
                   +party/leave                            leave; the LAST member leaving disbands\n\
                   \x20                                      the party, and a leaving leader hands\n\
                   \x20                                      the lead to the next member\n\
                   +party/kick <who>                       leader only, and never yourself —\n\
                   \x20                                      use +party/disband for that\n\
                   +party/role <who>=<label>               leader only; a LABEL on a member, never\n\
                   \x20                                      authority. An empty value clears it.\n\
                   +party/disband                          leader only\n\
                   A party is a roster and nothing else: it holds no money, names no account and\n\
                   grants no permission. A party whose crew has been taken on for a job is LOCKED,\n\
                   and a locked roster cannot change at all — a substitution needs the job's terms\n\
                   amended first. A party nothing happens to for seven days disbands on its own.\n\
                   Which party: these verbs use the one you lead (or, for /leave, the one you are\n\
                   in). With more than one they REFUSE and list the ids; name one by putting it\n\
                   first — +party/kick 3 Bob, +party/leave 3, +party/role 3 Bob=face.\n\
                   A party you may not see answers \"No such party.\", exactly as one that does\n\
                   not exist does."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_party),
    )
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

/// Which verb the switches named.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Verb {
    List,
    Create,
    Invite,
    Uninvite,
    Accept,
    Decline,
    Leave,
    Kick,
    Role,
    Disband,
}

/// The first recognised switch, or [`Verb::List`] for the bare verb. Dispatch has
/// already refused anything that is not in the spec's switch list.
fn verb_of(sw: &SwitchSet) -> Verb {
    for name in sw.iter() {
        let v = match name {
            "create" => Verb::Create,
            "invite" => Verb::Invite,
            "uninvite" => Verb::Uninvite,
            "accept" => Verb::Accept,
            "decline" => Verb::Decline,
            "leave" => Verb::Leave,
            "kick" => Verb::Kick,
            "role" => Verb::Role,
            "disband" => Verb::Disband,
            _ => continue,
        };
        return v;
    }
    Verb::List
}

fn cmd_party(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let verb = verb_of(&ctx.switches);
    let lhs = ctx.args.lhs().trim().to_string();
    let rhs = ctx.args.rhs().map(str::to_string);
    let raw = ctx.args.raw().trim().to_string();
    match verb {
        Verb::List => list(ctx, &lhs),
        Verb::Create => create(ctx, &raw),
        Verb::Invite => invite(ctx, &lhs),
        Verb::Uninvite => uninvite(ctx, &lhs),
        Verb::Accept => accept(ctx, &lhs),
        Verb::Decline => decline(ctx, &lhs),
        Verb::Leave => leave(ctx, &lhs),
        Verb::Kick => kick(ctx, &lhs),
        Verb::Role => role(ctx, &lhs, rhs.as_deref()),
        Verb::Disband => disband(ctx, &lhs),
    }
}

// ---------------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------------

/// Split `[<id> ]<target>`: a leading party id is recognised **only** when two or
/// more whitespace-separated words were given, so `+party/kick #12` is a dbref and
/// `+party/kick 3 #12` is party 3.
fn split_target(lhs: &str) -> (Option<PartyId>, String) {
    let trimmed = lhs.trim();
    match trimmed.split_once(char::is_whitespace) {
        Some((head, rest)) if !rest.trim().is_empty() => match parse_party_id(head) {
            Some(id) => (Some(id), rest.trim().to_string()),
            None => (None, trimmed.to_string()),
        },
        _ => (None, trimmed.to_string()),
    }
}

/// The whole argument read as a party id, or `None` when it was empty.
///
/// # Errors
///
/// [`NO_SUCH_PARTY`] for anything that is not an id — a malformed id and an
/// unviewable party read the same, which is the point.
fn only_id(lhs: &str) -> Result<Option<PartyId>, CmdError> {
    let trimmed = lhs.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    parse_party_id(trimmed)
        .map(Some)
        .ok_or_else(|| CmdError::msg(NO_SUCH_PARTY))
}

/// Which parties a verb may act on when no id was given.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Pool {
    /// The live parties the actor leads.
    Led,
    /// The live parties the actor is a member of.
    Member,
    /// The live parties the actor holds an invitation to.
    Invited,
}

impl Pool {
    fn candidates(self, w: &World, who: ObjId) -> Vec<PartyId> {
        let Some(store) = w.module_state::<PartyStore>() else {
            return Vec::new();
        };
        match self {
            Pool::Led => store.live_parties_led_by(who),
            Pool::Member => store.live_parties_of(who),
            Pool::Invited => store.live_parties_inviting(who),
        }
    }

    fn prefix(self) -> &'static str {
        match self {
            Pool::Led => "You lead more than one party",
            Pool::Member => "You are in more than one party",
            Pool::Invited => "You hold more than one invitation",
        }
    }
}

/// The party a verb acts on: the explicit id when one was given, otherwise the
/// actor's single candidate.
///
/// # Errors
///
/// [`NO_SUCH_PARTY`] when the actor may not view the named party, or has no candidate
/// at all; a listing refusal naming `usage` when they have several.
fn resolve(
    ctx: &Ctx<'_>,
    explicit: Option<PartyId>,
    pool: Pool,
    usage: &str,
) -> Result<Party, CmdError> {
    let who = ctx.actor;
    let id = match explicit {
        Some(id) => id,
        None => {
            let ids = pool.candidates(ctx.w, who);
            match ids.split_first() {
                None => return Err(CmdError::msg(NO_SUCH_PARTY)),
                Some((only, [])) => *only,
                Some(_) => {
                    let list = ids
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Err(CmdError::msg(format!(
                        "{} ({list}); name one: {usage}.",
                        pool.prefix()
                    )));
                }
            }
        }
    };
    if !may_view(ctx.w, who, id) {
        return Err(CmdError::msg(NO_SUCH_PARTY));
    }
    ctx.w
        .module_state::<PartyStore>()
        .and_then(|s| s.party(id))
        .cloned()
        .ok_or_else(|| CmdError::msg(NO_SUCH_PARTY))
}

/// Resolve a character by name or dbref.
///
/// # Errors
///
/// `"No such player."` for anything that is not a live player or NPC — one message,
/// so a probe learns nothing about what does exist.
fn character(ctx: &Ctx<'_>, raw: &str) -> Result<ObjId, CmdError> {
    let id =
        match_object(ctx.w, ctx.actor, raw.trim()).map_err(|_| CmdError::msg("No such player."))?;
    if is_character(ctx.w, id) {
        Ok(id)
    } else {
        Err(CmdError::msg("No such player."))
    }
}

/// Turn a record-level refusal into the line a player reads.
fn open_refusal(e: PartyError) -> CmdError {
    match e {
        PartyError::Locked => CmdError::msg(
            "That party is locked to a job; its roster cannot change until the job's terms are amended.",
        ),
        PartyError::State => CmdError::msg("That party has disbanded."),
        PartyError::Empty | PartyError::NoStanding | PartyError::TooLarge(_) => {
            CmdError::msg(NO_SUCH_PARTY)
        }
    }
}

/// Capitalise a validator's message and end it with a full stop.
fn as_line(e: &str) -> String {
    let mut chars = e.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut out = String::with_capacity(e.len().saturating_add(1));
    out.extend(first.to_uppercase());
    out.push_str(chars.as_str());
    if !out.ends_with('.') {
        out.push('.');
    }
    out
}

fn notify(ctx: &mut Ctx<'_>, who: ObjId, msg: impl Into<String>) {
    ctx.out.sys_to_actor(who, NS, msg);
}

/// Tell every member except `except` — bounded by [`MAX_PARTY_SIZE`].
fn notify_members(ctx: &mut Ctx<'_>, p: &Party, except: ObjId, msg: &str) {
    let members: Vec<ObjId> = p
        .members
        .iter()
        .copied()
        .filter(|m| *m != except)
        .take(MAX_PARTY_SIZE)
        .collect();
    for m in members {
        notify(ctx, m, msg.to_string());
    }
}

// ---------------------------------------------------------------------------
// Verbs
// ---------------------------------------------------------------------------

/// `+party` — the actor's own parties and invitations; `+party <id>` — one party.
fn list(ctx: &mut Ctx<'_>, lhs: &str) -> Result<(), CmdError> {
    let who = ctx.actor;
    let mut lines: Vec<String> = Vec::new();

    if let Some(id) = only_id(lhs)? {
        if !may_view(ctx.w, who, id) {
            return Err(CmdError::msg(NO_SUCH_PARTY));
        }
        let Some(p) = ctx
            .w
            .module_state::<PartyStore>()
            .and_then(|s| s.party(id))
            .cloned()
        else {
            return Err(CmdError::msg(NO_SUCH_PARTY));
        };
        lines.extend(show_lines(ctx.w, who, &p));
    } else {
        let (mine, invites) = {
            let Some(store) = ctx.w.module_state::<PartyStore>() else {
                return Err(CmdError::msg("The party system is not running."));
            };
            let mine: Vec<Party> = store
                .live_parties()
                .filter(|p| p.is_member(who))
                .take(LIST_CAP)
                .cloned()
                .collect();
            let invites: Vec<Party> = store
                .live_parties()
                .filter(|p| p.is_invited(who))
                .take(LIST_CAP)
                .cloned()
                .collect();
            (mine, invites)
        };
        if mine.is_empty() && invites.is_empty() {
            lines.push("You are not in a party.".to_string());
        }
        if !mine.is_empty() {
            lines.push("Parties you are in:".to_string());
            for p in &mine {
                lines.extend(show_lines(ctx.w, who, p));
            }
        }
        if !invites.is_empty() {
            lines.push("Invitations:".to_string());
            for p in &invites {
                lines.push(format!(
                    "  {} {} — from {} (+party/accept {} or +party/decline {})",
                    p.id,
                    p.name,
                    render_actor(ctx.w, p.leader),
                    p.id,
                    p.id
                ));
            }
        }
    }

    for line in lines {
        notify(ctx, who, line);
    }
    Ok(())
}

/// One party, rendered. Roster and role labels only — a party holds nothing else.
///
/// The [`ContractRef`](crate::ContractRef) a `Locked` party carries is **the
/// ruleset's handle, not a player-facing fact** (E6a verify fold, D6): printing it
/// handed every member — and, before the lock prunes them, every invitee — an
/// economy-side contract id that the party itself cannot interpret and that the
/// contract's own view gate never admitted them to. Members read "locked to a job";
/// staff, who may already read the contract through the economy's own surface, keep
/// the id for support.
fn show_lines(w: &World, viewer: ObjId, p: &Party) -> Vec<String> {
    let status = match p.status {
        PartyStatus::Forming => "forming".to_string(),
        PartyStatus::Locked { by } => {
            if crate::module::is_staff(w, viewer) {
                format!("locked to a job (staff: job {by})")
            } else {
                "locked to a job".to_string()
            }
        }
        PartyStatus::Disbanded { how } => format!("disbanded ({})", how.label()),
    };
    let mut lines = vec![format!(
        "  {} {} — {}, led by {}",
        p.id,
        p.name,
        status,
        render_actor(w, p.leader)
    )];
    let roster: Vec<String> = p
        .members
        .iter()
        .take(MAX_PARTY_SIZE)
        .map(|m| match p.role_of(*m) {
            Some(label) => format!("{} [{label}]", render_actor(w, *m)),
            None => render_actor(w, *m),
        })
        .collect();
    if roster.is_empty() {
        lines.push("      (nobody)".to_string());
    } else {
        lines.push(format!("      {}", roster.join(", ")));
    }
    if !p.invited.is_empty() {
        let invited: Vec<String> = p
            .invited
            .iter()
            .take(MAX_PARTY_INVITES)
            .map(|i| render_actor(w, i.who))
            .collect();
        lines.push(format!("      invited: {}", invited.join(", ")));
    }
    lines
}

/// `+party/create <name>`.
fn create(ctx: &mut Ctx<'_>, raw: &str) -> Result<(), CmdError> {
    let name = raw.trim();
    check_label(name, MAX_PARTY_NAME_BYTES, "a party name")
        .map_err(|e| CmdError::msg(as_line(&e)))?;
    let who = ctx.actor;
    let now = ctx.w.now();

    let id = {
        let Some(store) = ctx.w.module_state_mut::<PartyStore>() else {
            return Err(CmdError::msg("The party system is not running."));
        };
        if store.live_parties_led_by(who).len() >= MAX_PARTIES_PER_LEADER {
            return Err(CmdError::msg(format!(
                "You already lead {MAX_PARTIES_PER_LEADER} parties. Disband one first."
            )));
        }
        if store.live_count() >= MAX_PARTIES {
            return Err(CmdError::msg(
                "There are too many parties right now. Try again later.",
            ));
        }
        match store.alloc_party() {
            Some(id) => id,
            None => return Err(CmdError::msg("No party can be created right now.")),
        }
    };

    let p = Party::new(id, name, who, now).map_err(|e| CmdError::msg(as_line(&e)))?;
    let label = p.name.to_string();
    put_party(ctx, p);
    persist_meta(ctx);
    notify(
        ctx,
        who,
        format!("Party {id} created: {label}. You lead it."),
    );
    Ok(())
}

/// `+party/invite [<id>] <who>`.
fn invite(ctx: &mut Ctx<'_>, lhs: &str) -> Result<(), CmdError> {
    let (explicit, name) = split_target(lhs);
    if name.is_empty() {
        return Err(CmdError::msg("Usage: +party/invite <who>."));
    }
    let mut p = resolve(ctx, explicit, Pool::Led, "+party/invite <id> <who>")?;
    let who = ctx.actor;
    if !p.is_leader(who) {
        return Err(CmdError::msg("You do not lead that party."));
    }
    p.check_open().map_err(open_refusal)?;
    let guest = character(ctx, &name)?;
    if p.is_member(guest) {
        return Err(CmdError::msg("They are already in that party."));
    }
    if p.is_invited(guest) {
        return Err(CmdError::msg("They are already invited."));
    }
    if p.check_room_for_invite().is_err() {
        return Err(CmdError::msg(format!(
            "That party already has {MAX_PARTY_INVITES} invitations outstanding."
        )));
    }
    let now = ctx.w.now();
    p.invited.push(Invitation::new(guest, who, now));
    p.touch(now);
    let (id, label) = (p.id, p.name.to_string());
    let guest_name = render_actor(ctx.w, guest);
    let leader_name = render_actor(ctx.w, who);
    put_party(ctx, p);
    notify(
        ctx,
        who,
        format!("You invited {guest_name} to {label} (party {id})."),
    );
    notify(
        ctx,
        guest,
        format!(
            "{leader_name} invited you to {label} (party {id}). \
             +party/accept {id} or +party/decline {id}."
        ),
    );
    Ok(())
}

/// `+party/uninvite [<id>] <who>`.
fn uninvite(ctx: &mut Ctx<'_>, lhs: &str) -> Result<(), CmdError> {
    let (explicit, name) = split_target(lhs);
    if name.is_empty() {
        return Err(CmdError::msg("Usage: +party/uninvite <who>."));
    }
    let mut p = resolve(ctx, explicit, Pool::Led, "+party/uninvite <id> <who>")?;
    let who = ctx.actor;
    if !p.is_leader(who) {
        return Err(CmdError::msg("You do not lead that party."));
    }
    p.check_open().map_err(open_refusal)?;
    let guest = character(ctx, &name)?;
    if !p.is_invited(guest) {
        return Err(CmdError::msg("They are not invited to that party."));
    }
    let now = ctx.w.now();
    p.invited.retain(|i| i.who != guest);
    p.touch(now);
    let id = p.id;
    let guest_name = render_actor(ctx.w, guest);
    put_party(ctx, p);
    notify(
        ctx,
        who,
        format!("You withdrew {guest_name}'s invitation to party {id}."),
    );
    Ok(())
}

/// `+party/accept [<id>]`.
fn accept(ctx: &mut Ctx<'_>, lhs: &str) -> Result<(), CmdError> {
    let explicit = only_id(lhs)?;
    let mut p = resolve(ctx, explicit, Pool::Invited, "+party/accept <id>")?;
    let who = ctx.actor;
    if p.is_member(who) {
        return Err(CmdError::msg("You are already in that party."));
    }
    p.check_open().map_err(open_refusal)?;
    let now = ctx.w.now();
    // Only staff reach the `else`: everybody else who is not invited was already
    // refused with `NO_SUCH_PARTY` by `resolve`.
    let Some(inv) = p.invitation_of(who).copied() else {
        return Err(CmdError::msg("You are not invited to that party."));
    };
    if inv.is_expired(now) {
        p.invited.retain(|i| i.who != who);
        p.bump();
        put_party(ctx, p);
        return Err(CmdError::msg("That invitation has expired."));
    }
    if p.check_room_for_member().is_err() {
        return Err(CmdError::msg(format!(
            "That party is full ({MAX_PARTY_SIZE} members)."
        )));
    }
    p.invited.retain(|i| i.who != who);
    p.members.push(who);
    p.touch(now);
    let (id, label) = (p.id, p.name.to_string());
    let joined = format!("{} joined {label} (party {id}).", render_actor(ctx.w, who));
    notify_members(ctx, &p, who, &joined);
    put_party(ctx, p);
    notify(ctx, who, format!("You joined {label} (party {id})."));
    Ok(())
}

/// `+party/decline [<id>]`.
fn decline(ctx: &mut Ctx<'_>, lhs: &str) -> Result<(), CmdError> {
    let explicit = only_id(lhs)?;
    let mut p = resolve(ctx, explicit, Pool::Invited, "+party/decline <id>")?;
    let who = ctx.actor;
    p.check_open().map_err(open_refusal)?;
    if !p.is_invited(who) {
        return Err(CmdError::msg("You are not invited to that party."));
    }
    let now = ctx.w.now();
    p.invited.retain(|i| i.who != who);
    p.touch(now);
    let (id, leader, label) = (p.id, p.leader, p.name.to_string());
    put_party(ctx, p);
    notify(ctx, who, format!("You declined {label} (party {id})."));
    let line = format!(
        "{} declined {label} (party {id}).",
        render_actor(ctx.w, who)
    );
    notify(ctx, leader, line);
    Ok(())
}

/// `+party/leave [<id>]`.
fn leave(ctx: &mut Ctx<'_>, lhs: &str) -> Result<(), CmdError> {
    let explicit = only_id(lhs)?;
    let mut p = resolve(ctx, explicit, Pool::Member, "+party/leave <id>")?;
    let who = ctx.actor;
    p.check_open().map_err(open_refusal)?;
    if !p.is_member(who) {
        return Err(CmdError::msg("You are not in that party."));
    }
    let now = ctx.w.now();
    let was_leader = p.is_leader(who);
    if !p.remove_member(who) {
        return Err(CmdError::msg("You are not in that party."));
    }
    let (id, label) = (p.id, p.name.to_string());
    let mut mine = format!("You left {label} (party {id}).");
    if p.is_terminal() {
        mine.push_str(" It disbanded — you were the last member.");
    } else {
        p.touch(now);
        if was_leader {
            let next = render_actor(ctx.w, p.leader);
            mine.push_str(&format!(" {next} now leads it."));
        }
        let line = format!("{} left {label} (party {id}).", render_actor(ctx.w, who));
        notify_members(ctx, &p, who, &line);
    }
    put_party(ctx, p);
    notify(ctx, who, mine);
    Ok(())
}

/// `+party/kick [<id>] <who>`.
fn kick(ctx: &mut Ctx<'_>, lhs: &str) -> Result<(), CmdError> {
    let (explicit, name) = split_target(lhs);
    if name.is_empty() {
        return Err(CmdError::msg("Usage: +party/kick <who>."));
    }
    let mut p = resolve(ctx, explicit, Pool::Led, "+party/kick <id> <who>")?;
    let who = ctx.actor;
    if !p.is_leader(who) {
        return Err(CmdError::msg("You do not lead that party."));
    }
    p.check_open().map_err(open_refusal)?;
    let victim = character(ctx, &name)?;
    if victim == p.leader {
        return Err(CmdError::msg(
            "You lead that party; use +party/disband to end it.",
        ));
    }
    if !p.is_member(victim) {
        return Err(CmdError::msg("They are not in that party."));
    }
    let now = ctx.w.now();
    if !p.remove_member(victim) {
        return Err(CmdError::msg("They are not in that party."));
    }
    p.touch(now);
    let (id, label) = (p.id, p.name.to_string());
    let victim_name = render_actor(ctx.w, victim);
    let line = format!("{victim_name} was removed from {label} (party {id}).");
    notify_members(ctx, &p, victim, &line);
    put_party(ctx, p);
    notify(
        ctx,
        victim,
        format!("You were removed from {label} (party {id})."),
    );
    Ok(())
}

/// `+party/role [<id>] <who>=<label>` — an empty value clears.
fn role(ctx: &mut Ctx<'_>, lhs: &str, rhs: Option<&str>) -> Result<(), CmdError> {
    let (explicit, name) = split_target(lhs);
    if name.is_empty() {
        return Err(CmdError::msg("Usage: +party/role <who>=<label>."));
    }
    let Some(label) = rhs else {
        return Err(CmdError::msg("Usage: +party/role <who>=<label>."));
    };
    let mut p = resolve(ctx, explicit, Pool::Led, "+party/role <id> <who>=<label>")?;
    let who = ctx.actor;
    if !p.is_leader(who) {
        return Err(CmdError::msg("You do not lead that party."));
    }
    p.check_open().map_err(open_refusal)?;
    let member = character(ctx, &name)?;
    if !p.is_member(member) {
        return Err(CmdError::msg("They are not in that party."));
    }
    let label = label.trim();
    let member_name = render_actor(ctx.w, member);
    let line = if label.is_empty() {
        p.roles.remove(&member);
        format!("Cleared {member_name}'s role.")
    } else {
        check_label(label, MAX_PARTY_ROLE_BYTES, "a role label")
            .map_err(|e| CmdError::msg(as_line(&e)))?;
        p.roles.insert(member, SmolStr::new(label));
        format!("{member_name} is now the {label}.")
    };
    // A role is activity: a leader labelling the crew is a leader working on the
    // crew, and idle expiry must not collect a party somebody is organising.
    let now = ctx.w.now();
    p.touch(now);
    put_party(ctx, p);
    notify(ctx, who, line);
    Ok(())
}

/// `+party/disband [<id>]`.
fn disband(ctx: &mut Ctx<'_>, lhs: &str) -> Result<(), CmdError> {
    let explicit = only_id(lhs)?;
    let mut p = resolve(ctx, explicit, Pool::Led, "+party/disband <id>")?;
    let who = ctx.actor;
    if !p.is_leader(who) {
        return Err(CmdError::msg("You do not lead that party."));
    }
    p.check_open().map_err(open_refusal)?;
    p.disband(DisbandCause::Disbanded);
    let (id, label) = (p.id, p.name.to_string());
    let line = format!("{label} (party {id}) has disbanded.");
    notify_members(ctx, &p, who, &line);
    put_party(ctx, p);
    notify(ctx, who, format!("You disbanded {label} (party {id})."));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Verb, as_line, only_id, split_target, verb_of};
    use crate::ids::PartyId;
    use mush_core::SwitchSet;

    #[test]
    fn a_leading_id_needs_a_second_word() {
        assert_eq!(split_target("Bob"), (None, "Bob".to_string()));
        assert_eq!(
            split_target("#12"),
            (None, "#12".to_string()),
            "a lone dbref is a target, never party 12"
        );
        assert_eq!(split_target("3 Bob"), (Some(PartyId(3)), "Bob".to_string()));
        assert_eq!(
            split_target("#3 #12"),
            (Some(PartyId(3)), "#12".to_string())
        );
        assert_eq!(
            split_target("Bob Smith"),
            (None, "Bob Smith".to_string()),
            "a non-id first word leaves the whole string as the target"
        );
        assert_eq!(split_target("   "), (None, String::new()));
    }

    #[test]
    fn a_bare_id_argument_parses_or_hides() {
        assert!(only_id("").expect("empty is fine").is_none());
        assert_eq!(only_id("7").expect("id"), Some(PartyId(7)));
        assert_eq!(only_id("#7").expect("id"), Some(PartyId(7)));
        assert!(only_id("Bob").is_err(), "a malformed id hides");
    }

    #[test]
    fn the_first_recognised_switch_wins_and_bare_lists() {
        assert_eq!(
            verb_of(&SwitchSet::from_names(std::iter::empty())),
            Verb::List
        );
        assert_eq!(verb_of(&SwitchSet::from_names(["Create"])), Verb::Create);
        assert_eq!(verb_of(&SwitchSet::from_names(["disband"])), Verb::Disband);
    }

    #[test]
    fn validator_messages_become_lines() {
        assert_eq!(
            as_line("a party name may not be empty"),
            "A party name may not be empty."
        );
        assert_eq!(as_line("already ends."), "Already ends.");
        assert_eq!(as_line(""), "");
    }
}
