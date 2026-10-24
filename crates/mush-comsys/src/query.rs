//! Web `Req` hydration for comsys (M4): the `"comsys.state"` and
//! `"comsys.backlog"` query handlers, registered via
//! [`Registrar::query_handler`](mush_core::Registrar::query_handler) and
//! invoked by the engine's `Inbound::Query` handling. Replies arrive at the
//! client as `Reply { id, event: Panel { feed: "comsys", .. } }` frames.
//!
//! Both handlers are **read-only** (they never advance `last_read`
//! cursors — cursors move only on live delivery) and enforce the channel
//! **see lock with no admin bypass**, exactly like live delivery and
//! `@channel/recall`: hydration is delivery, and a member barred from
//! hearing a channel must not read its state or history through a panel
//! either.

use mush_core::{ObjId, QueryHandler, World};
use mush_proto::{OutputEvent, PanelFrameKind, RichText};
use smol_str::SmolStr;

use crate::data::{ChanMsg, Channel, ComsysState, Membership};

/// Hard cap on one `comsys.backlog` reply (the client pages with
/// `before_seq`).
pub const MAX_BACKLOG_LIMIT: u32 = 200;

fn err(msg: impl Into<String>) -> RichText {
    RichText::tag("error", msg)
}

fn state_of(w: &World) -> Result<&ComsysState, RichText> {
    w.module_state::<ComsysState>()
        .ok_or_else(|| err("Comsys is not loaded."))
}

/// One backlog entry / recall message as JSON for the web panel: the same
/// field names the live [`OutputEvent::Channel`] frame carries, so the
/// client renders both with one code path.
fn msg_json(m: &ChanMsg) -> serde_json::Value {
    serde_json::json!({
        "seq": m.seq,
        "actor": { "id": m.actor.to_string(), "name": m.actor_name },
        "title": m.title,
        "kind": serde_json::to_value(m.kind).expect("kind serializes"),
        "text": m.text,
        "ts": m.ts,
    })
}

/// `"comsys.state"` — the channel-tab panel keyframe: every channel the
/// actor has joined **and may see**, with alias, comtitle, gag state, the
/// channel color (ANSI name + web hex), flags, and the unread count derived
/// from the channel `seq` vs. the member's `last_read` cursor.
///
/// Reply payload: `Panel { feed: "comsys", kind: Keyframe, data:
/// { "channels": [ { name, alias, title, gagged, color: { ansi, web_hex },
/// seq, last_read, unread, loud, logged } ] } }`.
pub struct StateQuery;

impl QueryHandler for StateQuery {
    fn query(
        &self,
        w: &World,
        actor: ObjId,
        _params: &serde_json::Value,
    ) -> Result<OutputEvent, RichText> {
        let st = state_of(w)?;
        let mut channels = Vec::new();
        for m in st.memberships_of(actor) {
            let Some(c) = st.channels.get(&m.channel) else {
                continue; // orphaned membership; boot cleanup owns it
            };
            // Delivery semantics: the see lock, no admin bypass — through
            // `Channel::hears`, so a closed channel's member keeps its tab (16b
            // cut Z.4: its see lock is what hides it from everybody else).
            if !c.hears(w, actor) {
                continue;
            }
            channels.push(serde_json::json!({
                "name": c.name,
                "alias": m.alias,
                "title": m.title,
                "gagged": m.gagged,
                "color": { "ansi": c.color.ansi, "web_hex": c.color.web_hex },
                "seq": crate::transit::sequence(c,m),
                "last_read": m.last_read,
                "unread": crate::transit::sequence(c,m).saturating_sub(m.last_read),
                "loud": c.flags.loud,
                "logged": c.flags.logged,
                "ic": c.flags.ic,
                "epoch": m.ic.as_ref().map(|k| &k.epoch),
            }));
        }
        Ok(OutputEvent::Panel {
            feed: SmolStr::new_static("comsys"),
            kind: PanelFrameKind::Keyframe,
            data: serde_json::json!({ "channels": channels }),
        })
    }
}

/// `"comsys.backlog"` — a history slice for one channel tab.
///
/// Params: `{ "channel": <name or personal alias>, "before_seq": <u64 |
/// null>, "limit": <u32> }` (mirrors
/// [`Query::ChannelBacklog`](mush_proto::Query::ChannelBacklog): messages
/// with `seq` strictly below `before_seq`, newest-first; `null` starts from
/// the newest; `limit` is clamped to 1..=[`MAX_BACKLOG_LIMIT`]).
///
/// Gates, in order: the actor must be **on** the channel, must pass its
/// **see lock** (no admin bypass — recall semantics), and the channel must
/// be LOGGED. History comes from the module's in-RAM recall ring (the same
/// source `@channel/recall` reads).
///
/// Reply payload: `Panel { feed: "comsys", kind: Delta, data:
/// { "backlog": { "channel", "before_seq", "messages": [ … newest-first,
/// same shape as live Channel frames ] } } }`.
pub struct BacklogQuery;

impl BacklogQuery {
    /// Resolve `spec` to the actor's membership: channel name first
    /// (case-insensitive — what the client sees in `Channel` frames), then
    /// the actor's personal alias.
    fn resolve<'a>(
        st: &'a ComsysState,
        actor: ObjId,
        spec: &str,
    ) -> Option<(&'a Channel, &'a Membership)> {
        if let Some(c) = st.channel_by_name(spec)
            && let Some(m) = st.members.get(&(actor, c.id))
        {
            return Some((c, m));
        }
        let m = st.membership_by_alias(actor, spec)?;
        st.channels.get(&m.channel).map(|c| (c, m))
    }
}

impl QueryHandler for BacklogQuery {
    fn query(
        &self,
        w: &World,
        actor: ObjId,
        params: &serde_json::Value,
    ) -> Result<OutputEvent, RichText> {
        let st = state_of(w)?;
        let spec = params
            .get("channel")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| err("Backlog of which channel?"))?;
        let before_seq = params.get("before_seq").and_then(serde_json::Value::as_u64);
        let limit = params
            .get("limit")
            .and_then(serde_json::Value::as_u64)
            .map_or(MAX_BACKLOG_LIMIT, |l| {
                u32::try_from(l).unwrap_or(MAX_BACKLOG_LIMIT)
            })
            .clamp(1, MAX_BACKLOG_LIMIT);

        let Some((c, membership)) = Self::resolve(st, actor, spec) else {
            // Membership-gated like recall: not distinguishing "no such
            // channel" from "not joined" keeps see-locked channels
            // unenumerable.
            return Err(err("You are not on that channel."));
        };
        // 16b cut Z.4: `Channel::hears`, as live delivery.
        if !c.hears(w, actor) {
            return Err(err(format!("You cannot listen to {}.", c.name)));
        }
        if !c.flags.logged {
            return Err(err(format!(
                "{} does not log history.  (LOGGED is off.)",
                c.name
            )));
        }
        let ring = crate::transit::history(st, membership);
        let messages: Vec<serde_json::Value> = {
            ring.iter()
                .rev() // newest-first
                .filter(|m| before_seq.is_none_or(|b| m.seq < b))
                .take(limit as usize)
                .map(msg_json)
                .collect()
        };
        Ok(OutputEvent::Panel {
            feed: SmolStr::new_static("comsys"),
            kind: PanelFrameKind::Delta,
            data: serde_json::json!({
                "backlog": {
                    "channel": c.name,
                    "epoch": membership.ic.as_ref().map(|k| &k.epoch),
                    "before_seq": before_seq,
                    "messages": messages,
                }
            }),
        })
    }
}
