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
//! D3 adds the **interactive** render: a request becomes its own message with
//! its answers as buttons or a select ([`request_rows`]), a notice with an
//! action gets a reply button, and the writer remembers which message shows
//! which request so the bridge can mark it answered and the writer can mark
//! it expired at the deadline ([`FollowState::take_request`]).
//!
//! The one invariant the writer must keep is the session's slow-client
//! trip-wire: it **drains its channel eagerly** into its own bounded buffer
//! and never awaits Discord while the channel could fill.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mush_proto::{
    Envelope, NarrativeKind, NoticeSeverity, OutputEvent, RequestKind, RequestOption, RichText,
};
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::api::{
    ApiError, Button, ButtonStyle, DiscordApi, MAX_BUTTONS_PER_ROW, MAX_CUSTOM_ID_BYTES, MAX_ROWS,
    MAX_SELECT_OPTIONS, Outgoing, Row, Select,
};

/// How long a failed (retryable) send waits before the next attempt.
pub const RETRY_BACKOFF: Duration = Duration::from_secs(5);

/// Longest room description the compact `look` render keeps, characters.
pub const ROOM_DESC_CHARS: usize = 300;

/// The most notice reply buttons a follow remembers the prefill for.
pub const MAX_NOTICE_ACTIONS: usize = 64;

/// A choice with at most this many options is buttons (rows of five); more
/// become selects. Leaves the fifth row for Decline.
pub const MAX_CHOICE_BUTTONS: usize = 20;

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

/// Where a request is shown in the DM (D3): the message carrying its
/// buttons, and the text to keep when the buttons are replaced by the answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestMessage {
    /// Engine lifetime echoed with the response.
    pub epoch: String,
    /// The DM channel.
    pub channel_id: u64,
    /// The message.
    pub message_id: u64,
    /// The prompt as rendered.
    pub text: String,
    /// The answer components as sent, so an answer can be named by its label.
    pub rows: Vec<Row>,
}

impl RequestMessage {
    /// The label of the button `custom_id` or the select option `value` the
    /// player picked, for the "answered" edit.
    #[must_use]
    pub fn label_for(&self, custom_id: &str, value: Option<&str>) -> Option<String> {
        for row in &self.rows {
            match row {
                Row::Buttons(buttons) => {
                    if let Some(b) = buttons.iter().find(|b| b.custom_id == custom_id) {
                        return Some(b.label.clone());
                    }
                }
                Row::Select(select) if select.custom_id == custom_id => {
                    if let Some((_, label)) = select
                        .options
                        .iter()
                        .find(|(v, _)| Some(v.as_str()) == value)
                    {
                        return Some(label.clone());
                    }
                }
                Row::Select(_) => {}
            }
        }
        None
    }
}

/// The notice reply buttons offered, newest first, bounded.
#[derive(Debug, Default)]
struct NoticeActions {
    order: VecDeque<u64>,
    by_nonce: HashMap<u64, (String, String)>,
}

/// What the writer has learned about the follow, shared with the bridge:
/// whether the character is in a scene (the idle sweeper never detaches a
/// scene member), the quiet preference, whether Discord closed the DM, and
/// (D3) which message shows which open request and what each notice reply
/// button prefills.
#[derive(Debug, Default)]
pub struct FollowState {
    /// Stops queued output and expiry edits when the follow is revoked.
    pub cancelled: tokio_util::sync::CancellationToken,
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
    /// Open requests shown with components: request id → the message.
    requests: Mutex<HashMap<u64, RequestMessage>>,
    /// Notice reply buttons: nonce → `(label, prefill)`.
    actions: Mutex<NoticeActions>,
    next_nonce: AtomicU64,
}

impl FollowState {
    /// Discard controls from a previous engine lifetime.
    pub fn clear_requests(&self) {
        self.requests.lock().expect("requests poisoned").clear();
    }

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

    /// Remember that request `id` is shown by `message` (the writer, once the
    /// send succeeded).
    pub fn request_shown(&self, id: u64, message: RequestMessage) {
        self.requests
            .lock()
            .expect("requests poisoned")
            .insert(id, message);
    }

    /// Where request `id` is shown, if it is still open here.
    #[must_use]
    pub fn request(&self, id: u64) -> Option<RequestMessage> {
        self.requests
            .lock()
            .expect("requests poisoned")
            .get(&id)
            .cloned()
    }

    /// Forget request `id`, returning where it was shown: the bridge on an
    /// answer, the writer at the deadline. `None` means the other side got
    /// there first (already answered, or already expired).
    pub fn take_request(&self, id: u64) -> Option<RequestMessage> {
        self.requests.lock().expect("requests poisoned").remove(&id)
    }

    /// Forget only the originating message, never a replacement received
    /// while forwarding its predecessor's answer to the world.
    pub fn take_request_message(&self, id: u64, message_id: u64) -> Option<RequestMessage> {
        let mut requests = self.requests.lock().expect("requests poisoned");
        if requests
            .get(&id)
            .is_some_and(|r| r.message_id == message_id)
        {
            requests.remove(&id)
        } else {
            None
        }
    }

