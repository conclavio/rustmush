//! Speaking on channels: the [`AliasStage`] (treaty ruling 5's dedicated
//! dispatch stage), the delivery fan-out, and the LOUD login notice.
//!
//! The stage claims a line whose first word is one of the actor's channel
//! aliases (`pub Hello`, `pub :waves`, `pub ;grins`). Once claimed the line
//! is always [`AliasOutcome::Handled`] — errors (gagged, speak lock, empty
//! message) are reported on the channel's behalf, never leaked to `Huh?`.
//!
//! Delivery emits one [`OutputEvent::Channel`] per receiving member as
//! `Audience::Actor` — the session manager fans each actor out to their
//! live connections. A member receives a message iff they are **connected
//! with at least one live connection** (link-dead members receive nothing,
//! so their cursor must not move), **ungagged**, **pass the channel's see
//! lock**, and are **not `@ignore`-ing the speaker**; exactly those members
//! have their [`crate::data::Membership::last_read`] cursor advanced (and mirrored),
//! which is what makes the login unread notice honest.

use mush_core::lock::MAX_LOCK_DEPTH;
use mush_core::output::obj_ref;
use mush_core::{
    AliasOutcome, AliasStage, Audience, Ctx, ModuleWrite, ObjId, Outbox, RelayRefusal,
    RelaySpeaker, World, eval_lock, is_ignoring,
};
use mush_proto::{NarrativeKind, OutputEvent, RichText};
use smol_str::SmolStr;

use crate::data::{ChanMsg, ChannelId, ComsysState, chan_key, hist_key, member_key, push_op};

/// The comsys alias stage (registered via `Registrar::alias_stage`).
pub struct ComsysAliases;

impl AliasStage for ComsysAliases {
    fn try_alias(&self, w: &mut World, actor: ObjId, line: &str, out: &mut Outbox) -> AliasOutcome {
        let (head, rest) = split_head(line);
        if head.is_empty() {
            return AliasOutcome::NotMine;
        }
        let Some(chan) = w
            .module_state::<ComsysState>()
            .and_then(|st| st.membership_by_alias(actor, head).map(|m| m.channel))
        else {
            return AliasOutcome::NotMine;
        };
        // From here the line is ours: always Handled.
        let Some((chan_name, gagged, speak)) = w.module_state::<ComsysState>().and_then(|st| {
            let m = st.members.get(&(actor, chan))?;
            let c = st.channels.get(&chan)?;
            Some((c.name.clone(), m.gagged, c.perms.speak.clone()))
        }) else {
            out.error(actor, "That channel no longer exists.");
            return AliasOutcome::Handled;
        };
        if rest.is_empty() {
            out.error(
                actor,
                format!(
                    "What do you want to say on {chan_name}?  \
                     ({head} <message>, {head} :<pose>, {head} ;<pose>)"
                ),
            );
            return AliasOutcome::Handled;
        }
        if gagged {
            out.error(
                actor,
                format!("You have {chan_name} gagged.  (@channel/ungag {head} to listen again.)"),
            );
            return AliasOutcome::Handled;
        }
        if !eval_lock(w, &speak, actor, MAX_LOCK_DEPTH) {
            out.error(actor, format!("You cannot speak on {chan_name}."));
            return AliasOutcome::Handled;
        }
        let (kind, text) = pose_split(rest);
        if text.trim().is_empty() {
            out.error(actor, format!("What do you want to say on {chan_name}?"));
            return AliasOutcome::Handled;
        }
        channel_speak(w, out, actor, chan, kind, text, None);
        AliasOutcome::Handled
    }
}

/// Split off the first whitespace-delimited word.
fn split_head(s: &str) -> (&str, &str) {
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], s[i..].trim_start()),
        None => (s, ""),
    }
}

/// The TinyMUX pose forms: `:waves` → pose, `;grins` → semipose, anything
/// else is said.
fn pose_split(rest: &str) -> (NarrativeKind, &str) {
    if let Some(t) = rest.strip_prefix(':') {
        (NarrativeKind::Pose, t)
    } else if let Some(t) = rest.strip_prefix(';') {
        (NarrativeKind::SemiPose, t)
    } else {
        (NarrativeKind::Say, rest)
    }
}

