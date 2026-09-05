//! # mush-testkit
//!
//! An in-process driver for integration-style tests: no sockets, no tokio,
//! no session layer — just a [`World`], the core command
//! [`Registry`](mush_core::Registry), and helpers that make asserting on
//! [`OutputEvent`] streams pleasant.
//!
//! The centerpiece is [`TestWorld`]:
//!
//! ```
//! use mush_core::LIMBO;
//! use mush_testkit::{TestWorld, assert_room_saw};
//!
//! let mut tw = TestWorld::new();
//! let alice = tw.spawn_player("Alice");
//! let events = tw.run(alice, "\"Hello!");
//! assert_room_saw(&events, LIMBO, "Hello!");
//! ```
//!
//! [`boot_world`] is the shared boot wiring (minimal world + core commands)
//! also used by the `mushd` M0 REPL, so tests and the binary drive exactly
//! the same world.
//!
//! Rendering here ([`text_of`]) is the *plain-text test renderer*: spans are
//! flattened, narrative kinds get their classic MUSH shapes ("Alice says,
//! \"hi\"", "Alice grins."). It is deliberately simple; the real ANSI
//! renderer lands in `mush-ansi` later and owns niceties like You-vs-Name.

use std::collections::HashMap;

use mush_core::{
    Audience, Effects, Engine, GOD, GameModule, Inbound, LIMBO, ModuleBootData, ModuleOp, ObjId,
    ObjKind, Outbox, Registry, World, WorldEvent, can_perceive, event_subject,
    register_core_commands,
};
use mush_proto::{NarrativeKind, ObjRef, OutputEvent, RichText, RoomView, WhoEntry};
use smol_str::SmolStr;

/// Build the standard M0 world wiring: a minimal [`World`] (Limbo `#0` +
/// God `#1`) and a [`Registry`] with every core command registered.
///
/// Shared by [`TestWorld::new`] and the `mushd` REPL so both drive an
/// identical world.
///
/// # Panics
///
/// Panics if core command registration collides — that is a programming
/// error in `mush-core`, not a runtime condition.
///
/// ```
/// let (world, registry) = mush_testkit::boot_world();
/// assert!(world.get(mush_core::GOD).is_some());
/// assert!(registry.spec_of("look").is_some());
/// ```
#[must_use]
pub fn boot_world() -> (World, Registry) {
    let world = World::new_minimal();
    let mut registry = Registry::new();
    register_core_commands(&mut registry).expect("core command registration must not collide");
    (world, registry)
}

/// An in-process game world plus command registry, driven one input line at
/// a time. God (`#1`) is marked connected at construction.
///
/// Since M3 this wraps the real [`Engine`] (rather than raw `dispatch`), so
/// game modules plug straight in: [`TestWorld::with_modules`] registers
/// them, installs their state slots, arms their ticks, and runs a
/// first-boot [`Engine::modules_loaded`] with empty boot data — exactly the
/// server's boot order over an empty store. Module persistence ops
/// accumulate in the engine and are inspectable via
/// [`TestWorld::take_module_ops`]; timers (ticks, `@wait`) fire through
/// [`TestWorld::fire_due`].
pub struct TestWorld {
    engine: Engine,
    next_conn: u64,
}

impl TestWorld {
    /// A fresh world (Limbo + God, all core commands registered, God
    /// connected). No modules — identical surface to the M0 testkit.
    #[must_use]
    pub fn new() -> TestWorld {
        TestWorld::with_modules(Vec::new())
    }

