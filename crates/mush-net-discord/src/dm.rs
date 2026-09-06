//! The DM writer (D2, D-D-6): the lossy render of one follow connection.
//!
//! A follow is a structured connection of the character's own session, so
//! the events arriving here have already passed every gate the terminal has
//! (perception, `@ignore`, the room fan-out). What this module adds is the
//! **loss**: an allow-list of what a DM shows ([`render`]), plain text with
//! Discord's markdown escaped, coalescing into bounded messages, and a drop
//! policy under pressure — oldest narrative first, with a visible marker —
//! that never drops a [`OutputEvent::Notice`] or a [`OutputEvent::Request`].
//!
//! The one invariant the writer must keep is the session's slow-client
//! trip-wire: it **drains its channel eagerly** into its own bounded buffer
//! and never awaits Discord while the channel could fill.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use mush_proto::{Envelope, NarrativeKind, NoticeSeverity, OutputEvent, RequestKind, RichText};
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::api::{ApiError, DiscordApi, Outgoing};

/// How long a failed (retryable) send waits before the next attempt.
pub const RETRY_BACKOFF: Duration = Duration::from_secs(5);

/// Longest room description the compact `look` render keeps, characters.
pub const ROOM_DESC_CHARS: usize = 300;

/// The writer's tunables (`[discord].dm_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmPolicy {
    /// How long to gather lines before sending one message.
    pub coalesce: Duration,
    /// Longest message the writer emits (Discord's cap is 2 000).
    pub max_chars: usize,
    /// Rendered lines buffered before the oldest narrative is dropped.
    pub queue_lines: usize,
}

impl DmPolicy {
    /// Build a policy, clamping nonsense to workable minimums.
    #[must_use]
    pub fn new(coalesce_ms: u64, max_chars: usize, queue_lines: usize) -> DmPolicy {
        DmPolicy {
            coalesce: Duration::from_millis(coalesce_ms.max(50)),
            max_chars: max_chars.clamp(200, 1_990),
            queue_lines: queue_lines.max(10),
        }
    }
}

/// What the writer has learned about the follow, shared with the bridge:
/// whether the character is in a scene (the idle sweeper never detaches a
/// scene member), the quiet preference, and whether Discord closed the DM.
#[derive(Debug, Default)]
pub struct FollowState {
    /// The `scenes` keyframe said the character is in an active scene.
    pub in_scene: AtomicBool,
    /// Quiet follows show only notices, requests and the character's scene.
    pub quiet: AtomicBool,
    /// A send failed with [`ApiError::CannotDm`]; the follow is deaf.
    pub dm_closed: AtomicBool,
    /// The followed character's object id as the world writes it in
    /// `ObjRef::id` (`#7`), so the render can say "You" for their own speech
    /// the way the terminal does.
    pub self_id: String,
}

impl FollowState {
    /// A state for a follow of `self_id` with the given quiet preference.
    #[must_use]
    pub fn new(quiet: bool, self_id: impl Into<String>) -> Arc<FollowState> {
        let s = FollowState {
            self_id: self_id.into(),
            ..FollowState::default()
        };
        s.quiet.store(quiet, Ordering::Relaxed);
        Arc::new(s)
    }
}

/// What one event becomes on the DM.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rendered {
    /// Not shown.
    Drop,
    /// A narrative-class line: coalesced, and the first to go under pressure.
    Line(String),
    /// A notice or request: sent promptly, never dropped.
    Priority(String),
}

