//! The output pipeline: audiences, the [`Outbox`], and view builders.
//!
//! Per treaty ruling 1 the core emits [`mush_proto`] types **directly** —
//! there is no parallel event enum here. Handlers queue
//! `(Audience, OutputEvent)` pairs into an [`Outbox`]; the (future) world
//! task resolves game audiences (room → actor list) and the session layer
//! resolves actors → live connections (treaty ruling 2). In tests the outbox
//! is simply inspected.

use mush_proto::{ExitRef, ObjRef, OutputEvent, RichText, RoomView, Span};

use crate::cmd::Registry;
use crate::engine::Deferred;
use crate::flags::{Flags, Powers};
use crate::module::perceives_via_action_state;
use crate::obj::{ObjId, ObjKind};
use crate::perm;
use crate::world::World;

/// Durability relationship of presentation or external work produced by one
/// world step. The driver uses this tag after persistence; it never infers a
/// commit claim from event text or a command name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryClass {
    /// Release only after the step's authoritative write is acknowledged.
    OnCommit,
    /// Commit-bound output reconstructible from durable state, such as a
    /// panel keyframe.
    Recoverable,
    /// Delivery represented in a durable outbox. K3 adds dequeue receipts;
    /// K0 treats it as commit-bound.
    DurableOutbox,
    /// Non-authoritative status/transport output that makes no commit claim.
    BestEffort,
    /// A refusal/failure response released only when the step aborts (or no
    /// authoritative write was attempted).
    OnAbort,
}

/// Durable disposition of the world step that produced a queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryOutcome {
    /// The step produced no authoritative write, so its ordinary output is
    /// immediately final.
    NoWrite,
    /// The exact write was acknowledged.
    Committed,
    /// Persistence deterministically refused or lost the write.
    Aborted,
}

impl DeliveryClass {
    /// Whether an item in this class may be released for `outcome`.
    #[must_use]
    pub const fn releases_on(self, outcome: DeliveryOutcome) -> bool {
        match outcome {
            DeliveryOutcome::NoWrite => true,
            DeliveryOutcome::Committed => !matches!(self, Self::OnAbort),
            DeliveryOutcome::Aborted => matches!(self, Self::BestEffort | Self::OnAbort),
        }
    }
}

/// Connection id, allocated by the session layer. The core only needs an
/// opaque handle to address one specific connection (pre-login prompts,
/// per-connection routing); `mush-session` owns allocation and delivery.
pub type ConnId = u64;

/// Who should receive an event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Audience {
    /// One specific connection (pre-auth, panel hydration).
    Conn(ConnId),
    /// Every connection attached to one actor.
    Actor(ObjId),
    /// Everyone in a room, minus the excluded actors (usually the one who
    /// already got a personalized line).
    Room {
        /// The room whose occupants should hear it.
        room: ObjId,
        /// Actors in the room who should *not* receive the event.
        exclude: Vec<ObjId>,
        /// The occupant this room line *reveals* — its perception subject
        /// (M11). The fan-out skips a recipient who cannot
        /// [perceive](crate::module::can_perceive) this subject. `None` for
        /// lines that name no one (weather, panels) — delivered to the whole
        /// room. Movement/arrival/departure/take/drop/give/connect lines set
        /// it to the acting mover so a concealed actor's echoes are gated even
        /// though they ride an *unattributed* [`OutputEvent::Narrative`]
        /// (`actor: None`) — the subject travels in the routing layer, so the
        /// wire event is byte-for-byte unchanged when no hook is registered.
        /// Attributed speech (say/pose/`@emit`) leaves this `None` and is
        /// gated via [`crate::commands::event_subject`] on the event instead.
        subject: Option<ObjId>,
    },
    /// Every connection on the game (`@wall`, shutdown notices).
    All,
}