    /// A fresh world with `modules` registered and first-booted (empty
    /// [`ModuleBootData`] — modules seed their defaults exactly as on a
    /// server's very first boot). God is connected through the engine, so
    /// module `Connected` hooks have fired for him.
    ///
    /// For reboot tests (persisted boot data), drive [`Engine`] directly:
    /// build the boot map yourself and call [`Engine::modules_loaded`].
    ///
    /// # Panics
    ///
    /// Panics if registration or module boot fails — in a test fixture that
    /// is always a bug in the test.
    #[must_use]
    pub fn with_modules(modules: Vec<Box<dyn GameModule>>) -> TestWorld {
        let (world, registry) = boot_world();
        let mut engine = Engine::with_modules(world, registry, modules)
            .expect("test module registration must not collide");
        let mut out = Outbox::new();
        let mut fx = Effects::new();
        engine
            .modules_loaded(HashMap::new(), &mut out, &mut fx)
            .expect("test module boot must not fail");
        let mut tw = TestWorld {
            engine,
            next_conn: 0,
        };
        tw.connect(GOD); // boot-time output (none expected) is discarded
        tw
    }

    /// Read access to the world, for assertions on game state.
    #[must_use]
    pub fn world(&self) -> &World {
        &self.engine.world
    }

    /// Mutable access to the world, for fixture setup that has no command
    /// surface (granting flags, planting attributes, creating props).
    pub fn world_mut(&mut self) -> &mut World {
        &mut self.engine.world
    }

    /// The command registry (for spec introspection).
    #[must_use]
    pub fn registry(&self) -> &Registry {
        &self.engine.registry
    }

    /// The engine itself, for tests that need the full surface (scheduler
    /// inspection, `modules_loaded` with real boot data, conn bookkeeping).
    #[must_use]
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Mutable engine access (see [`TestWorld::engine`]).
    pub fn engine_mut(&mut self) -> &mut Engine {
        &mut self.engine
    }

    /// Consume the harness and return its real engine for driver-level tests.
    #[must_use]
    pub fn into_engine(self) -> Engine {
        self.engine
    }

    /// Create a player named `name` in Limbo and mark them connected
    /// (through the engine, so module `ObjCreated` observers and
    /// `Connected` hooks fire — connect-time output is discarded; use
    /// [`TestWorld::connect`] on a fresh player to capture it).
    ///
    /// # Panics
    ///
    /// Panics if creation fails (e.g. the name is already taken) — in a
    /// test fixture that is always a bug in the test.
    pub fn spawn_player(&mut self, name: &str) -> ObjId {
        let id = self
            .engine
            .world
            .create(ObjKind::Player, name, GOD, Some(LIMBO))
            .unwrap_or_else(|e| panic!("spawn_player({name:?}) failed: {e}"));
        let mut out = Outbox::new();
        let mut fx = Effects::new();
        self.engine.emit_world_event(
            &WorldEvent::ObjCreated {
                obj: id,
                kind: ObjKind::Player,
                creator: GOD,
            },
            &mut out,
            &mut fx,
        );
        self.connect(id);
        id
    }

    /// Attach one fresh engine connection to `actor` (fires module
    /// `Connected` hooks on the first one) and return the emitted events —
    /// connect announcements, module login notices.
    pub fn connect(&mut self, actor: ObjId) -> Vec<(Audience, OutputEvent)> {
        self.next_conn += 1;
        let conn = self.next_conn;
        let mut out = Outbox::new();
        let mut fx = Effects::new();
        self.engine.step(
            Inbound::ConnUp {
                conn,
                actor,
                caps_structured: false,
                from_linkdead: false,
            },
            &mut out,
            &mut fx,
        );
        out.take()
    }

    /// Dispatch one input line as `actor` and return every queued
    /// `(Audience, OutputEvent)` pair, in emission order.
    pub fn run(&mut self, actor: ObjId, line: &str) -> Vec<(Audience, OutputEvent)> {
        let mut out = Outbox::new();
        let mut fx = Effects::new();
        self.engine.step(
            Inbound::Cmd {
                conn: 0,
                actor,
                line: line.to_string(),
            },
            &mut out,
            &mut fx,
        );
        out.take()
    }

    /// Feed wall time in (unix seconds) — module ticks and `@wait` due
    /// times are relative to it.
    pub fn set_now(&mut self, unix_seconds: i64) {
        self.engine.set_now(unix_seconds);
    }