/// Emit one channel message: bump the channel `seq` (mirroring the channel
/// doc), deliver to every connected, ungagged, see-passing member (advancing
/// and mirroring their `last_read`), and record history (ring always when
/// LOGGED; `hist:{id}` stream when LOGGED **and** the policy persists).
///
/// Callers have already verified the speaker may speak (gag + speak lock).
pub(crate) fn channel_speak(
    w: &mut World,
    out: &mut Outbox,
    actor: ObjId,
    chan: ChannelId,
    kind: NarrativeKind,
    text: &str,
    origin: Option<&str>,
) {
    let now_ms = w.now().saturating_mul(1000);
    let actor_ref = obj_ref(w, actor);

    // Phase 1: bump seq, snapshot what delivery needs, then release the
    // mutable module-state borrow before selecting the live audience.
    let (seq, chan_name, flags, hist, see, chan_doc, title) = {
        let Some(st) = w.module_state_mut::<ComsysState>() else {
            return;
        };
        let Some(c) = st.channels.get_mut(&chan) else {
            return;
        };
        c.seq += 1;
        let seq = c.seq;
        let chan_name = c.name.clone();
        let flags = c.flags;
        let hist = c.history;
        let see = c.perms.see.clone();
        let chan_doc = serde_json::to_value(&*c).expect("channel doc serializes");
        let title = if flags.no_titles {
            None
        } else {
            st.members.get(&(actor, chan)).and_then(|m| m.title.clone())
        };
        (seq, chan_name, flags, hist, see, chan_doc, title)
    };
    push_op(
        out,
        ModuleWrite::JsonSet {
            key: chan_key(chan),
            value: chan_doc,
        },
    );

    // Phase 2: audience = actually deliverable ∧ ungagged ∧ see-lock pass
    // ∧ not ignoring the speaker. `has_live_conns` (not `is_connected`):
    // a link-dead member is still world-connected but receives nothing —
    // advancing their cursor would mark messages read that were never
    // delivered, so they are excluded from the deliver set entirely and
    // get the unread notice on reconnect instead. `@ignore` applies to
    // channel traffic exactly as it does to room speech (staff speakers
    // are never filtered — `is_ignoring` owns that policy).
    // Iterate the world's live-side index rather than materializing every
    // ungagged membership first. Offline memberships can be numerous and can
    // never receive this send; production lifecycle admission independently
    // bounds this connected set.
    let st = w
        .module_state::<ComsysState>()
        .expect("comsys state remains installed during a synchronous send");
    let deliver: Vec<ObjId> = w
        .connected()
        .filter(|&p| {
            st.members
                .get(&(p, chan))
                .is_some_and(|membership| !membership.gagged)
                && w.has_live_conns(p)
                && eval_lock(w, &see, p, MAX_LOCK_DEPTH)
                && !is_ignoring(w, p, actor)
        })
        .collect();

    // Phase 3: emit, advance cursors, record history.
    let ev = OutputEvent::Channel {
        channel: chan_name.clone(),
        seq,
        actor: actor_ref.clone(),
        title: title.clone(),
        kind,
        text: RichText::plain(text),
        ts: now_ms,
    };
    // Programme D4: a relay-flagged channel publishes the line to its named
    // relay sink as well — unless the line came in from the relay (the loop
    // guard: a Discord user's own line is never mirrored back to Discord).
    if flags.relay && origin != Some("discord") {
        out.push(Audience::Relay(chan_name), ev.clone());
    }
    for &p in &deliver {
        out.push(Audience::Actor(p), ev.clone());
        // Interface plan I11: a line that names a recipient (a whole word,
        // case-insensitive, never the speaker) also lands on their
        // notification surface, with their own alias for the channel ready
        // as the reply. Same audience as the line itself — the deliver set
        // already applied live-conns, gag, see-lock and @ignore.
        if p != actor {
            let name = obj_ref(w, p).name;
            let named = text
                .split(|c: char| !c.is_alphanumeric())
                .any(|word| !word.is_empty() && word.eq_ignore_ascii_case(&name));
            if named && let Some(m) = st.members.get(&(p, chan)) {
                let alias = m.alias.clone();
                out.notice_info_to_actor(
                    p,
                    now_ms,
                    "mention",
                    format!("{} on {alias}: {text}", actor_ref.name),
                    Some(("Reply", &format!("{alias} "))),
                );
            }
        }
    }
    let msg = ChanMsg {
        seq,
        actor,
        actor_name: actor_ref.name,
        title,
        kind,
        text: text.to_string(),
        ts: now_ms,
        origin: origin.map(SmolStr::new),
    };
    let st = w.module_state_mut::<ComsysState>().expect("comsys state");
    for &p in &deliver {
        if let Some(m) = st.members.get_mut(&(p, chan))
            && m.last_read != seq
        {
            m.last_read = seq;
            let value = serde_json::to_value(&*m).expect("membership doc serializes");
            push_op(
                out,
                ModuleWrite::JsonSet {
                    key: member_key(p, chan),
                    value,
                },
            );
        }
    }
    if flags.logged {
        let cap = hist.cap.max(1);
        let ring = st.rings.entry(chan).or_default();
        ring.push_back(msg.clone());
        while ring.len() > cap as usize {
            ring.pop_front();
        }
        if hist.persist {
            push_op(
                out,
                ModuleWrite::StreamAppend {
                    key: hist_key(chan),
                    value: serde_json::to_value(msg).expect("chan msg serializes"),
                    maxlen: cap,
                },
            );
        }
    }
}