/// An ordered queue of `(Audience, OutputEvent)` pairs collected while a
/// command runs. Draining and delivery belong to the caller.
///
/// The outbox also carries the command's **deferred work side-queue**
/// ([`Outbox::defer`]): handlers are sync and cannot reach the effects
/// queue or the scheduler directly, so they describe async work / timers
/// as [`Deferred`] values here and
/// [`Engine::step`](crate::engine::Engine::step) routes them after the
/// handler returns.
#[derive(Debug, Default)]
pub struct Outbox {
    events: Vec<(Audience, OutputEvent)>,
    // Parallel storage preserves the long-standing `events()`/`take()` API
    // while giving the persistence driver typed entries.
    classes: Vec<DeliveryClass>,
    deferred: Vec<Deferred>,
    /// How much of `deferred` is **pinned** — non-truncatable by a failing
    /// handler. See [`Outbox::pin_deferred`].
    deferred_pin: usize,
}

impl Outbox {
    /// An empty outbox.
    #[must_use]
    pub fn new() -> Outbox {
        Outbox::default()
    }

    /// Queue an event for an arbitrary audience.
    pub fn push(&mut self, audience: Audience, event: OutputEvent) {
        self.push_classified(DeliveryClass::OnCommit, audience, event);
    }

    /// Queue an event with an explicit durability relationship.
    pub fn push_classified(
        &mut self,
        class: DeliveryClass,
        audience: Audience,
        event: OutputEvent,
    ) {
        self.events.push((audience, event));
        self.classes.push(class);
    }

    /// Queue an event to a single actor.
    pub fn to_actor(&mut self, actor: ObjId, event: OutputEvent) {
        self.push(Audience::Actor(actor), event);
    }

    /// Queue an event to a single connection.
    pub fn to_conn(&mut self, conn: ConnId, event: OutputEvent) {
        self.push(Audience::Conn(conn), event);
    }

    /// Queue non-authoritative connection/status output that never claims a
    /// mutation committed.
    pub fn best_effort_to_conn(&mut self, conn: ConnId, event: OutputEvent) {
        self.push_classified(DeliveryClass::BestEffort, Audience::Conn(conn), event);
    }

    /// Queue an error-styled, non-authoritative diagnostic to one actor.
    ///
    /// Use this for transient/runtime truth that remains useful regardless of
    /// whether the step's unrelated authoritative writes commit (for example,
    /// refusing one more entry from an in-memory scheduler queue).
    pub fn best_effort_error(&mut self, actor: ObjId, msg: impl Into<String>) {
        self.push_classified(
            DeliveryClass::BestEffort,
            Audience::Actor(actor),
            OutputEvent::Sys(RichText::tag("error", msg)),
        );
    }

    /// Queue an event to everyone in `room` except `exclude`. The line names
    /// no perception subject in the routing layer (attributed speech is gated
    /// via its event actor; see [`Audience::Room`]).
    pub fn to_room(&mut self, room: ObjId, exclude: Vec<ObjId>, event: OutputEvent) {
        self.push(
            Audience::Room {
                room,
                exclude,
                subject: None,
            },
            event,
        );
    }

    /// Queue a room line that *reveals* `subject` — the acting mover of an
    /// unattributed narrative (movement/arrival/departure/take/drop/give/
    /// connect). The M11 fan-out skips any recipient who cannot
    /// [perceive](crate::module::can_perceive) `subject`, so a concealed
    /// actor's echoes are gated. Byte-for-byte identical to [`to_room`](Self::to_room)
    /// when no `Perceive` hook is registered (the subject only affects routing).
    pub fn to_room_from(
        &mut self,
        room: ObjId,
        exclude: Vec<ObjId>,
        subject: ObjId,
        event: OutputEvent,
    ) {
        self.push(
            Audience::Room {
                room,
                exclude,
                subject: Some(subject),
            },
            event,
        );
    }

    /// Queue an event to everyone connected.
    pub fn to_all(&mut self, event: OutputEvent) {
        self.push(Audience::All, event);
    }

    /// Queue a [`OutputEvent::Sys`] line to one actor, tagged `tag`
    /// (`"error"`, `"system"`, …).
    pub fn sys_to_actor(&mut self, actor: ObjId, tag: &str, msg: impl Into<String>) {
        self.to_actor(actor, OutputEvent::Sys(RichText::tag(tag, msg)));
    }