    /// A notice reply button's `(label, prefill)` by its nonce.
    #[must_use]
    pub fn action(&self, nonce: u64) -> Option<(String, String)> {
        self.actions
            .lock()
            .expect("actions poisoned")
            .by_nonce
            .get(&nonce)
            .cloned()
    }

    /// Offer a notice reply button; returns its nonce. The oldest offers are
    /// forgotten past [`MAX_NOTICE_ACTIONS`] (their buttons then say so).
    pub fn offer_action(&self, label: &str, prefill: &str) -> u64 {
        let nonce = self.next_nonce.fetch_add(1, Ordering::Relaxed) + 1;
        let mut actions = self.actions.lock().expect("actions poisoned");
        while actions.order.len() >= MAX_NOTICE_ACTIONS {
            if let Some(old) = actions.order.pop_front() {
                actions.by_nonce.remove(&old);
            }
        }
        actions.order.push_back(nonce);
        actions
            .by_nonce
            .insert(nonce, (label.to_string(), prefill.to_string()));
        nonce
    }
}

/// A notice with a reply button or a request with its answers (D3): sent as
/// its own message, promptly, never dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interactive {
    /// The message text.
    pub text: String,
    /// Its action rows.
    pub rows: Vec<Row>,
    /// For a request, `(request id, deadline in Unix ms)` so the writer can
    /// record the message and expire it.
    pub request: Option<(u64, String, Option<u64>)>,
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
    /// A notice with a reply button, or a request with its answers.
    Interactive(Interactive),
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
        NarrativeKind::Emit => text.to_string(),
        NarrativeKind::Spoof => format!("[{name}] {text}"),
        NarrativeKind::Page => format!("{name} pages: {text}"),
        NarrativeKind::Whisper if own => format!("You whisper, \"{text}\""),
        NarrativeKind::Whisper => format!("{name} whispers, \"{text}\""),
        NarrativeKind::Ooc => match actor {
            Some(name) => format!("<OOC> {name} {text}"),
            None => format!("<OOC> {text}"),
        },
    }
}