    /// Fire every scheduled entry due at `now_ms` (unix milliseconds) and
    /// return the emitted events. Drive module ticks with this.
    pub fn fire_due(&mut self, now_ms: i64) -> Vec<(Audience, OutputEvent)> {
        let mut out = Outbox::new();
        let mut fx = Effects::new();
        self.engine.fire_due(now_ms, &mut out, &mut fx);
        out.take()
    }

    /// Drain the module persistence ops queued since the last call (what
    /// the server would fold into the next `WriteBatch`).
    pub fn take_module_ops(&mut self) -> Vec<ModuleOp> {
        self.engine.take_module_ops()
    }

    /// Drain the whole write batch a command produced — the dirty object set
    /// **and** the module ops — exactly as the world task folds them into one
    /// `WriteBatch` per step. The boundary rig for ledger consumers: assert
    /// that a command's world mutation and its ledger append land in the SAME
    /// drained batch, so they commit atomically (see
    /// [`WriteBatchView::asserts_atomic_with_ledger`]).
    ///
    /// Draining is destructive (it clears both the dirty set and the op
    /// queue), so call it once per command you are asserting on.
    pub fn drain_write_batch(&mut self) -> WriteBatchView {
        let dirty = self.engine.world.take_dirty();
        let module_ops = self.engine.take_module_ops();
        WriteBatchView { dirty, module_ops }
    }

    /// Hand each module its persisted boot data (reboot tests): a thin
    /// forward to [`Engine::modules_loaded`], with output discarded.
    ///
    /// # Panics
    ///
    /// Panics if a module rejects its boot data.
    pub fn modules_loaded(&mut self, boot: HashMap<SmolStr, ModuleBootData>) {
        let mut out = Outbox::new();
        let mut fx = Effects::new();
        self.engine
            .modules_loaded(boot, &mut out, &mut fx)
            .expect("module boot data must load");
    }

    /// Render exactly what `viewer` would have received from `events`,
    /// resolving audiences against current world state: their own actor
    /// events, room events for the room they are in (unless excluded), and
    /// broadcast events while they are connected. `Conn` events never
    /// deliver (there is no session layer here).
    #[must_use]
    pub fn seen_by(&self, events: &[(Audience, OutputEvent)], viewer: ObjId) -> Vec<String> {
        let location = self.world().get(viewer).and_then(|o| o.location);
        events
            .iter()
            .filter(|(aud, ev)| match aud {
                Audience::Actor(a) => *a == viewer,
                Audience::All => self.world().is_connected(viewer),
                Audience::Room {
                    room,
                    exclude,
                    subject,
                } => {
                    // M11 per-observer projection: mirror the world task's
                    // fan-out — a viewer who cannot perceive the event's
                    // subject does not see it. The subject is the one carried
                    // by the routing layer (movement/item/connect echoes) or
                    // the attributed actor of a narrative line. Free pass with
                    // no Perceive hook registered.
                    location == Some(*room)
                        && !exclude.contains(&viewer)
                        && subject.or_else(|| event_subject(ev)).is_none_or(|subj| {
                            can_perceive(&self.engine.registry, self.world(), viewer, subj)
                        })
                }
                Audience::Conn(_) => false,
            })
            .map(|(_, ev)| text_of(ev))
            .collect()
    }