    /// Queue an error line (`Sys` tagged `"error"`) to one actor.
    pub fn error(&mut self, actor: ObjId, msg: impl Into<String>) {
        self.push_classified(
            DeliveryClass::OnAbort,
            Audience::Actor(actor),
            OutputEvent::Sys(RichText::tag("error", msg)),
        );
    }

    /// Queue an error-styled line whose truth depends on the step committing.
    ///
    /// Use this for a diagnostic that accompanies a successful mutation (for
    /// example, disabling a faulty script). Unlike [`error`](Self::error), it
    /// is withheld on authoritative abort and released after the exact write
    /// receipt. A step that produced no write releases it immediately.
    pub fn error_on_commit(&mut self, actor: ObjId, msg: impl Into<String>) {
        self.push_classified(
            DeliveryClass::OnCommit,
            Audience::Actor(actor),
            OutputEvent::Sys(RichText::tag("error", msg)),
        );
    }

    /// Queue explanatory failure detail to one actor without styling it as
    /// the terminal error.  A handler may emit several of these and then
    /// return `Err`; dispatch preserves them while discarding any speculative
    /// success presentation from that handler.
    pub fn abort_info(&mut self, actor: ObjId, msg: impl Into<String>) {
        self.push_classified(
            DeliveryClass::OnAbort,
            Audience::Actor(actor),
            OutputEvent::Sys(RichText::tag("system", msg)),
        );
    }

    /// Queue an informational line (`Sys` tagged `"system"`) to one actor.
    pub fn info(&mut self, actor: ObjId, msg: impl Into<String>) {
        self.sys_to_actor(actor, "system", msg);
    }

    /// Defer async work or a timer (routed by the engine after the current
    /// handler returns — see the type-level docs).
    pub fn defer(&mut self, deferred: Deferred) {
        self.deferred.push(deferred);
    }

    /// The deferred work queued so far, in push order.
    #[must_use]
    pub fn deferred(&self) -> &[Deferred] {
        &self.deferred
    }

    /// Pin the deferred-work queued **so far** as non-truncatable: a later
    /// [`truncate_deferred`](Self::truncate_deferred) can only discard entries
    /// pushed after this call.
    ///
    /// The one caller is a **terminal commit** — `mush_canon::commit`, which
    /// appends to the chronicle, then synchronously runs observers that may
    /// mutate the world and queue writes of their own. Dispatch does not roll
    /// RAM back when a handler returns `Err`, only the deferred queue, so
    /// truncating a terminal commit's writes leaves RAM claiming something the
    /// store never received — canon recorded, an economy contract settled, and
    /// neither on disk. Pinning makes "committed" mean the same thing on both
    /// sides. Everything queued *after* the pin is still discarded normally.
    pub fn pin_deferred(&mut self) {
        self.deferred_pin = self.deferred.len();
    }

    /// How much of the deferred queue is pinned (see
    /// [`pin_deferred`](Self::pin_deferred)).
    #[must_use]
    pub fn deferred_pin(&self) -> usize {
        self.deferred_pin
    }

    /// Truncate the deferred-work side-queue back to `len` entries — or to the
    /// [pin](Self::pin_deferred), whichever is longer.
    /// Dispatch uses this to discard everything a **failed** handler
    /// deferred (persistence ops, timers, effects) — an `Err` return must
    /// not half-commit the handler's queued writes.
    pub fn truncate_deferred(&mut self, len: usize) {
        self.deferred.truncate(len.max(self.deferred_pin));
    }

    /// Drain the deferred-work side-queue, leaving it empty. (Separate from
    /// [`Outbox::take`]: events go to delivery, deferred work goes to the
    /// engine's effects queue and scheduler.)
    pub fn take_deferred(&mut self) -> Vec<Deferred> {
        self.deferred_pin = 0;
        std::mem::take(&mut self.deferred)
    }

    /// The queued events, in emission order.
    #[must_use]
    pub fn events(&self) -> &[(Audience, OutputEvent)] {
        &self.events
    }

    /// Durability classes parallel to [`events`](Self::events).
    #[must_use]
    pub fn classes(&self) -> &[DeliveryClass] {
        &self.classes
    }