/// Escape Discord's inline markdown so player text renders as typed.
#[must_use]
pub fn escape_md(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        if matches!(c, '*' | '_' | '~' | '`' | '|' | '>' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn first_tag(text: &RichText) -> Option<&str> {
    text.0.first().and_then(|s| s.tag.as_deref())
}

/// One narrative event as a line; `own` when the actor is the followed
/// character (second person for speech, as the terminal renders it).
fn narrative_line(kind: NarrativeKind, actor: Option<&str>, own: bool, text: &str) -> String {
    let name = actor.unwrap_or("Someone");
    match kind {
        NarrativeKind::Say if own => format!("You say, \"{text}\""),
        NarrativeKind::Say => format!("{name} says, \"{text}\""),
        NarrativeKind::Pose => format!("{name} {text}"),
        NarrativeKind::SemiPose => format!("{name}{text}"),
        NarrativeKind::Emit | NarrativeKind::Spoof => text.to_string(),
        NarrativeKind::Page => format!("{name} pages: {text}"),
        NarrativeKind::Whisper if own => format!("You whisper, \"{text}\""),
        NarrativeKind::Whisper => format!("{name} whispers, \"{text}\""),
        NarrativeKind::Ooc => match actor {
            Some(name) => format!("<OOC> {name} {text}"),
            None => format!("<OOC> {text}"),
        },
    }
}

/// The allow-list (04 §3.2): what a follow's DM shows, and how.
///
/// Quiet mode keeps notices, requests, the `scenes` line and — while the
/// character is in a scene — narrative; everything ambient is dropped.
#[must_use]
pub fn render(event: &OutputEvent, state: &FollowState) -> Rendered {
    let quiet = state.quiet.load(Ordering::Relaxed);
    let in_scene = state.in_scene.load(Ordering::Relaxed);
    match event {
        OutputEvent::Narrative { kind, actor, text } => {
            if quiet && !in_scene {
                return Rendered::Drop;
            }
            let own = actor.as_ref().is_some_and(|a| a.id == state.self_id);
            let line = narrative_line(
                *kind,
                actor.as_ref().map(|a| a.name.as_str()),
                own,
                &text.to_plain_text(),
            );
            Rendered::Line(escape_md(&line))
        }
        OutputEvent::Sys(text) => match first_tag(text) {
            Some("scene" | "discord") => Rendered::Line(escape_md(&text.to_plain_text())),
            Some("error" | "system" | "channel" | "privilege") if !quiet => {
                Rendered::Line(escape_md(&text.to_plain_text()))
            }
            _ => Rendered::Drop,
        },
        OutputEvent::Room(view) if !quiet => {
            let mut desc: String = view.desc.to_plain_text();
            if desc.chars().count() > ROOM_DESC_CHARS {
                desc = desc.chars().take(ROOM_DESC_CHARS).collect::<String>() + "…";
            }
            let mut s = format!("**{}**", escape_md(&view.name));
            if !desc.trim().is_empty() {
                s.push('\n');
                s.push_str(&escape_md(desc.trim()));
            }
            if !view.contents.is_empty() {
                let names: Vec<String> = view.contents.iter().map(|o| escape_md(&o.name)).collect();
                s.push_str(&format!("\nHere: {}", names.join(", ")));
            }
            if !view.exits.is_empty() {
                let names: Vec<String> = view.exits.iter().map(|e| escape_md(&e.name)).collect();
                s.push_str(&format!("\nExits: {}", names.join(", ")));
            }
            Rendered::Line(s)
        }
        OutputEvent::Notice {
            severity,
            category,
            text,
            action,
            ..
        } => {
            let marker = match severity {
                NoticeSeverity::Info => "•",
                NoticeSeverity::Warn => "⚠",
                NoticeSeverity::Urgent => "‼",
            };
            let mut s = format!(
                "{marker} [{}] {}",
                escape_md(category),
                escape_md(&text.to_plain_text())
            );
            if let Some(a) = action {
                s.push_str(&format!("  ({}: `{}`)", escape_md(&a.label), a.prefill));
            }
            Rendered::Priority(s)
        }
        OutputEvent::Request {
            kind,
            prompt,
            options,
            ..
        } => {
            let mut s = format!("❓ {}", escape_md(&prompt.to_plain_text()));
            match kind {
                RequestKind::Confirm => s.push_str("\nReply `respond yes` or `respond no`."),
                RequestKind::Choice => {
                    for (i, o) in options.iter().enumerate() {
                        s.push_str(&format!("\n  {}) {}", i + 1, escape_md(&o.label)));
                    }
                    s.push_str("\nReply `respond <number>`.");
                }
                RequestKind::Target | RequestKind::Text => {
                    s.push_str("\nReply `respond <your answer>`.");
                }
            }
            s.push_str(" `respond cancel` declines.");
            Rendered::Priority(s)
        }
        OutputEvent::Panel { feed, data, .. } if feed == "scenes" => {
            if data.is_null() {
                let was = state.in_scene.swap(false, Ordering::Relaxed);
                if was {
                    Rendered::Line("You are no longer in a scene.".to_string())
                } else {
                    Rendered::Drop
                }
            } else {
                state.in_scene.store(true, Ordering::Relaxed);
                let id = data
                    .get("id")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("");
                let status = data.get("status").and_then(|v| v.as_str()).unwrap_or("?");
                let round = data
                    .get("round")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                let turn = data
                    .get("current_actor")
                    .and_then(|a| a.get("name"))
                    .and_then(|n| n.as_str());
                let mut s = format!("Scene #{id}");
                if !title.is_empty() {
                    s.push_str(&format!(" “{}”", escape_md(title)));
                }
                s.push_str(&format!(" — {status}, round {round}"));
                if let Some(who) = turn {
                    s.push_str(&format!("; {}'s turn", escape_md(who)));
                }
                s.push('.');
                Rendered::Line(s)
            }
        }
        OutputEvent::Weather { text, .. } if !quiet => {
            Rendered::Line(escape_md(&text.to_plain_text()))
        }
        OutputEvent::SessionInfo {
            character,
            connections,
            ..
        } => Rendered::Line(format!(
            "Following **{}** ({connections} connection{}). Lines you send here run as them; `/detach` or `QUIT` stops.",
            escape_md(&character.name),
            if *connections == 1 { "" } else { "s" }
        )),
        OutputEvent::Disconnect { reason } => Rendered::Priority(format!(
            "Disconnected: {}",
            escape_md(&reason.to_plain_text())
        )),
        _ => Rendered::Drop,
    }
}

/// Split ordered lines into messages of at most `max_chars`, breaking only
/// between lines except for a single over-long line, which is hard-split.
#[must_use]
pub fn chunk(lines: &[String], max_chars: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut cur_len = 0usize;
    for line in lines {
        let len = line.chars().count();
        if len > max_chars {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
                cur_len = 0;
            }
            let chars: Vec<char> = line.chars().collect();
            for piece in chars.chunks(max_chars) {
                out.push(piece.iter().collect());
            }
            continue;
        }
        let sep = usize::from(!cur.is_empty());
        if cur_len + sep + len > max_chars {
            out.push(std::mem::take(&mut cur));
            cur_len = 0;
        }
        if !cur.is_empty() {
            cur.push('\n');
            cur_len += 1;
        }
        cur.push_str(line);
        cur_len += len;
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// The writer's buffers between flushes.
#[derive(Debug, Default)]
struct Buffers {
    /// Notices and requests, in order — never dropped.
    priority: VecDeque<String>,
    /// Narrative-class lines, in order — bounded; the oldest go first.
    pending: VecDeque<String>,
    /// Lines dropped since the last marker.
    dropped: usize,
}

impl Buffers {
    fn push(&mut self, rendered: Rendered, queue_lines: usize) -> bool {
        match rendered {
            Rendered::Drop => false,
            Rendered::Line(s) => {
                while self.pending.len() >= queue_lines {
                    self.pending.pop_front();
                    self.dropped += 1;
                }
                self.pending.push_back(s);
                true
            }
            Rendered::Priority(s) => {
                self.priority.push_back(s);
                true
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.priority.is_empty() && self.pending.is_empty() && self.dropped == 0
    }
}

/// Outcome of one flush attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flush {
    /// Everything buffered went out (or was permanently dropped).
    Done,
    /// A retryable failure; the unsent lines are back in the buffers.
    Retry,
}

/// Send everything buffered: priority lines first, then the drop marker,
/// then the narrative, each group chunked to the policy. A retryable failure
/// stops the flush and puts the unsent lines back **in order**; a permanent
/// one drops that chunk and continues; a closed DM drops everything and
/// marks the follow deaf.
async fn flush(
    api: &dyn DiscordApi,
    channel_id: u64,
    policy: &DmPolicy,
    buffers: &mut Buffers,
    state: &FollowState,
) -> Flush {
    if buffers.dropped > 0 {
        let n = buffers.dropped;
        buffers.dropped = 0;
        buffers.pending.push_front(format!(
            "… {n} line{} dropped (the DM could not keep up; your terminal's backlog has them) …",
            if n == 1 { "" } else { "s" }
        ));
    }
    // Two queues, one loop: priority first.
    for is_priority in [true, false] {
        let queue = if is_priority {
            &mut buffers.priority
        } else {
            &mut buffers.pending
        };
        while !queue.is_empty() {
            // Take as many whole lines as fit one message.
            let mut lines: Vec<String> = Vec::new();
            let mut len = 0usize;
            while let Some(front) = queue.front() {
                let l = front.chars().count();
                if !lines.is_empty() && len + 1 + l > policy.max_chars {
                    break;
                }
                len += l + usize::from(!lines.is_empty());
                lines.push(queue.pop_front().expect("front exists"));
                if l > policy.max_chars {
                    break;
                }
            }
            let messages = chunk(&lines, policy.max_chars);
            for (i, text) in messages.iter().enumerate() {
                match api.send(channel_id, Outgoing::plain(text.clone())).await {
                    Ok(_) => {
                        tracing::debug!(
                            priority = is_priority,
                            chars = text.chars().count(),
                            "discord dm: message sent"
                        );
                    }
                    Err(ApiError::Retryable(why)) => {
                        tracing::debug!(%why, "discord dm: send deferred");
                        // Put back what did not go, this chunk included, in order.
                        // (A chunk boundary inside a hard-split line loses the
                        // split's sent prefix on retry — a lossy surface's
                        // accepted duplicate.)
                        let mut unsent: Vec<String> = messages[i..].to_vec();
                        while let Some(m) = unsent.pop() {
                            queue.push_front(m);
                        }
                        return Flush::Retry;
                    }
                    Err(ApiError::CannotDm) => {
                        if !state.dm_closed.swap(true, Ordering::Relaxed) {
                            tracing::warn!("discord dm: the user's DMs are closed; follow is deaf");
                        }
                        buffers.priority.clear();
                        buffers.pending.clear();
                        return Flush::Done;
                    }
                    Err(ApiError::Permanent(why)) => {
                        tracing::warn!(%why, "discord dm: message dropped");
                    }
                }
            }
        }
    }
    Flush::Done
}

/// Run one follow's DM writer until the session drops its handle (the
/// channel closes) — then flush what is left and return.
pub async fn run_dm_writer(
    api: Arc<dyn DiscordApi>,
    channel_id: u64,
    mut rx: mpsc::Receiver<Arc<Envelope>>,
    policy: DmPolicy,
    state: Arc<FollowState>,
) {
    let mut buffers = Buffers::default();
    let mut deadline: Option<Instant> = None;
    loop {
        let wait = async {
            match deadline {
                Some(d) => tokio::time::sleep_until(d).await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            ev = rx.recv() => match ev {
                None => {
                    // Drain first, then a bounded final flush: one attempt,
                    // no retry loop on a closing follow.
                    if !buffers.is_empty() {
                        let _ = flush(&*api, channel_id, &policy, &mut buffers, &state).await;
                    }
                    return;
                }
                Some(env) => {
                    let rendered = render(&env.event, &state);
                    tracing::trace!(seq = env.seq, kept = !matches!(rendered, Rendered::Drop), "discord dm: event");
                    let priority = matches!(rendered, Rendered::Priority(_));
                    if buffers.push(rendered, policy.queue_lines) {
                        let now = Instant::now();
                        deadline = Some(match deadline {
                            // A priority line flushes as soon as we return to
                            // the loop; a narrative line waits out the window.
                            _ if priority => now,
                            Some(d) => d,
                            None => now + policy.coalesce,
                        });
                    }
                }
            },
            () = wait => {
                deadline = match flush(&*api, channel_id, &policy, &mut buffers, &state).await {
                    Flush::Done => None,
                    Flush::Retry => Some(Instant::now() + RETRY_BACKOFF),
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::testing::{Call, FakeApi};
    use mush_proto::{NoticeAction, ObjRef, PanelFrameKind, RequestOption, RoomView, SmolStr};

    fn env(event: OutputEvent) -> Arc<Envelope> {
        Arc::new(Envelope::new(1, 0, event))
    }

    fn say(who: &str, what: &str) -> OutputEvent {
        OutputEvent::Narrative {
            kind: NarrativeKind::Say,
            actor: Some(ObjRef::new("#7", who)),
            text: RichText::plain(what),
        }
    }

    /// Let the writer task run until it pends (it needs a few polls: drain
    /// the channel, arm or fire the timer, flush).
    async fn settle() {
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
    }

    #[test]
    fn the_allow_list_renders_what_a_follow_shows_and_drops_the_rest() {
        let state = FollowState::new(false, "#1");
        assert_eq!(
            render(&say("Nick", "hi *all*"), &state),
            Rendered::Line("Nick says, \"hi \\*all\\*\"".into())
        );
        // The followed character's own speech is second person, like the terminal.
        let own = FollowState::new(false, "#7");
        assert_eq!(
            render(&say("Nick", "hi"), &own),
            Rendered::Line("You say, \"hi\"".into())
        );
        assert_eq!(
            render(
                &OutputEvent::Narrative {
                    kind: NarrativeKind::Whisper,
                    actor: Some(ObjRef::new("#7", "Nick")),
                    text: RichText::plain("psst"),
                },
                &own
            ),
            Rendered::Line("You whisper, \"psst\"".into())
        );
        assert!(
            matches!(
                render(
                    &OutputEvent::Narrative {
                        kind: NarrativeKind::Pose,
                        actor: Some(ObjRef::new("#7", "Nick")),
                        text: RichText::plain("waves."),
                    },
                    &own
                ),
                Rendered::Line(s) if s == "Nick waves."
            ),
            "poses stay third person, as at the terminal"
        );
        assert_eq!(
            render(
                &OutputEvent::Narrative {
                    kind: NarrativeKind::Pose,
                    actor: Some(ObjRef::new("#7", "Nick")),
                    text: RichText::plain("waves."),
                },
                &state
            ),
            Rendered::Line("Nick waves.".into())
        );
        assert_eq!(
            render(
                &OutputEvent::Sys(RichText::tag("scene", "Time stops.")),
                &state
            ),
            Rendered::Line("Time stops.".into())
        );
        assert_eq!(
            render(&OutputEvent::Sys(RichText::tag("weather", "Rain.")), &state),
            Rendered::Drop,
            "an unlisted Sys tag is dropped"
        );
        let notice = OutputEvent::Notice {
            id: 1,
            severity: NoticeSeverity::Urgent,
            category: SmolStr::new("page"),
            text: RichText::plain("Wiz pages: come back"),
            action: Some(NoticeAction {
                label: "Reply".into(),
                prefill: "page Wiz=".into(),
            }),
            ts: 0,
        };
        assert_eq!(
            render(&notice, &state),
            Rendered::Priority("‼ [page] Wiz pages: come back  (Reply: `page Wiz=`)".into())
        );
        let request = OutputEvent::Request {
            id: 9,
            kind: RequestKind::Choice,
            prompt: RichText::plain("End the scene?"),
            options: vec![
                RequestOption {
                    id: SmolStr::new("yes"),
                    label: "Yes".into(),
                },
                RequestOption {
                    id: SmolStr::new("no"),
                    label: "No".into(),
                },
            ],
            deadline_ms: None,
        };
        let Rendered::Priority(text) = render(&request, &state) else {
            panic!("a request is priority");
        };
        assert!(
            text.contains("1) Yes") && text.contains("2) No") && text.contains("respond <number>")
        );
        // Dropped kinds.
        for ev in [
            OutputEvent::Who(vec![]),
            OutputEvent::Ping { nonce: 1 },
            OutputEvent::Composing {
                actor: ObjRef::new("#7", "Nick"),
                active: true,
            },
            OutputEvent::Channel {
                channel: SmolStr::new("pub"),
                seq: 1,
                actor: ObjRef::new("#7", "Nick"),
                title: None,
                kind: NarrativeKind::Say,
                text: RichText::plain("hi"),
                ts: 0,
            },
        ] {
            assert_eq!(render(&ev, &state), Rendered::Drop, "{ev:?}");
        }
        // A room render is compact.
        let room = OutputEvent::Room(RoomView {
            name: "Tap Room".into(),
            id: "#3".into(),
            desc: RichText::plain("x".repeat(400)),
            contents: vec![ObjRef::new("#7", "Nick")],
            exits: vec![mush_proto::ExitRef {
                name: "out".into(),
                aliases: vec![],
            }],
            zone: None,
            ambient: Default::default(),
        });
        let Rendered::Line(text) = render(&room, &state) else {
            panic!("a room is a line");
        };
        assert!(text.starts_with("**Tap Room**"));
        assert!(text.contains("…"), "long descriptions are cut");
        assert!(text.contains("Here: Nick") && text.contains("Exits: out"));
    }

    #[test]
    fn quiet_keeps_notices_requests_and_the_scene_only() {
        let state = FollowState::new(true, "#1");
        assert_eq!(render(&say("Nick", "hi"), &state), Rendered::Drop);
        assert_eq!(
            render(&OutputEvent::Sys(RichText::tag("error", "Huh?")), &state),
            Rendered::Drop
        );
        // A scene keyframe flips the state; narrative then flows.
        let keyframe = OutputEvent::Panel {
            feed: SmolStr::new("scenes"),
            kind: PanelFrameKind::Keyframe,
            data: serde_json::json!({
                "id": 5, "title": "Bar fight", "status": "active", "round": 3,
                "current_actor": { "id": "#7", "name": "Nick" }
            }),
        };
        assert_eq!(
            render(&keyframe, &state),
            Rendered::Line("Scene #5 “Bar fight” — active, round 3; Nick's turn.".into())
        );
        assert!(state.in_scene.load(Ordering::Relaxed));
        assert!(matches!(
            render(&say("Nick", "hi"), &state),
            Rendered::Line(_)
        ));
        let gone = OutputEvent::Panel {
            feed: SmolStr::new("scenes"),
            kind: PanelFrameKind::Keyframe,
            data: serde_json::Value::Null,
        };
        assert_eq!(
            render(&gone, &state),
            Rendered::Line("You are no longer in a scene.".into())
        );
        assert!(!state.in_scene.load(Ordering::Relaxed));
        assert_eq!(
            render(&gone, &state),
            Rendered::Drop,
            "only the transition is a line"
        );
    }

    #[test]
    fn chunking_breaks_between_lines_and_hard_splits_one_long_line() {
        let lines: Vec<String> = vec!["a".repeat(10), "b".repeat(10), "c".repeat(25), "d".into()];
        let out = chunk(&lines, 21);
        assert_eq!(out[0], format!("{}\n{}", "a".repeat(10), "b".repeat(10)));
        assert_eq!(out[1], "c".repeat(21));
        assert_eq!(out[2], "c".repeat(4));
        assert_eq!(out[3], "d");
        assert!(out.iter().all(|m| m.chars().count() <= 21));
    }

    #[tokio::test(start_paused = true)]
    async fn the_writer_coalesces_flushes_priority_promptly_and_marks_drops() {
        let api = FakeApi::new();
        let state = FollowState::new(false, "#1");
        let (tx, rx) = mpsc::channel::<Arc<Envelope>>(256);
        let policy = DmPolicy::new(1_500, 1_900, 10);
        let writer = tokio::spawn(run_dm_writer(api.clone(), 555, rx, policy, state.clone()));
        // Twelve narrative lines into a ten-line queue: two dropped, a marker.
        for i in 0..12 {
            tx.send(env(say("Nick", &format!("line {i}"))))
                .await
                .unwrap();
        }
        // The writer drains and arms the window; nothing goes out before it.
        settle().await;
        assert!(api.calls().is_empty(), "nothing is sent inside the window");
        tokio::time::advance(Duration::from_millis(1_600)).await;
        settle().await;
        let sends: Vec<String> = api
            .calls()
            .into_iter()
            .filter_map(|c| match c {
                Call::Send { channel_id, msg } => {
                    assert_eq!(channel_id, 555);
                    Some(msg.content)
                }
                _ => None,
            })
            .collect();
        assert_eq!(sends.len(), 1, "one coalesced message: {sends:?}");
        assert!(sends[0].starts_with("… 2 lines dropped"));
        assert!(sends[0].contains("line 2") && sends[0].contains("line 11"));
        assert!(!sends[0].contains("\"line 1\"") && !sends[0].contains("\"line 0\""));
        // A notice goes out on the next loop turn, ahead of buffered narrative.
        tx.send(env(say("Nick", "later"))).await.unwrap();
        tx.send(env(OutputEvent::Notice {
            id: 2,
            severity: NoticeSeverity::Info,
            category: SmolStr::new("mail"),
            text: RichText::plain("New mail"),
            action: None,
            ts: 0,
        }))
        .await
        .unwrap();
        // A priority line flushes on the writer's next turn, no window.
        settle().await;
        let sends: Vec<String> = api
            .calls()
            .into_iter()
            .filter_map(|c| match c {
                Call::Send { msg, .. } => Some(msg.content),
                _ => None,
            })
            .collect();
        assert_eq!(sends.len(), 3, "{sends:?}");
        assert!(
            sends[1].starts_with("• [mail] New mail"),
            "priority first: {sends:?}"
        );
        assert!(sends[2].contains("later"));
        drop(tx);
        writer.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn a_retryable_failure_keeps_order_and_a_closed_dm_goes_deaf() {
        let api = FakeApi::new();
        let state = FollowState::new(false, "#1");
        let (tx, rx) = mpsc::channel::<Arc<Envelope>>(256);
        let policy = DmPolicy::new(100, 1_900, 100);
        let writer = tokio::spawn(run_dm_writer(api.clone(), 555, rx, policy, state.clone()));
        api.script([Some(ApiError::Retryable("429".into()))]);
        tx.send(env(say("Nick", "first"))).await.unwrap();
        settle().await;
        tokio::time::advance(Duration::from_millis(150)).await;
        settle().await; // the flush attempt fails; the line is back in order
        tx.send(env(say("Nick", "second"))).await.unwrap();
        settle().await;
        tokio::time::advance(RETRY_BACKOFF + Duration::from_millis(50)).await;
        settle().await;
        let sends: Vec<String> = api
            .calls()
            .into_iter()
            .filter_map(|c| match c {
                Call::Send { msg, .. } => Some(msg.content),
                _ => None,
            })
            .collect();
        // The first attempt failed; the retry carried both lines, in order.
        assert_eq!(sends.len(), 2, "{sends:?}");
        assert!(
            sends[1].contains("\"first\"\n") && sends[1].ends_with("\"second\""),
            "{sends:?}"
        );
        // Closed DMs: the follow goes deaf and buffers are dropped.
        api.script([Some(ApiError::CannotDm)]);
        tx.send(env(say("Nick", "unheard"))).await.unwrap();
        settle().await;
        tokio::time::advance(Duration::from_millis(150)).await;
        settle().await;
        assert!(state.dm_closed.load(Ordering::Relaxed));
        drop(tx);
        writer.await.unwrap();
    }
}