    /// Assert that `viewer` received an event rendering to text containing
    /// `needle` (audiences resolved as in [`TestWorld::seen_by`]).
    ///
    /// # Panics
    ///
    /// Panics (with every event dumped) if nothing matched.
    #[track_caller]
    pub fn assert_saw(&self, events: &[(Audience, OutputEvent)], viewer: ObjId, needle: &str) {
        let seen = self.seen_by(events, viewer);
        assert!(
            seen.iter().any(|l| l.contains(needle)),
            "expected {viewer} to see {needle:?}; they saw:\n{}\nall events:\n{}",
            if seen.is_empty() {
                "  (nothing)".to_string()
            } else {
                seen.iter()
                    .map(|l| format!("  {}", l.replace('\n', " | ")))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            dump(events),
        );
    }
}

impl Default for TestWorld {
    fn default() -> Self {
        TestWorld::new()
    }
}

/// A modeled view of the single `WriteBatch` a command produced — the dirty
/// object ids the persistence layer would upsert, plus the module ops queued
/// in the same step. Because the world task folds exactly these two into one
/// batch per command, anything present here commits **atomically**. The
/// boundary rig for ledger consumers (returned by
/// [`TestWorld::drain_write_batch`]).
#[derive(Clone, Debug, Default)]
pub struct WriteBatchView {
    /// Objects mutated this command (upserted by the persistence layer).
    pub dirty: std::collections::BTreeSet<ObjId>,
    /// Module persistence ops queued this command.
    pub module_ops: Vec<ModuleOp>,
}

impl WriteBatchView {
    /// The ledger-chunk ops in this batch (`log:*` `JsonSet`s), as
    /// `(namespace, log_id, chunk)` with the decoded chunk document.
    #[must_use]
    pub fn ledger_appends(&self) -> Vec<(SmolStr, SmolStr, mush_core::LedgerChunk)> {
        self.module_ops
            .iter()
            .filter_map(|op| match &op.op {
                mush_core::ModuleWrite::JsonSet { key, value } if mush_core::is_log_key(key) => {
                    // `log:{id}:{chunk}` — the log id is everything between the
                    // prefix and the final `:` segment.
                    let rest = key.strip_prefix("log:")?;
                    let (log_id, _) = rest.rsplit_once(':')?;
                    let chunk =
                        serde_json::from_value::<mush_core::LedgerChunk>(value.clone()).ok()?;
                    Some((op.ns.clone(), SmolStr::new(log_id), chunk))
                }
                _ => None,
            })
            .collect()
    }

    /// Whether this batch appended to the given `(namespace, log_id)` — i.e.
    /// the command produced at least one ledger op for that log.
    #[must_use]
    pub fn appended_to(&self, ns: &str, log_id: &str) -> bool {
        self.ledger_appends()
            .iter()
            .any(|(n, id, _)| n == ns && id == log_id)
    }