    /// Drain typed events in emission order.
    pub fn take_classified(&mut self) -> Vec<(DeliveryClass, Audience, OutputEvent)> {
        debug_assert_eq!(self.events.len(), self.classes.len());
        let events = std::mem::take(&mut self.events);
        let classes = std::mem::take(&mut self.classes);
        classes
            .into_iter()
            .zip(events)
            .map(|(class, (audience, event))| (class, audience, event))
            .collect()
    }

    /// Truncate emitted events and their durability tags together. Dispatch
    /// uses this to discard partial success presentation from a handler that
    /// ultimately returns an error.
    pub fn truncate(&mut self, len: usize) {
        self.events.truncate(len);
        self.classes.truncate(len);
    }

    /// Roll a failed handler back to `len`, retaining only suffix output whose
    /// class is explicitly safe to release on abort.  This keeps useful,
    /// ordered refusal diagnostics while ensuring partial success messages
    /// cannot escape a handler that ultimately failed.
    pub fn retain_abort_since(&mut self, len: usize) {
        debug_assert_eq!(self.events.len(), self.classes.len());
        debug_assert!(len <= self.events.len());

        let suffix_events: Vec<_> = self.events.drain(len..).collect();
        let suffix_classes: Vec<_> = self.classes.drain(len..).collect();
        for (class, event) in suffix_classes.into_iter().zip(suffix_events) {
            if class.releases_on(DeliveryOutcome::Aborted) {
                self.classes.push(class);
                self.events.push(event);
            }
        }
    }

    /// Drain the queue, leaving the outbox empty.
    pub fn take(&mut self) -> Vec<(Audience, OutputEvent)> {
        debug_assert_eq!(self.events.len(), self.classes.len());
        self.classes.clear();
        std::mem::take(&mut self.events)
    }

    /// Number of queued events.
    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Whether nothing has been queued.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

/// The semantic span tag for an object kind (`"player"`, `"npc"`, `"exit"`,
/// `"room"`, `"thing"`). Renderers resolve these to theme colors / CSS.
#[must_use]
pub fn kind_tag(kind: ObjKind) -> &'static str {
    match kind {
        ObjKind::Room => "room",
        ObjKind::Exit => "exit",
        ObjKind::Thing => "thing",
        ObjKind::Player => "player",
        ObjKind::Npc => "npc",
    }
}

/// The display name of an object: for exits, the primary (first `;`-segment)
/// name; `"nothing"` for missing objects.
#[must_use]
pub fn display_name(w: &World, id: ObjId) -> String {
    match w.get(id) {
        Some(o) => primary_name(&o.name).to_string(),
        None => "nothing".to_string(),
    }
}

/// The primary segment of a possibly `;`-aliased name (`"north;n"` →
/// `"north"`).
#[must_use]
pub fn primary_name(name: &str) -> &str {
    name.split(';').next().unwrap_or(name).trim()
}

/// A wire [`ObjRef`] (`{id: "#5", name: "sword"}`) for an object.
#[must_use]
pub fn obj_ref(w: &World, id: ObjId) -> ObjRef {
    ObjRef::new(id.to_string(), display_name(w, id))
}

/// A [`Span`] carrying the object's display name, semantically tagged by
/// its kind — the building block for MUSH-style narrative lines.
#[must_use]
pub fn name_span(w: &World, id: ObjId) -> Span {
    match w.get(id) {
        Some(o) => Span::tagged(kind_tag(o.kind), primary_name(&o.name)),
        None => Span::plain("nothing"),
    }
}

/// Build the structured room render for `looker` looking at `room`
/// (`None` if the room does not exist).
///
/// Visibility rules honored:
/// - a `DARK` room hides its contents entirely;
/// - an `OPAQUE` room hides contents from lookers **outside** it;
/// - `DARK` objects and exits are omitted individually;
/// - `GOING` husks never appear;
/// - all of the above are waived when the looker controls the room/object
///   or holds the `SEE_HIDDEN` power;
/// - an occupant the looker cannot [perceive](can_perceive) (the M11
///   per-observer projection gate) is omitted — a free pass with no
///   `Perceive` hook registered, so the listing is unchanged for a game
///   without a concealment ruleset.
#[must_use]
pub fn room_view(reg: &Registry, w: &World, looker: ObjId, room: ObjId) -> Option<RoomView> {
    room_view_with_action_state(reg, w, looker, room, None)
}