/// Deliver a system notice line (`[Public] Nick has joined this channel.`)
/// to every connected, ungagged, see-passing member. `about` is the actor
/// the notice concerns (joiner, leaver, destroyer): members `@ignore`-ing
/// them are skipped, same as for their speech. Notices do not bump `seq`
/// and are never logged.
pub(crate) fn channel_notice(
    w: &World,
    out: &mut Outbox,
    chan: ChannelId,
    about: Option<ObjId>,
    text: &str,
) {
    let Some(st) = w.module_state::<ComsysState>() else {
        return;
    };
    let Some(c) = st.channels.get(&chan) else {
        return;
    };
    let line = format!("[{}] {text}", c.name);
    let see = &c.perms.see;
    for m in st.members_of(chan) {
        if m.gagged || !w.is_connected(m.player) || !eval_lock(w, see, m.player, MAX_LOCK_DEPTH) {
            continue;
        }
        if let Some(about) = about
            && is_ignoring(w, m.player, about)
        {
            continue;
        }
        out.sys_to_actor(m.player, "channel", line.clone());
    }
}

/// The LOUD login notice lines for one actor: one per ungagged membership
/// of a LOUD channel with messages beyond their `last_read` cursor.
/// Memberships failing the channel's **see lock** are skipped (delivery
/// semantics): a member barred from receiving a channel must not have its
/// traffic volume leaked — and their cursor never advances, so the count
/// would only ever grow.
#[must_use]
pub(crate) fn unread_notices(w: &World, actor: ObjId) -> Vec<String> {
    let Some(st) = w.module_state::<ComsysState>() else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    for m in st.memberships_of(actor) {
        if m.gagged {
            continue;
        }
        let Some(c) = st.channels.get(&m.channel) else {
            continue;
        };
        if !c.flags.loud || !eval_lock(w, &c.perms.see, actor, MAX_LOCK_DEPTH) {
            continue;
        }
        let unread = c.seq.saturating_sub(m.last_read);
        if unread > 0 {
            lines.push(format!(
                "[{}] {unread} message{} since you last read.",
                c.name,
                if unread == 1 { "" } else { "s" }
            ));
        }
    }
    lines
}

/// One recall line for a recorded message, in the channel formatter shapes:
/// say → `Name: text`, pose → `Name text`, semipose → `Nametext` (titles
/// prefixed when recorded).
#[must_use]
pub(crate) fn render_recall(msg: &ChanMsg) -> String {
    let name = match &msg.title {
        Some(t) => format!("{t} {}", msg.actor_name),
        None => msg.actor_name.clone(),
    };
    match msg.kind {
        NarrativeKind::Pose => format!("{name} {}", msg.text),
        NarrativeKind::SemiPose => format!("{name}{}", msg.text),
        _ => format!("{name}: {}", msg.text),
    }
}

/// The relay's way onto a channel (interface programme D4): a linked Discord
/// user's line, forwarded by the gateway and re-validated by the server,
/// passes the same gates as a typed line — the channel must be flagged
/// `relay`, the character a member, ungagged, and admitted by the speak lock —
/// and is spoken with its origin marked so it is never mirrored back out.
pub struct RelaySpeak;

impl RelaySpeaker for RelaySpeak {
    fn relay_speak(
        &self,
        ctx: &mut Ctx<'_>,
        channel: &str,
        kind: NarrativeKind,
        text: &str,
    ) -> Result<(), RelayRefusal> {
        let actor = ctx.actor;
        let text = text.trim();
        if text.is_empty() {
            return Err(RelayRefusal::Empty);
        }
        let (chan, gagged, speak) = {
            let st = ctx
                .w
                .module_state::<ComsysState>()
                .ok_or(RelayRefusal::NoSpeaker)?;
            let c = st
                .channel_by_name(channel)
                .ok_or(RelayRefusal::NoSuchChannel)?;
            if !c.flags.relay {
                return Err(RelayRefusal::NotRelay);
            }
            let m = st
                .members
                .get(&(actor, c.id))
                .ok_or(RelayRefusal::NotMember)?;
            (c.id, m.gagged, c.perms.speak.clone())
        };
        if gagged {
            return Err(RelayRefusal::Gagged);
        }
        if !eval_lock(ctx.w, &speak, actor, MAX_LOCK_DEPTH) {
            return Err(RelayRefusal::SpeakLock);
        }
        channel_speak(ctx.w, ctx.out, actor, chan, kind, text, Some("discord"));
        Ok(())
    }
}