    /// Assert the atomic pairing a ledger consumer must maintain: `obj` was
    /// mutated **and** an entry was appended to `(ns, log_id)` in this one
    /// batch — so the mutation and its record commit together or not at all.
    ///
    /// # Panics
    ///
    /// Panics (with a diagnostic) if either half is missing — which is exactly
    /// the mutate-without-recording (or record-without-mutating) divergence the
    /// ledger substrate exists to prevent.
    pub fn asserts_atomic_with_ledger(&self, obj: ObjId, ns: &str, log_id: &str) {
        let dirtied = self.dirty.contains(&obj);
        let recorded = self.appended_to(ns, log_id);
        assert!(
            dirtied && recorded,
            "write-batch boundary violated: object {obj} dirtied={dirtied}, \
             ledger {ns}:{log_id} appended={recorded} — a mutation and its \
             ledger entry must land in ONE batch (ops: {:?})",
            self.module_ops
        );
    }
}

/// Render one [`OutputEvent`] to plain text — spans flattened, narrative
/// kinds given their classic MUSH shapes. This is the test/REPL renderer;
/// `mush-ansi` owns the real one later.
#[must_use]
pub fn text_of(event: &OutputEvent) -> String {
    match event {
        OutputEvent::Room(rv) => room_text(rv),
        OutputEvent::Narrative { kind, actor, text } => narrative_text(*kind, actor.as_ref(), text),
        OutputEvent::Sys(text) => text.to_plain_text(),
        OutputEvent::Channel {
            channel,
            actor,
            text,
            ..
        } => format!("[{channel}] {}: {}", actor.name, text.to_plain_text()),
        OutputEvent::MailNotice {
            from,
            subject,
            id,
            unread,
        } => format!(
            "MAIL: message {id} from {} -- {subject} ({unread} unread)",
            from.name
        ),
        OutputEvent::Who(entries) => who_text(entries),
        OutputEvent::Reply { id, event } => format!("[reply {id}] {}", text_of(event)),
        OutputEvent::Ping { nonce } => format!("[ping {nonce}]"),
        OutputEvent::Resumed { replayed } => format!("[resumed replayed={replayed}]"),
        OutputEvent::Composing { actor, active } => {
            format!("[composing {} active={active}]", actor.name)
        }
        OutputEvent::Panel { feed, .. } => format!("[panel {feed}]"),
        OutputEvent::View(view) => format!("[view {}]", view.view()),
        OutputEvent::Weather { text, .. } => text.to_plain_text(),
        OutputEvent::Prompt(p) => p
            .text
            .as_ref()
            .map(RichText::to_plain_text)
            .unwrap_or_default(),
        OutputEvent::Hello { server, proto, .. } => format!("{server} (protocol {proto})"),
        OutputEvent::SessionInfo {
            character, account, ..
        } => format!("Connected as {} ({account})", character.name),
        OutputEvent::Disconnect { reason } => reason.to_plain_text(),
    }
}

/// The last [`RoomView`] in `events`, regardless of audience — i.e. what
/// the mover/looker most recently had rendered.
#[must_use]
pub fn last_room_view(events: &[(Audience, OutputEvent)]) -> Option<&RoomView> {
    events.iter().rev().find_map(|(_, ev)| match ev {
        OutputEvent::Room(rv) => Some(rv),
        _ => None,
    })
}

/// Assert that some event addressed to `actor` (their [`Audience::Actor`]
/// stream, or a broadcast) renders to text containing `needle`.
///
/// This is the world-free helper: room events are **not** considered even
/// if the actor is standing in the room — use [`TestWorld::assert_saw`]
/// when you want full audience resolution.
///
/// # Panics
///
/// Panics (with every event dumped) if nothing matched.
#[track_caller]
pub fn assert_actor_saw(events: &[(Audience, OutputEvent)], actor: ObjId, needle: &str) {
    let hit = events.iter().any(|(aud, ev)| {
        let addressed =
            matches!(aud, Audience::Actor(a) if *a == actor) || matches!(aud, Audience::All);
        addressed && text_of(ev).contains(needle)
    });
    assert!(
        hit,
        "expected an event to actor {actor} containing {needle:?}; events:\n{}",
        dump(events),
    );
}

/// Assert that some event addressed to `room`'s occupants (or a broadcast)
/// renders to text containing `needle`. Exclusion lists are ignored — this
/// asserts the room *was told*, not that one specific occupant heard it.
///
/// # Panics
///
/// Panics (with every event dumped) if nothing matched.
#[track_caller]
pub fn assert_room_saw(events: &[(Audience, OutputEvent)], room: ObjId, needle: &str) {
    let hit = events.iter().any(|(aud, ev)| {
        let addressed = matches!(aud, Audience::Room { room: r, .. } if *r == room)
            || matches!(aud, Audience::All);
        addressed && text_of(ev).contains(needle)
    });
    assert!(
        hit,
        "expected an event to room {room} containing {needle:?}; events:\n{}",
        dump(events),
    );
}

/// One classic narrative line: `Alice says, "hi"` / `Alice grins.` / ….
fn narrative_text(kind: NarrativeKind, actor: Option<&ObjRef>, text: &RichText) -> String {
    let name = actor.map_or("Someone", |a| a.name.as_str());
    let t = text.to_plain_text();
    match kind {
        NarrativeKind::Say => format!("{name} says, \"{t}\""),
        NarrativeKind::Pose => format!("{name} {t}"),
        NarrativeKind::SemiPose => format!("{name}{t}"),
        NarrativeKind::Emit | NarrativeKind::Spoof => t,
        NarrativeKind::Page => format!("{name} pages: {t}"),
        NarrativeKind::Whisper => format!("{name} whispers, \"{t}\""),
        NarrativeKind::Ooc => format!("<OOC> {name}: {t}"),
    }
}

/// A multi-line classic room render: name(id), desc, contents, exits, zone.
fn room_text(rv: &RoomView) -> String {
    let mut s = format!("{}({})", rv.name, rv.id);
    s.push('\n');
    s.push_str(&rv.desc.to_plain_text());
    if !rv.contents.is_empty() {
        s.push_str("\nContents:");
        for c in &rv.contents {
            s.push_str("\n  ");
            s.push_str(&c.name);
        }
    }
    if !rv.exits.is_empty() {
        let names: Vec<&str> = rv.exits.iter().map(|e| e.name.as_str()).collect();
        s.push_str("\nObvious exits: ");
        s.push_str(&names.join("  "));
    }
    if let Some(z) = &rv.zone {
        s.push_str("\nZone: ");
        s.push_str(&z.name);
    }
    s
}

/// A simple WHO table: one line per entry, then a count line.
fn who_text(entries: &[WhoEntry]) -> String {
    let mut s = String::new();
    for e in entries {
        s.push_str(&e.name);
        if let Some(loc) = &e.location {
            s.push_str(&format!(" [{loc}]"));
        }
        if let Some(d) = &e.doing {
            s.push_str(&format!(" -- {d}"));
        }
        s.push('\n');
    }
    s.push_str(&format!(
        "{} player{} connected.",
        entries.len(),
        if entries.len() == 1 { "" } else { "s" }
    ));
    s
}

/// Render an event list for assertion panic messages.
fn dump(events: &[(Audience, OutputEvent)]) -> String {
    if events.is_empty() {
        return "  (no events)".to_string();
    }
    events
        .iter()
        .map(|(aud, ev)| format!("  {:?} :: {}", aud, text_of(ev).replace('\n', " | ")))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_proto::{ExitRef, Span};

    fn say(actor: &str, text: &str) -> OutputEvent {
        OutputEvent::Narrative {
            kind: NarrativeKind::Say,
            actor: Some(ObjRef::new("#9", actor)),
            text: RichText::plain(text),
        }
    }

    #[test]
    fn narrative_shapes() {
        let cases = [
            (NarrativeKind::Say, "Alice says, \"hi\""),
            (NarrativeKind::Pose, "Alice hi"),
            (NarrativeKind::SemiPose, "Alicehi"),
            (NarrativeKind::Emit, "hi"),
            (NarrativeKind::Spoof, "hi"),
            (NarrativeKind::Page, "Alice pages: hi"),
            (NarrativeKind::Whisper, "Alice whispers, \"hi\""),
            (NarrativeKind::Ooc, "<OOC> Alice: hi"),
        ];
        let alice = ObjRef::new("#9", "Alice");
        for (kind, want) in cases {
            assert_eq!(
                narrative_text(kind, Some(&alice), &RichText::plain("hi")),
                want
            );
        }
        // Unattributed narratives fall back to "Someone".
        assert_eq!(
            narrative_text(NarrativeKind::Say, None, &RichText::plain("hi")),
            "Someone says, \"hi\""
        );
    }

    #[test]
    fn text_of_flattens_spans() {
        let ev = OutputEvent::Sys(
            RichText::tag("error", "Huh?")
                .text("  ")
                .span(Span::tagged("system", "(hint)")),
        );
        assert_eq!(text_of(&ev), "Huh?  (hint)");
    }

    #[test]
    fn room_text_renders_all_sections() {
        let rv = RoomView {
            name: "Hall".into(),
            id: "#5".into(),
            desc: RichText::plain("A big hall."),
            contents: vec![ObjRef::new("#7", "ball")],
            exits: vec![ExitRef {
                name: "north".into(),
                aliases: vec!["n".into()],
            }],
            zone: Some(ObjRef::new("#3", "Downtown")),
            ambient: Default::default(),
        };
        let t = room_text(&rv);
        assert_eq!(
            t,
            "Hall(#5)\nA big hall.\nContents:\n  ball\nObvious exits: north\nZone: Downtown"
        );
    }

    #[test]
    fn who_text_counts() {
        let e = WhoEntry {
            name: "Alice".into(),
            alias: None,
            idle_s: 0,
            conn_s: 0,
            location: Some("Limbo".into()),
            doing: Some("testing".into()),
        };
        assert_eq!(
            who_text(&[e]),
            "Alice [Limbo] -- testing\n1 player connected."
        );
        assert_eq!(who_text(&[]), "0 players connected.");
    }

    #[test]
    fn last_room_view_picks_last() {
        let a = RoomView {
            name: "A".into(),
            id: "#2".into(),
            desc: RichText::plain(""),
            contents: vec![],
            exits: vec![],
            zone: None,
            ambient: Default::default(),
        };
        let mut b = a.clone();
        b.name = "B".into();
        let events = vec![
            (Audience::Actor(GOD), OutputEvent::Room(a)),
            (Audience::Actor(GOD), OutputEvent::Sys(RichText::plain("x"))),
            (Audience::Actor(GOD), OutputEvent::Room(b)),
        ];
        assert_eq!(last_room_view(&events).unwrap().name, "B");
        assert!(last_room_view(&events[..2]).unwrap().name == "A");
        assert!(last_room_view(&events[1..2]).is_none());
    }

    #[test]
    fn seen_by_resolves_audiences() {
        let mut tw = TestWorld::new();
        let alice = tw.spawn_player("Alice");
        let bob = tw.spawn_player("Bob");
        let hall = tw
            .world_mut()
            .create(ObjKind::Room, "Hall", GOD, None)
            .unwrap();
        tw.world_mut().move_to(bob, hall).unwrap();

        let events = vec![
            (Audience::Actor(alice), say("Alice", "just for me")),
            (
                Audience::Room {
                    room: LIMBO,
                    exclude: vec![alice],
                    subject: None,
                },
                say("Alice", "limbo minus alice"),
            ),
            (Audience::All, say("Alice", "everyone")),
            (Audience::Conn(7), say("Alice", "a connection")),
        ];
        // Alice: her actor event + the broadcast; excluded from the room one.
        let seen = tw.seen_by(&events, alice);
        assert_eq!(seen.len(), 2);
        assert!(seen[0].contains("just for me"));
        assert!(seen[1].contains("everyone"));
        // Bob is in Hall, not Limbo: only the broadcast.
        assert_eq!(
            tw.seen_by(&events, bob),
            vec!["Alice says, \"everyone\"".to_string()]
        );
        // God is in Limbo and not excluded: room event + broadcast.
        assert_eq!(tw.seen_by(&events, GOD).len(), 2);
    }

    #[test]
    fn run_drains_outbox() {
        let mut tw = TestWorld::new();
        let alice = tw.spawn_player("Alice");
        let events = tw.run(alice, "think ping");
        assert_eq!(events.len(), 1);
        assert_actor_saw(&events, alice, "ping");
        // A second run starts from an empty outbox.
        let events = tw.run(alice, "think pong");
        assert_eq!(events.len(), 1);
        assert_actor_saw(&events, alice, "pong");
    }

    #[test]
    fn spawn_player_is_connected_in_limbo() {
        let mut tw = TestWorld::new();
        let alice = tw.spawn_player("Alice");
        assert!(tw.world().is_connected(alice));
        assert_eq!(tw.world().get(alice).unwrap().location, Some(LIMBO));
        assert!(tw.world().is_connected(GOD));
    }

    #[test]
    #[should_panic(expected = "expected an event to actor")]
    fn assert_actor_saw_panics_with_dump() {
        assert_actor_saw(&[], GOD, "anything");
    }
}