pub(crate) fn room_view_with_action_state(
    reg: &Registry,
    w: &World,
    looker: ObjId,
    room: ObjId,
    action_state: Option<&crate::action_state::ActionStateStore>,
) -> Option<RoomView> {
    let r = w.get(room)?;
    let see_hidden = perm::has_power(w, looker, Powers::SEE_HIDDEN);
    let inside = w.get(looker).and_then(|o| o.location) == Some(room);
    let room_privileged = see_hidden || perm::controls(w, looker, room);

    let desc = w.attr(room, "desc").and_then(|a| a.as_text()).map_or_else(
        || RichText::plain("You see nothing special."),
        RichText::plain,
    );

    let hide_contents = !room_privileged
        && (r.flags.contains(Flags::DARK) || (r.flags.contains(Flags::OPAQUE) && !inside));

    let mut contents = Vec::new();
    if !hide_contents {
        for &c in &r.contents {
            if c == looker {
                continue;
            }
            let Some(o) = w.get(c) else { continue };
            if o.flags.contains(Flags::GOING) {
                continue;
            }
            // Disconnected players stay in the room (they don't leave on
            // logout) but are hidden from the `look` listing — nobody wants a
            // room that reads as crowded with logged-off people, and this
            // drops system objects like God from every room. NPCs are always
            // present. Not waived for staff: a staffer wanting to see who's
            // offline here uses `examine`, which lists raw contents.
            if o.kind == ObjKind::Player && !w.is_connected(c) {
                continue;
            }
            if o.flags.contains(Flags::DARK) && !see_hidden && !perm::controls(w, looker, c) {
                continue;
            }
            // M11 per-observer projection: hide an occupant this looker
            // cannot perceive (Obfuscate vs Auspex, …). Free pass with no
            // Perceive hook registered.
            if !perceives_via_action_state(
                reg.hooks(crate::module::HookPoint::Perceive),
                w,
                looker,
                c,
                action_state,
            ) {
                continue;
            }
            // Primary segment only: "lever;switch" renders as "lever",
            // consistent with every other name path.
            contents.push(ObjRef::new(
                c.to_string(),
                primary_name(&o.name).to_string(),
            ));
        }
    }

    let mut exits = Vec::new();
    for &e in &r.exits {
        let Some(o) = w.get(e) else { continue };
        if o.flags.contains(Flags::GOING) {
            continue;
        }
        if o.flags.contains(Flags::DARK) && !see_hidden && !perm::controls(w, looker, e) {
            continue;
        }
        let mut segs = o.name.split(';').map(str::trim).filter(|s| !s.is_empty());
        let name = segs.next().unwrap_or("").to_string();
        let mut aliases: Vec<String> = segs.map(str::to_string).collect();
        aliases.extend(o.aliases.iter().map(|a| a.to_string()));
        exits.push(ExitRef { name, aliases });
    }

    let zone = r
        .zone
        .and_then(|z| w.get(z))
        .map(|zo| ObjRef::new(zo.id.to_string(), zo.name.clone()));

    // Structured ambience (weather, scene, the IC clock) for a client's room
    // band; the `LookRendered` prose lines still follow the `Room` event for
    // line mode (interface plan I1, D-I-4).
    let ambient = crate::module::room_ambient_with_action_state(reg, w, looker, room, action_state);

    Some(RoomView {
        name: r.name.clone(),
        id: room.to_string(),
        desc,
        contents,
        exits,
        zone,
        ambient,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attr::Attr;
    use crate::obj::{GOD, LIMBO};

    fn setup() -> (World, ObjId, ObjId) {
        let mut w = World::new_minimal();
        let nick = w.create(ObjKind::Player, "Nick", GOD, Some(LIMBO)).unwrap();
        let hall = w.create(ObjKind::Room, "Hall", GOD, None).unwrap();
        (w, nick, hall)
    }

    #[test]
    fn outbox_helpers_and_take() {
        let mut out = Outbox::new();
        assert!(out.is_empty());
        out.error(ObjId(2), "nope");
        out.info(ObjId(2), "fine");
        out.to_room(
            LIMBO,
            vec![ObjId(2)],
            OutputEvent::Sys(RichText::plain("x")),
        );
        assert_eq!(out.len(), 3);
        match &out.events()[0] {
            (Audience::Actor(a), OutputEvent::Sys(t)) => {
                assert_eq!(*a, ObjId(2));
                assert_eq!(t.to_plain_text(), "nope");
                assert_eq!(t.spans()[0].tag.as_deref(), Some("error"));
            }
            other => panic!("unexpected: {other:?}"),
        }
        match &out.events()[1] {
            (_, OutputEvent::Sys(t)) => {
                assert_eq!(t.spans()[0].tag.as_deref(), Some("system"));
            }
            other => panic!("unexpected: {other:?}"),
        }
        let drained = out.take();
        assert_eq!(drained.len(), 3);
        assert!(out.is_empty());
    }

    #[test]
    fn classified_output_preserves_order_and_legacy_take_clears_tags() {
        let mut out = Outbox::new();
        out.push_classified(
            DeliveryClass::BestEffort,
            Audience::Conn(7),
            OutputEvent::Sys(RichText::plain("status")),
        );
        out.info(ObjId(2), "committed");
        out.error_on_commit(ObjId(2), "committed error");
        out.error(ObjId(2), "aborted");
        assert_eq!(
            out.classes(),
            &[
                DeliveryClass::BestEffort,
                DeliveryClass::OnCommit,
                DeliveryClass::OnCommit,
                DeliveryClass::OnAbort,
            ]
        );

        let classified = out.take_classified();
        assert_eq!(classified.len(), 4);
        assert_eq!(classified[0].0, DeliveryClass::BestEffort);
        assert_eq!(classified[1].0, DeliveryClass::OnCommit);
        assert_eq!(classified[2].0, DeliveryClass::OnCommit);
        assert_eq!(classified[3].0, DeliveryClass::OnAbort);
        assert!(out.is_empty());
        assert!(out.classes().is_empty());

        out.info(ObjId(2), "legacy");
        assert_eq!(out.take().len(), 1);
        assert!(out.classes().is_empty());
    }

    #[test]
    fn deterministic_abort_releases_only_failure_or_non_authoritative_output() {
        for class in [
            DeliveryClass::OnCommit,
            DeliveryClass::Recoverable,
            DeliveryClass::DurableOutbox,
        ] {
            assert!(!class.releases_on(DeliveryOutcome::Aborted));
            assert!(class.releases_on(DeliveryOutcome::Committed));
        }
        assert!(DeliveryClass::BestEffort.releases_on(DeliveryOutcome::Aborted));
        assert!(DeliveryClass::OnAbort.releases_on(DeliveryOutcome::Aborted));
        assert!(!DeliveryClass::OnAbort.releases_on(DeliveryOutcome::Committed));
    }

    #[test]
    fn best_effort_error_is_styled_and_survives_every_disposition() {
        let mut out = Outbox::new();
        out.best_effort_error(ObjId(2), "runtime refusal");

        assert_eq!(out.classes(), &[DeliveryClass::BestEffort]);
        assert!(DeliveryClass::BestEffort.releases_on(DeliveryOutcome::NoWrite));
        assert!(DeliveryClass::BestEffort.releases_on(DeliveryOutcome::Committed));
        assert!(DeliveryClass::BestEffort.releases_on(DeliveryOutcome::Aborted));
        match &out.events()[0] {
            (Audience::Actor(actor), OutputEvent::Sys(text)) => {
                assert_eq!(*actor, ObjId(2));
                assert_eq!(text.to_plain_text(), "runtime refusal");
                assert_eq!(text.spans()[0].tag.as_deref(), Some("error"));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn failed_handler_checkpoint_keeps_only_abort_safe_suffix_in_order() {
        let mut out = Outbox::new();
        out.info(ObjId(1), "earlier");
        let checkpoint = out.len();
        out.info(ObjId(2), "speculative success");
        out.abort_info(ObjId(2), "first refusal");
        out.push_classified(
            DeliveryClass::BestEffort,
            Audience::Conn(9),
            OutputEvent::Sys(RichText::plain("transport status")),
        );
        out.error(ObjId(2), "second refusal");

        out.retain_abort_since(checkpoint);

        assert_eq!(
            out.classes(),
            &[
                DeliveryClass::OnCommit,
                DeliveryClass::OnAbort,
                DeliveryClass::BestEffort,
                DeliveryClass::OnAbort,
            ]
        );
        let text: Vec<_> = out
            .events()
            .iter()
            .map(|(_, event)| match event {
                OutputEvent::Sys(text) => text.to_plain_text(),
                other => panic!("unexpected event: {other:?}"),
            })
            .collect();
        assert_eq!(
            text,
            [
                "earlier",
                "first refusal",
                "transport status",
                "second refusal"
            ]
        );
    }

    #[test]
    fn name_span_tags_by_kind() {
        let (mut w, nick, hall) = setup();
        let exit = w.create(ObjKind::Exit, "north;n", GOD, Some(hall)).unwrap();
        let npc = w.create(ObjKind::Npc, "Barkeep", GOD, Some(hall)).unwrap();
        let thing = w.create(ObjKind::Thing, "mug", GOD, Some(hall)).unwrap();
        assert_eq!(name_span(&w, nick).tag.as_deref(), Some("player"));
        assert_eq!(name_span(&w, npc).tag.as_deref(), Some("npc"));
        assert_eq!(name_span(&w, hall).tag.as_deref(), Some("room"));
        assert_eq!(name_span(&w, thing).tag.as_deref(), Some("thing"));
        let e = name_span(&w, exit);
        assert_eq!(e.tag.as_deref(), Some("exit"));
        assert_eq!(e.text, "north"); // primary segment only
        assert_eq!(name_span(&w, ObjId(99)).text, "nothing");
        assert_eq!(obj_ref(&w, nick).id, nick.to_string());
    }

    #[test]
    fn room_view_basics_and_desc() {
        let reg = Registry::new();
        let (mut w, nick, hall) = setup();
        w.move_to(nick, hall).unwrap();
        w.set_attr(hall, "desc", Attr::text("A big hall.", GOD))
            .unwrap();
        let exit = w.create(ObjKind::Exit, "north;n", GOD, Some(hall)).unwrap();
        w.add_alias(exit, "no").unwrap();
        let ball = w.create(ObjKind::Thing, "ball", GOD, Some(hall)).unwrap();

        let rv = room_view(&reg, &w, nick, hall).unwrap();
        assert_eq!(rv.name, "Hall");
        assert_eq!(rv.id, hall.to_string());
        assert_eq!(rv.desc.to_plain_text(), "A big hall.");
        // Looker excluded; ball listed.
        assert_eq!(rv.contents.len(), 1);
        assert_eq!(rv.contents[0].name, "ball");
        assert_eq!(rv.contents[0].id, ball.to_string());
        // Exit name split into primary + aliases (segments then alias list).
        assert_eq!(rv.exits.len(), 1);
        assert_eq!(rv.exits[0].name, "north");
        assert_eq!(rv.exits[0].aliases, vec!["n".to_string(), "no".to_string()]);
        // Default desc when unset.
        let rv0 = room_view(&reg, &w, nick, LIMBO).unwrap();
        assert_eq!(rv0.desc.to_plain_text(), "You see nothing special.");
        assert!(room_view(&reg, &w, nick, ObjId(99)).is_none());
    }

    #[test]
    fn room_view_honors_dark_and_opaque() {
        let reg = Registry::new();
        let (mut w, nick, hall) = setup();
        w.move_to(nick, hall).unwrap();
        w.set_connected(nick, true); // a listed player must be connected (see below)
        let ball = w.create(ObjKind::Thing, "ball", GOD, Some(hall)).unwrap();
        let exit = w.create(ObjKind::Exit, "north", GOD, Some(hall)).unwrap();

        // DARK object hidden from plain players, visible to God (controls).
        // (Hall holds Nick and the ball; the looker is never listed.)
        w.set_flag(ball, Flags::DARK, true).unwrap();
        assert!(room_view(&reg, &w, nick, hall).unwrap().contents.is_empty());
        assert_eq!(room_view(&reg, &w, GOD, hall).unwrap().contents.len(), 2);
        w.set_flag(ball, Flags::DARK, false).unwrap();

        // DARK exit hidden.
        w.set_flag(exit, Flags::DARK, true).unwrap();
        assert!(room_view(&reg, &w, nick, hall).unwrap().exits.is_empty());
        w.set_flag(exit, Flags::DARK, false).unwrap();

        // DARK room hides all contents from non-controllers.
        w.set_flag(hall, Flags::DARK, true).unwrap();
        assert!(room_view(&reg, &w, nick, hall).unwrap().contents.is_empty());
        assert_eq!(room_view(&reg, &w, GOD, hall).unwrap().contents.len(), 2);
        w.set_flag(hall, Flags::DARK, false).unwrap();

        // OPAQUE hides contents from outside lookers only.
        w.set_flag(hall, Flags::OPAQUE, true).unwrap();
        assert_eq!(room_view(&reg, &w, nick, hall).unwrap().contents.len(), 1);
        let outsider = w.create(ObjKind::Player, "Out", GOD, Some(LIMBO)).unwrap();
        assert!(
            room_view(&reg, &w, outsider, hall)
                .unwrap()
                .contents
                .is_empty()
        );

        // SEE_HIDDEN power waives DARK (and OPAQUE-from-outside).
        w.set_flag(hall, Flags::DARK, true).unwrap();
        w.get_mut(outsider).unwrap().powers = Powers::SEE_HIDDEN;
        assert_eq!(
            room_view(&reg, &w, outsider, hall).unwrap().contents.len(),
            2
        );
    }

    #[test]
    fn room_view_hides_disconnected_players() {
        let reg = Registry::new();
        let (mut w, nick, hall) = setup();
        w.move_to(nick, hall).unwrap();
        w.set_connected(nick, true);
        let bob = w.create(ObjKind::Player, "Bob", GOD, Some(hall)).unwrap(); // offline
        let npc = w.create(ObjKind::Npc, "Barkeep", GOD, Some(hall)).unwrap();
        let mug = w.create(ObjKind::Thing, "mug", GOD, Some(hall)).unwrap();

        // Nick (connected) looks: sees the connected... nothing of self; the
        // NPC and the thing; NOT the offline Bob.
        let names: Vec<_> = room_view(&reg, &w, nick, hall)
            .unwrap()
            .contents
            .into_iter()
            .map(|r| r.name)
            .collect();
        assert!(names.contains(&"Barkeep".to_string()));
        assert!(names.contains(&"mug".to_string()));
        assert!(!names.contains(&"Bob".to_string()), "offline player hidden");

        // Not waived for staff/controllers either — God's look also hides Bob
        // (staff use `examine` to see who's offline in a room).
        let god_names: Vec<_> = room_view(&reg, &w, GOD, hall)
            .unwrap()
            .contents
            .into_iter()
            .map(|r| r.name)
            .collect();
        assert!(!god_names.contains(&"Bob".to_string()));
        assert!(god_names.contains(&"Nick".to_string())); // connected player shows

        // Bob reconnecting makes him appear.
        w.set_connected(bob, true);
        let names2: Vec<_> = room_view(&reg, &w, nick, hall)
            .unwrap()
            .contents
            .into_iter()
            .map(|r| r.name)
            .collect();
        assert!(names2.contains(&"Bob".to_string()));
        let _ = npc;
        let _ = mug;
    }

    #[test]
    fn room_view_includes_zone() {
        let reg = Registry::new();
        let (mut w, nick, hall) = setup();
        let zmo = w
            .create(ObjKind::Thing, "Downtown", GOD, Some(LIMBO))
            .unwrap();
        w.set_flag(zmo, Flags::ZONE_MASTER, true).unwrap();
        w.set_zone(hall, Some(zmo)).unwrap();
        let rv = room_view(&reg, &w, nick, hall).unwrap();
        assert_eq!(rv.zone.unwrap().name, "Downtown");
    }
}