/// The components for a request (D3). `Confirm` → Yes / No. `Choice` →
/// buttons in rows of five up to [`MAX_CHOICE_BUTTONS`] options, else one or
/// two selects; `Target` / `Text` → an "Answer…" button that opens a modal.
/// Every kind but `Confirm` also offers Decline (`respond cancel`). The
/// routing keys are `d:req:<id>:yes|no|cancel|text|o:<option>|s<n>`; an option
/// whose id would not fit a `custom_id` is left to the text path.
#[must_use]
pub fn request_rows(id: u64, kind: RequestKind, options: &[RequestOption]) -> Vec<Row> {
    let cid = |suffix: &str| format!("d:req:{id}:{suffix}");
    let decline = Button {
        custom_id: cid("cancel"),
        label: "Decline".into(),
        style: ButtonStyle::Secondary,
    };
    match kind {
        RequestKind::Confirm => vec![Row::Buttons(vec![
            Button {
                custom_id: cid("yes"),
                label: "Yes".into(),
                style: ButtonStyle::Success,
            },
            Button {
                custom_id: cid("no"),
                label: "No".into(),
                style: ButtonStyle::Danger,
            },
        ])],
        RequestKind::Choice => {
            let fits: Vec<&RequestOption> = options
                .iter()
                .filter(|o| cid(&format!("o:{}", o.id)).len() <= MAX_CUSTOM_ID_BYTES)
                .collect();
            let mut rows = Vec::new();
            if fits.len() <= MAX_CHOICE_BUTTONS {
                for chunk in fits.chunks(MAX_BUTTONS_PER_ROW) {
                    rows.push(Row::Buttons(
                        chunk
                            .iter()
                            .map(|o| Button {
                                custom_id: cid(&format!("o:{}", o.id)),
                                label: o.label.clone(),
                                style: ButtonStyle::Primary,
                            })
                            .collect(),
                    ));
                }
            } else {
                for (n, chunk) in fits
                    .chunks(MAX_SELECT_OPTIONS)
                    .take(MAX_ROWS - 1)
                    .enumerate()
                {
                    rows.push(Row::Select(Select {
                        custom_id: cid(&format!("s{n}")),
                        placeholder: if n == 0 {
                            "Choose…".to_string()
                        } else {
                            format!("More choices ({})", n + 1)
                        },
                        options: chunk
                            .iter()
                            .map(|o| (o.id.to_string(), o.label.clone()))
                            .collect(),
                    }));
                }
            }
            rows.push(Row::Buttons(vec![decline]));
            rows
        }
        RequestKind::Target | RequestKind::Text => vec![Row::Buttons(vec![
            Button {
                custom_id: cid("text"),
                label: "Answer…".into(),
                style: ButtonStyle::Primary,
            },
            decline,
        ])],
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
        OutputEvent::Hello { .. } => {
            state.clear_requests();
            Rendered::Drop
        }
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
            let s = format!(
                "{marker} [{}] {}",
                escape_md(category),
                escape_md(&text.to_plain_text())
            );
            if let Some(a) = action {
                // A reply button opens a modal prefilled with the action's
                // line; the prefill stays here, never in the custom_id.
                let nonce = state.offer_action(&a.label, &a.prefill);
                return Rendered::Interactive(Interactive {
                    text: s,
                    rows: vec![Row::Buttons(vec![Button {
                        custom_id: format!("d:act:{nonce}"),
                        label: a.label.clone(),
                        style: ButtonStyle::Primary,
                    }])],
                    request: None,
                });
            }
            Rendered::Priority(s)
        }
        OutputEvent::Request {
            epoch,
            id,
            kind,
            prompt,
            options,
            deadline_ms,
        } => Rendered::Interactive(Interactive {
            text: format!("❓ {}", escape_md(&prompt.to_plain_text())),
            rows: request_rows(*id, *kind, options),
            request: Some((*id, epoch.clone(), *deadline_ms)),
        }),
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

/// One buffered priority item.
#[derive(Debug)]
enum PriorityItem {
    /// A line; consecutive lines coalesce into one message.
    Text(String),
    /// Its own message, with components.
    Interactive(Interactive),
}

/// The writer's buffers between flushes.
#[derive(Debug, Default)]
struct Buffers {
    /// Notices and requests, in order — never dropped.
    priority: VecDeque<PriorityItem>,
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
                self.priority.push_back(PriorityItem::Text(s));
                true
            }
            Rendered::Interactive(i) => {
                self.priority.push_back(PriorityItem::Interactive(i));
                true
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.priority.is_empty() && self.pending.is_empty() && self.dropped == 0
    }

    fn clear(&mut self) {
        self.priority.clear();
        self.pending.clear();
        self.dropped = 0;
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

/// Outcome of sending one group of plain lines.
enum SendOutcome {
    /// Every chunk went (or was permanently dropped).
    Sent,
    /// A retryable failure: the chunks that did not go, in order.
    Retry(Vec<String>),
    /// The user's DMs are closed.
    Deaf,
}

/// Send `lines` as as many messages as the policy needs.
async fn send_lines(
    api: &dyn DiscordApi,
    channel_id: u64,
    policy: &DmPolicy,
    lines: &[String],
    priority: bool,
    state: &FollowState,
) -> SendOutcome {
    let messages = chunk(lines, policy.max_chars);
    for (i, text) in messages.iter().enumerate() {
        match api.send(channel_id, Outgoing::plain(text.clone())).await {
            Ok(_) => {
                tracing::debug!(
                    priority,
                    chars = text.chars().count(),
                    "discord dm: message sent"
                );
            }
            Err(ApiError::Retryable(why)) => {
                tracing::debug!(%why, "discord dm: send deferred");
                // Put back what did not go, this chunk included, in order.
                // (A chunk boundary inside a hard-split line loses the
                // split's sent prefix on retry — a lossy surface's accepted
                // duplicate.)
                return SendOutcome::Retry(messages[i..].to_vec());
            }
            Err(ApiError::CannotDm) => {
                if !state.dm_closed.swap(true, Ordering::Relaxed) {
                    tracing::warn!("discord dm: the user's DMs are closed; follow is deaf");
                }
                return SendOutcome::Deaf;
            }
            Err(ApiError::Permanent(why)) => {
                tracing::warn!(%why, "discord dm: message dropped");
            }
        }
    }
    SendOutcome::Sent
}

/// At `deadline_ms` (Unix), mark request `id` expired in the DM unless it
/// was answered first.
fn spawn_expiry(api: Arc<dyn DiscordApi>, state: Arc<FollowState>, id: u64, deadline_ms: u64) {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or(u64::MAX);
    let wait = Duration::from_millis(deadline_ms.saturating_sub(now_ms));
    let Some(expected) = state.request(id) else {
        return;
    };
    tokio::spawn(async move {
        tokio::select! {
            () = state.cancelled.cancelled() => return,
            () = tokio::time::sleep(wait) => {}
        }
        let shown = {
            let mut requests = state.requests.lock().expect("requests poisoned");
            if requests
                .get(&id)
                .is_some_and(|r| r.message_id == expected.message_id && r.epoch == expected.epoch)
            {
                requests.remove(&id)
            } else {
                None
            }
        };
        if let Some(shown) = shown {
            let editing = api.edit(
                shown.channel_id,
                shown.message_id,
                Outgoing::plain(format!(
                    "{}\n⌛ Expired — the game applied its default.",
                    shown.text
                )),
            );
            tokio::select! { () = state.cancelled.cancelled() => {}, _ = editing => {} }
        }
    });
}

/// Send everything buffered: priority items first (text runs coalesced,
/// interactive items as their own messages), then the drop marker, then the
/// narrative. A retryable failure stops the flush and puts the unsent items
/// back **in order**; a permanent one drops that chunk and continues (an
/// interactive item falls back to plain text, so the request is still
/// answerable by `respond`); a closed DM drops everything and marks the
/// follow deaf.
async fn flush(
    api: &Arc<dyn DiscordApi>,
    channel_id: u64,
    policy: &DmPolicy,
    buffers: &mut Buffers,
    state: &Arc<FollowState>,
) -> Flush {
    if buffers.dropped > 0 {
        let n = buffers.dropped;
        buffers.dropped = 0;
        buffers.pending.push_front(format!(
            "… {n} line{} dropped (the DM could not keep up; your terminal's backlog has them) …",
            if n == 1 { "" } else { "s" }
        ));
    }
    while let Some(item) = buffers.priority.pop_front() {
        match item {
            PriorityItem::Text(first) => {
                let mut len = first.chars().count();
                let mut lines = vec![first];
                while let Some(PriorityItem::Text(next)) = buffers.priority.front() {
                    let l = next.chars().count();
                    if len + 1 + l > policy.max_chars {
                        break;
                    }
                    len += 1 + l;
                    if let Some(PriorityItem::Text(next)) = buffers.priority.pop_front() {
                        lines.push(next);
                    }
                }
                match send_lines(&**api, channel_id, policy, &lines, true, state).await {
                    SendOutcome::Sent => {}
                    SendOutcome::Retry(unsent) => {
                        for m in unsent.into_iter().rev() {
                            buffers.priority.push_front(PriorityItem::Text(m));
                        }
                        return Flush::Retry;
                    }
                    SendOutcome::Deaf => {
                        buffers.clear();
                        return Flush::Done;
                    }
                }
            }
            PriorityItem::Interactive(item) => {
                let msg = Outgoing::plain(item.text.clone()).with_components(item.rows.clone());
                match api.send(channel_id, msg).await {
                    Ok(message_id) => {
                        tracing::debug!(
                            rows = item.rows.len(),
                            request = item.request.as_ref().map(|(id, _, _)| id),
                            "discord dm: interactive message sent"
                        );
                        if let Some((id, epoch, deadline)) = item.request {
                            state.request_shown(
                                id,
                                RequestMessage {
                                    epoch,
                                    channel_id,
                                    message_id,
                                    text: item.text.clone(),
                                    rows: item.rows.clone(),
                                },
                            );
                            if let Some(deadline_ms) = deadline {
                                spawn_expiry(Arc::clone(api), Arc::clone(state), id, deadline_ms);
                            }
                        }
                    }
                    Err(ApiError::Retryable(why)) => {
                        tracing::debug!(%why, "discord dm: interactive send deferred");
                        buffers.priority.push_front(PriorityItem::Interactive(item));
                        return Flush::Retry;
                    }
                    Err(ApiError::CannotDm) => {
                        if !state.dm_closed.swap(true, Ordering::Relaxed) {
                            tracing::warn!("discord dm: the user's DMs are closed; follow is deaf");
                        }
                        buffers.clear();
                        return Flush::Done;
                    }
                    Err(ApiError::Permanent(why)) => {
                        // The components were refused; the text path still
                        // answers it.
                        tracing::warn!(%why, "discord dm: components refused; sending plain text");
                        let plain = format!("{}\n(reply with `respond …`)", item.text);
                        let _ = api.send(channel_id, Outgoing::plain(plain)).await;
                    }
                }
            }
        }
    }
    if !buffers.pending.is_empty() {
        let lines: Vec<String> = buffers.pending.drain(..).collect();
        match send_lines(&**api, channel_id, policy, &lines, false, state).await {
            SendOutcome::Sent => {}
            SendOutcome::Retry(unsent) => {
                for m in unsent.into_iter().rev() {
                    buffers.pending.push_front(m);
                }
                return Flush::Retry;
            }
            SendOutcome::Deaf => {
                buffers.clear();
                return Flush::Done;
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
    // A flush owns only one bounded batch. Keep draining the session while
    // REST waits, so narrative pressure is handled here instead of upstream.
    const PRIORITY_CAP: usize = 128;
    let mut buffers = Buffers::default();
    let mut deadline: Option<Instant> = None;
    let mut retrying = false;
    let mut closing: Option<Instant> = None;
    loop {
        if state.cancelled.is_cancelled() || state.dm_closed.load(Ordering::Relaxed) {
            return;
        }
        if closing.is_some() && buffers.is_empty() {
            return;
        }
        let wait = async {
            match deadline {
                Some(d) => tokio::time::sleep_until(d).await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            biased;
            () = state.cancelled.cancelled() => return,
            () = async { match closing { Some(d) => tokio::time::sleep_until(d).await, None => std::future::pending().await } } => return,
            () = wait => {
                let mut batch = Buffers::default();
                if let Some(item) = buffers.priority.pop_front() {
                    batch.priority.push_back(item);
                } else {
                    batch.dropped = std::mem::take(&mut buffers.dropped);
                    let mut chars = 0;
                    while let Some(line) = buffers.pending.front() {
                        let size = line.chars().count() + 1;
                        if chars > 0 && chars + size > policy.max_chars { break; }
                        chars += size;
                        batch.pending.push_back(buffers.pending.pop_front().expect("front exists"));
                    }
                }
                let result;
                {
                    let sending = flush(&api, channel_id, &policy, &mut batch, &state);
                    tokio::pin!(sending);
                    loop {
                        tokio::select! {
                            biased;
                            () = state.cancelled.cancelled() => return,
                            () = async { match closing { Some(d) => tokio::time::sleep_until(d).await, None => std::future::pending().await } } => return,
                            done = &mut sending => { result = done; break; },
                            ev = rx.recv(), if closing.is_none() => match ev {
                                Some(env) => {
                                    buffers.push(render(&env.event, &state), policy.queue_lines);
                                    if buffers.priority.len() >= PRIORITY_CAP {
                                        tracing::warn!("discord dm: priority queue exhausted; detaching");
                                        state.dm_closed.store(true, Ordering::Relaxed);
                                        return;
                                    }
                                }
                                None => closing = Some(Instant::now() + Duration::from_secs(5)),
                            }
                        }
                    }
                }
                // Unsent old work precedes new work. Narrative keeps the
                // newest lines within the same cap even after a retry.
                batch.priority.append(&mut buffers.priority);
                batch.pending.append(&mut buffers.pending);
                batch.dropped += buffers.dropped;
                while batch.pending.len() > policy.queue_lines {
                    batch.pending.pop_front();
                    batch.dropped += 1;
                }
                if batch.priority.len() >= PRIORITY_CAP {
                    state.dm_closed.store(true, Ordering::Relaxed);
                    return;
                }
                buffers = batch;
                retrying = result == Flush::Retry;
                deadline = if retrying { Some(Instant::now() + RETRY_BACKOFF) }
                    else if !buffers.is_empty() { Some(Instant::now()) } else { None };
            },
            ev = rx.recv(), if closing.is_none() => match ev {
                None => {
                    closing = Some(Instant::now() + Duration::from_secs(5));
                    if !retrying { deadline = Some(Instant::now()); }
                }
                Some(env) => {
                    let rendered = render(&env.event, &state);
                    let priority = matches!(rendered, Rendered::Priority(_) | Rendered::Interactive(_));
                    if buffers.push(rendered, policy.queue_lines) {
                        if buffers.priority.len() >= PRIORITY_CAP {
                            tracing::warn!("discord dm: priority queue exhausted; detaching");
                            state.dm_closed.store(true, Ordering::Relaxed);
                            return;
                        }
                        if !retrying {
                            deadline = Some(match deadline {
                                _ if priority => Instant::now(),
                                Some(d) => d,
                                None => Instant::now() + policy.coalesce,
                            });
                        }
                    }
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::testing::{Call, FakeApi};
    use mush_proto::{NoticeAction, ObjRef, PanelFrameKind, RoomView, SmolStr};

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

    fn option(id: &str, label: &str) -> RequestOption {
        RequestOption {
            id: SmolStr::new(id),
            label: label.into(),
        }
    }

    fn notice(text: &str) -> OutputEvent {
        OutputEvent::Notice {
            id: 1,
            severity: NoticeSeverity::Info,
            category: "test".into(),
            text: RichText::plain(text),
            action: None,
            ts: 0,
        }
    }

    fn sends(api: &FakeApi) -> Vec<Outgoing> {
        api.calls()
            .into_iter()
            .filter_map(|c| match c {
                Call::Send { msg, .. } => Some(msg),
                _ => None,
            })
            .collect()
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
        // A notice without an action is a priority line; with one it carries
        // a reply button whose prefill stays server-side of Discord.
        let plain_notice = OutputEvent::Notice {
            id: 1,
            severity: NoticeSeverity::Info,
            category: SmolStr::new("mail"),
            text: RichText::plain("New mail"),
            action: None,
            ts: 0,
        };
        assert_eq!(
            render(&plain_notice, &state),
            Rendered::Priority("• [mail] New mail".into())
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
        let Rendered::Interactive(i) = render(&notice, &state) else {
            panic!("a notice with an action is interactive");
        };
        assert_eq!(i.text, "‼ [page] Wiz pages: come back");
        assert_eq!(i.request, None);
        let Row::Buttons(buttons) = &i.rows[0] else {
            panic!("one button row");
        };
        assert_eq!(buttons.len(), 1);
        assert_eq!(buttons[0].label, "Reply");
        let nonce: u64 = buttons[0]
            .custom_id
            .strip_prefix("d:act:")
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            state.action(nonce),
            Some(("Reply".to_string(), "page Wiz=".to_string()))
        );
        assert!(
            !buttons[0].custom_id.contains("page"),
            "the prefill never rides the custom_id"
        );
        // A request is interactive: prompt text, answer components, and the
        // id/deadline the writer needs.
        let request = OutputEvent::Request {
            epoch: "test".into(),
            id: 9,
            kind: RequestKind::Choice,
            prompt: RichText::plain("End the scene?"),
            options: vec![option("yes", "Yes"), option("no", "No")],
            deadline_ms: Some(1_000),
        };
        let Rendered::Interactive(i) = render(&request, &state) else {
            panic!("a request is interactive");
        };
        assert_eq!(i.text, "❓ End the scene?");
        assert_eq!(i.request, Some((9, "test".into(), Some(1_000))));
        assert_eq!(i.rows.len(), 2, "a button row and the Decline row");
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
    fn request_rows_follow_the_layout_rules() {
        // Confirm: Yes / No, nothing else.
        let rows = request_rows(4, RequestKind::Confirm, &[]);
        assert_eq!(rows.len(), 1);
        let Row::Buttons(b) = &rows[0] else { panic!() };
        assert_eq!(
            b.iter()
                .map(|x| (x.custom_id.as_str(), x.style))
                .collect::<Vec<_>>(),
            vec![
                ("d:req:4:yes", ButtonStyle::Success),
                ("d:req:4:no", ButtonStyle::Danger)
            ]
        );
        // Choice with twelve options: rows of five, then Decline.
        let twelve: Vec<RequestOption> = (0..12)
            .map(|i| option(&format!("o{i}"), &format!("Option {i}")))
            .collect();
        let rows = request_rows(5, RequestKind::Choice, &twelve);
        assert_eq!(rows.len(), 4);
        let sizes: Vec<usize> = rows
            .iter()
            .map(|r| match r {
                Row::Buttons(b) => b.len(),
                Row::Select(_) => usize::MAX,
            })
            .collect();
        assert_eq!(sizes, vec![5, 5, 2, 1]);
        let Row::Buttons(first) = &rows[0] else {
            panic!()
        };
        assert_eq!(first[0].custom_id, "d:req:5:o:o0");
        let Row::Buttons(last) = &rows[3] else {
            panic!()
        };
        assert_eq!(last[0].custom_id, "d:req:5:cancel");
        // Thirty options: two selects (25 + 5), then Decline.
        let thirty: Vec<RequestOption> = (0..30)
            .map(|i| option(&format!("o{i}"), &format!("Option {i}")))
            .collect();
        let rows = request_rows(6, RequestKind::Choice, &thirty);
        assert_eq!(rows.len(), 3);
        let Row::Select(s0) = &rows[0] else { panic!() };
        let Row::Select(s1) = &rows[1] else { panic!() };
        assert_eq!((s0.options.len(), s1.options.len()), (25, 5));
        assert_eq!(
            (s0.custom_id.as_str(), s1.custom_id.as_str()),
            ("d:req:6:s0", "d:req:6:s1")
        );
        assert_eq!(s1.options[0].0, "o25");
        // An option id too long for a custom_id is left to the text path.
        let long = option(&"x".repeat(120), "Long");
        let rows = request_rows(7, RequestKind::Choice, &[option("a", "A"), long]);
        let Row::Buttons(b) = &rows[0] else { panic!() };
        assert_eq!(b.len(), 1);
        // Text and Target: Answer… opens a modal, plus Decline.
        for kind in [RequestKind::Text, RequestKind::Target] {
            let rows = request_rows(8, kind, &[]);
            let Row::Buttons(b) = &rows[0] else { panic!() };
            assert_eq!(
                b.iter().map(|x| x.custom_id.as_str()).collect::<Vec<_>>(),
                vec!["d:req:8:text", "d:req:8:cancel"]
            );
        }
    }

    #[test]
    fn notice_actions_are_bounded_and_the_oldest_are_forgotten() {
        let state = FollowState::new(false, "#1");
        let first = state.offer_action("Reply", "page A=");
        for i in 0..MAX_NOTICE_ACTIONS {
            state.offer_action("Reply", &format!("page B{i}="));
        }
        assert_eq!(state.action(first), None, "the oldest offer is gone");
        let last = state.offer_action("Reply", "page Z=");
        assert_eq!(
            state.action(last),
            Some(("Reply".to_string(), "page Z=".to_string()))
        );
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
        let out: Vec<String> = api
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
        assert_eq!(out.len(), 1, "one coalesced message: {out:?}");
        assert!(out[0].starts_with("… 2 lines dropped"));
        assert!(out[0].contains("line 2") && out[0].contains("line 11"));
        assert!(!out[0].contains("\"line 1\"") && !out[0].contains("\"line 0\""));
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
        let out: Vec<String> = sends(&api).into_iter().map(|m| m.content).collect();
        assert_eq!(out.len(), 3, "{out:?}");
        assert!(
            out[1].starts_with("• [mail] New mail"),
            "priority first: {out:?}"
        );
        assert!(out[2].contains("later"));
        drop(tx);
        writer.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn a_slow_rest_send_must_not_block_the_session_drain() {
        let api = FakeApi::new();
        api.stall_send.store(true, Ordering::Relaxed);
        let state = FollowState::new(false, "#1");
        let (tx, rx) = mpsc::channel::<Arc<Envelope>>(256);
        let writer = tokio::spawn(run_dm_writer(
            api.clone(),
            555,
            rx,
            DmPolicy::new(50, 1900, 2000),
            state,
        ));
        tx.send(env(say("Nick", "first"))).await.unwrap();
        settle().await;
        tokio::time::advance(Duration::from_millis(60)).await;
        settle().await;
        assert!(api.calls().iter().any(|c| matches!(c, Call::Send { .. })));
        for _ in 0..256 {
            tx.try_send(env(say("Nick", "burst"))).unwrap();
            tokio::task::yield_now().await;
        }
        let result = tx.try_send(env(say("Nick", "last")));
        writer.abort();
        assert!(
            result.is_ok(),
            "The 256-slot session sink filled while the REST send was pending"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_request_is_its_own_message_recorded_and_expired_at_the_deadline() {
        let api = FakeApi::new();
        let state = FollowState::new(false, "#1");
        let (tx, rx) = mpsc::channel::<Arc<Envelope>>(256);
        let policy = DmPolicy::new(1_500, 1_900, 100);
        let writer = tokio::spawn(run_dm_writer(api.clone(), 555, rx, policy, state.clone()));
        let now_ms = u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap();
        // A narrative line is waiting; the request does not wait with it.
        tx.send(env(say("Nick", "before"))).await.unwrap();
        tx.send(env(OutputEvent::Request {
            epoch: "test".into(),
            id: 9,
            kind: RequestKind::Confirm,
            prompt: RichText::plain("Spend a willpower?"),
            options: vec![],
            deadline_ms: Some(now_ms + 2_000),
        }))
        .await
        .unwrap();
        settle().await;
        let out = sends(&api);
        assert_eq!(
            out.len(),
            2,
            "the request went out at once, then the line: {out:?}"
        );
        assert_eq!(out[0].content, "❓ Spend a willpower?");
        assert_eq!(out[0].components.len(), 1, "the Yes/No row");
        assert!(out[1].components.is_empty());
        let shown = state.request(9).expect("the request is recorded");
        assert_eq!(shown.channel_id, 555);
        assert_eq!(shown.text, "❓ Spend a willpower?");
        // Unanswered at the deadline: the message is edited, buttons removed.
        tokio::time::advance(Duration::from_millis(2_100)).await;
        settle().await;
        let edits: Vec<(u64, Outgoing)> = api
            .calls()
            .into_iter()
            .filter_map(|c| match c {
                Call::Edit {
                    message_id, msg, ..
                } => Some((message_id, msg)),
                _ => None,
            })
            .collect();
        assert_eq!(edits.len(), 1, "{edits:?}");
        assert_eq!(edits[0].0, shown.message_id);
        assert!(edits[0].1.content.contains("Expired"));
        assert!(edits[0].1.components.is_empty());
        assert_eq!(state.request(9), None);
        // Answered first (the bridge takes it): the deadline edits nothing.
        tx.send(env(OutputEvent::Request {
            epoch: "test".into(),
            id: 10,
            kind: RequestKind::Confirm,
            prompt: RichText::plain("Again?"),
            options: vec![],
            deadline_ms: Some(now_ms + 4_000),
        }))
        .await
        .unwrap();
        settle().await;
        assert!(state.take_request(10).is_some());
        tokio::time::advance(Duration::from_millis(3_000)).await;
        settle().await;
        let edits = api
            .calls()
            .iter()
            .filter(|c| matches!(c, Call::Edit { .. }))
            .count();
        assert_eq!(edits, 1, "no second edit");
        drop(tx);
        writer.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn refused_components_fall_back_to_plain_text() {
        let api = FakeApi::new();
        let state = FollowState::new(false, "#1");
        let (tx, rx) = mpsc::channel::<Arc<Envelope>>(256);
        let policy = DmPolicy::new(100, 1_900, 100);
        let writer = tokio::spawn(run_dm_writer(api.clone(), 555, rx, policy, state.clone()));
        api.script([Some(ApiError::Permanent("bad component".into()))]);
        tx.send(env(OutputEvent::Request {
            epoch: "test".into(),
            id: 3,
            kind: RequestKind::Text,
            prompt: RichText::plain("Name it."),
            options: vec![],
            deadline_ms: None,
        }))
        .await
        .unwrap();
        settle().await;
        let out = sends(&api);
        assert_eq!(out.len(), 2, "{out:?}");
        assert!(!out[0].components.is_empty(), "the refused attempt");
        assert!(out[1].components.is_empty());
        assert!(out[1].content.contains("respond"));
        assert_eq!(state.request(3), None, "nothing to edit later");
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
        let out: Vec<String> = sends(&api).into_iter().map(|m| m.content).collect();
        // The first attempt failed; the retry carried both lines, in order.
        assert_eq!(out.len(), 2, "{out:?}");
        assert!(
            out[1].contains("\"first\"\n") && out[1].ends_with("\"second\""),
            "{out:?}"
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

    #[tokio::test(start_paused = true)]
    async fn stalled_send_keeps_priority_and_only_newest_narrative_then_cancels() {
        let api = FakeApi::new();
        api.stall_send.store(true, Ordering::Relaxed);
        let state = FollowState::new(false, "#1");
        let (tx, rx) = mpsc::channel(256);
        let writer = tokio::spawn(run_dm_writer(
            api.clone(),
            555,
            rx,
            DmPolicy::new(50, 1900, 10),
            state.clone(),
        ));
        tx.send(env(say("Nick", "first"))).await.unwrap();
        settle().await;
        tokio::time::advance(Duration::from_millis(60)).await;
        settle().await;
        for i in 0..20 {
            tx.send(env(say("Nick", &format!("queued-{i}"))))
                .await
                .unwrap();
        }
        tx.send(env(OutputEvent::Request {
            epoch: "test".into(),
            id: 4,
            kind: RequestKind::Confirm,
            prompt: RichText::plain("priority ask"),
            options: vec![],
            deadline_ms: None,
        }))
        .await
        .unwrap();
        settle().await;
        assert_eq!(sends(&api).len(), 1);
        api.stall_send.store(false, Ordering::Relaxed);
        api.send_release.notify_one();
        settle().await;
        let output = sends(&api);
        assert!(output[1].content.contains("priority ask"), "{output:?}");
        let rest = output[2..]
            .iter()
            .map(|o| o.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            rest.contains("10 lines dropped") && rest.contains("queued-19"),
            "{rest}"
        );
        assert!(!rest.contains("queued-0"));
        api.stall_send.store(true, Ordering::Relaxed);
        tx.send(env(say("Nick", "in flight"))).await.unwrap();
        settle().await;
        tokio::time::advance(Duration::from_millis(60)).await;
        settle().await;
        let count = sends(&api).len();
        state.cancelled.cancel();
        writer.await.unwrap();
        api.send_release.notify_one();
        settle().await;
        assert_eq!(sends(&api).len(), count);
        assert!(tx.is_closed());
    }

    #[tokio::test(start_paused = true)]
    async fn priority_arrival_cannot_shorten_retry_backoff() {
        let api = FakeApi::new();
        api.script([Some(ApiError::Retryable("429".into()))]);
        let state = FollowState::new(false, "#1");
        let (tx, rx) = mpsc::channel(256);
        let writer = tokio::spawn(run_dm_writer(
            api.clone(),
            555,
            rx,
            DmPolicy::new(50, 1900, 2000),
            state.clone(),
        ));
        tx.send(env(notice("old priority"))).await.unwrap();
        settle().await;
        tx.send(env(notice("new priority"))).await.unwrap();
        settle().await;
        assert_eq!(sends(&api).len(), 1);
        tokio::time::advance(RETRY_BACKOFF).await;
        settle().await;
        let output = sends(&api);
        assert!(output[1].content.contains("old priority"));
        assert!(output[2].content.contains("new priority"));
        state.cancelled.cancel();
        writer.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn priority_overflow_and_closing_flush_are_bounded() {
        for overflow in [false, true] {
            let api = FakeApi::new();
            api.stall_send.store(true, Ordering::Relaxed);
            let state = FollowState::new(false, "#1");
            let (tx, rx) = mpsc::channel(256);
            let writer = tokio::spawn(run_dm_writer(
                api.clone(),
                555,
                rx,
                DmPolicy::new(50, 1900, 2000),
                state.clone(),
            ));
            tx.send(env(notice("stalled"))).await.unwrap();
            settle().await;
            if overflow {
                for _ in 0..128 {
                    tx.send(env(notice("priority"))).await.unwrap();
                }
                settle().await;
                assert!(state.dm_closed.load(Ordering::Relaxed));
            }
            drop(tx);
            writer.await.unwrap(); // paused time reaches the five-second cap
            assert_eq!(sends(&api).len(), 1);
        }
    }

    #[test]
    fn answering_a_previous_message_leaves_its_replacement_open() {
        let state = FollowState::new(false, "#1");
        state.request_shown(
            1,
            RequestMessage {
                epoch: "new-world".into(),
                channel_id: 5,
                message_id: 9,
                text: "new ask".into(),
                rows: vec![],
            },
        );
        assert!(state.take_request_message(1, 8).is_none());
        assert_eq!(state.request(1).unwrap().message_id, 9);
        assert!(state.take_request_message(1, 9).is_some());
    }
}
