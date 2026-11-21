//! The onboarding gate: hold a freshly-created, not-yet-approved character to a
//! chargen command set until staff approve it (the `chargen_required` flow),
//! greet them on connect, place them in the chargen area, and lift the gate —
//! moving them to the live start room — on approval.
//!
//! Ported near-verbatim from `ruleset-wod`'s, over one [`SheetStore`] instead of
//! a template dispatch, and with the two things the game does not have removed:
//! `+spec` (no specialties) and the connect-time drop into the wizard (no
//! wizard). The three seams are the same:
//!
//! - a [`CommandGate`] ([`OnboardingGate`]) consulted in dispatch, before the
//!   comsys alias stage, so channel *speech* is gated too;
//! - a `Connected` [`EventHook`] ([`OnboardingHook`]) that greets a gated player
//!   and points at `+chargen`;
//! - an `ObjCreated` [`WorldObserver`] ([`OnboardingSpawn`]) that places a new
//!   Player in `start_room_unapproved`.
//!
//! Everything here is **inert unless `chargen_required` is on**, which is the
//! default — a server that never configures `[onboarding]` is byte-identical to
//! one built without this file.

use std::sync::Arc;

use mush_core::{
    CommandGate, Ctx, EventHook, ObjId, ObjKind, OnboardingConfig, Registry, Role, World,
    WorldEvent, WorldObserver, perm,
};
use mush_ruleset::schema::SheetSchema;
use mush_ruleset::{ApprovalState, SheetStore};

/// Whether `verb` is on the onboarding allowlist — the commands an unapproved,
/// gated character may still run. Everything else (exit names, `home`, building,
/// rolling, movement onto the grid) is refused. The gate runs **before** the
/// comsys alias stage, so channel speech (a per-player alias like `pub Hello`)
/// reaches it as its alias verb, which is not listed here, and is refused. The
/// channel *management* verbs stay listed so a waiting player can still
/// `comlist`/`addcom`.
#[must_use]
pub fn is_onboarding_verb(verb: &str) -> bool {
    const ALLOWED: &[&str] = &[
        // Perception + basic room roleplay in the chargen area.
        "look", "l", "say", "pose", "semipose", "think", // Help + who + leaving.
        "help", "+help", "who", "quit", // Character generation.
        "+chargen", "+stat", "+sheet",
        // Channel management (join/list); speaking on a channel is gated.
        "addcom", "delcom", "comlist", "comtitle", "clist",
    ];
    let verb = verb.to_ascii_lowercase();
    ALLOWED.iter().any(|allowed| *allowed == verb)
}

/// Whether `name` is on the onboarding query allowlist — the read-only client
/// hydration queries an unapproved, gated character still needs during chargen
/// (core `"room"` attach-look and `"who"` list, command palette, their own
/// pilot sheet, and joined channel list). Every live-grid/TacSim/arena/market
/// query is refused until approval.
#[must_use]
pub fn is_onboarding_query(name: &str) -> bool {
    const ALLOWED_QUERIES: &[&str] = &[
        "room",
        "who",
        "commands.list",
        "tacsim.sheet",
        "comsys.state",
    ];
    let name = name.to_ascii_lowercase();
    ALLOWED_QUERIES.iter().any(|allowed| *allowed == name)
}

/// The message shown when the gate refuses a command.
fn refusal() -> String {
    "You must finish creating your character and have it approved before you can do that. \
     Type +chargen to keep building; look, say, pose, and help still work while you wait."
        .to_string()
}

/// Whether `actor`'s sheet is Approved (an absent sheet is *not* approved).
fn is_approved(schema: &Arc<SheetSchema>, w: &World, actor: ObjId) -> bool {
    SheetStore::new(Arc::clone(schema))
        .get(w, actor)
        .is_some_and(|doc| matches!(doc.approved, ApprovalState::Approved { .. }))
}

/// Whether the onboarding restriction applies to `actor` right now: gating is
/// on, `actor` is a plain (non-staff) Player, and their sheet is not Approved.
fn gated(cfg: &OnboardingConfig, schema: &Arc<SheetSchema>, w: &World, actor: ObjId) -> bool {
    cfg.chargen_required
        && w.get(actor).is_some_and(|o| o.kind == ObjKind::Player)
        && !perm::has_role(w, actor, Role::Builder)
        && !is_approved(schema, w, actor)
}

/// The dispatch-level gate (treaty ruling 5's pre-command seam).
pub struct OnboardingGate {
    cfg: OnboardingConfig,
    schema: Arc<SheetSchema>,
}

impl OnboardingGate {
    /// The gate for `cfg` over `schema`.
    #[must_use]
    pub fn new(cfg: OnboardingConfig, schema: Arc<SheetSchema>) -> OnboardingGate {
        OnboardingGate { cfg, schema }
    }
}

impl CommandGate for OnboardingGate {
    fn check(
        &self,
        w: &World,
        _reg: &Registry,
        actor: ObjId,
        verb: &str,
        line: &str,
    ) -> Result<(), String> {
        if !gated(&self.cfg, &self.schema, w, actor) {
            return Ok(());
        }
        // Movement onto the live grid is the gate's central block. The verb
        // allowlist alone does not imply "no exit will match": dispatch matches
        // exits (room -> zone -> global exits on Limbo) after this gate and
        // against the whole line, so a global or zone exit whose name or alias
        // collides with an allowlisted verb (`l`, `look`, `say`, ...) would slip
        // through. Refuse any line that resolves to an exit.
        if mush_core::commands::movement::find_exit(w, actor, line).is_some() {
            return Err(refusal());
        }
        // Engine-level query gating passes synthetic `@query <name>` lines.
        // Only the onboarding-safe hydration queries pass; every live-grid,
        // TacSim, arena, and market query is refused until approval.
        if verb == "@query" {
            if let Some(query_name) = line.strip_prefix("@query ")
                && !query_name.contains(char::is_whitespace)
                && is_onboarding_query(query_name)
            {
                return Ok(());
            }
            return Err(refusal());
        }
        if !is_onboarding_verb(verb) {
            return Err(refusal());
        }
        Ok(())
    }
}

/// The `Connected` hook: greet a gated character and point them at `+chargen`.
///
/// The WoD twin pushed its guided wizard here. There is no wizard — the flow is
/// three commands — so this stays a greeting, and it still refuses to paint over
/// a live input context (a reconnect into an open prompt).
pub struct OnboardingHook {
    cfg: OnboardingConfig,
    schema: Arc<SheetSchema>,
}

impl OnboardingHook {
    /// The hook for `cfg` over `schema`.
    #[must_use]
    pub fn new(cfg: OnboardingConfig, schema: Arc<SheetSchema>) -> OnboardingHook {
        OnboardingHook { cfg, schema }
    }
}

impl EventHook for OnboardingHook {
    fn connected(&self, ctx: &mut Ctx<'_>, actor: ObjId) {
        if !gated(&self.cfg, &self.schema, ctx.w, actor) {
            return;
        }
        if ctx.w.input_depth(actor) > 0 {
            return;
        }
        // A submitted (Pending) sheet is locked awaiting a storyteller — say so
        // rather than inviting them back into chargen.
        let approved = SheetStore::new(Arc::clone(&self.schema))
            .get(ctx.w, actor)
            .map(|doc| doc.approved);
        if let Some(ApprovalState::Pending(job)) = approved {
            ctx.out.info(
                actor,
                format!(
                    "Your character is submitted and awaiting approval (job {job}). \
                     A storyteller will review it soon."
                ),
            );
            return;
        }
        ctx.out.info(
            actor,
            "Welcome! Type +chargen to finish creating your character: spend your pilot \
             ratings with +stat, then +chargen/submit.",
        );
    }
}

/// The `ObjCreated` observer: place a new unapproved Player in the chargen area.
pub struct OnboardingSpawn {
    cfg: OnboardingConfig,
}

impl OnboardingSpawn {
    /// The observer for `cfg`.
    #[must_use]
    pub fn new(cfg: OnboardingConfig) -> OnboardingSpawn {
        OnboardingSpawn { cfg }
    }
}

impl WorldObserver for OnboardingSpawn {
    fn on_event(&self, ctx: &mut Ctx<'_>, ev: &WorldEvent) {
        if !self.cfg.chargen_required {
            return;
        }
        let WorldEvent::ObjCreated { obj, kind, .. } = ev else {
            return;
        };
        if *kind != ObjKind::Player {
            return;
        }
        let dest = self.cfg.start_room_unapproved;
        if ctx.w.get(dest).is_none() {
            return;
        }
        if ctx.w.get(*obj).and_then(|o| o.location) == Some(dest) {
            return;
        }
        let _ = ctx.w.move_to(*obj, dest);
    }
}

/// Lift the gate on approval: move `target` to the live start room (if
/// configured and not already there) and re-render it. Called by the
/// [`pilot.chargen`](super::chargen::CHARGEN_HANDLER) job handler after the
/// sheet is stamped Approved. A no-op when `chargen_required` is off.
pub fn on_approved(cfg: &OnboardingConfig, ctx: &mut Ctx<'_>, target: ObjId) {
    if !cfg.chargen_required {
        return;
    }
    let dest = cfg.start_room;
    let at_dest = ctx.w.get(target).and_then(|o| o.location) == Some(dest);
    let dest_exists = ctx.w.get(dest).is_some();
    // Misconfiguration guard: `start_room` points at a room that no longer
    // exists and the character is not already there, so the post-approval move
    // cannot happen — they stay in the (possibly exit-less) chargen area even
    // though we are about to tell them they have full access. Surface it to
    // staff via the log rather than stranding them silently; the sheet is
    // already stamped Approved by the caller, so we cannot un-approve here.
    if !dest_exists && !at_dest {
        tracing::warn!(
            %target,
            start_room = %dest,
            "onboarding: start_room does not exist; approved character was not relocated \
             out of the chargen area — check [onboarding].start_room"
        );
    }
    let moved = dest_exists && !at_dest && ctx.w.move_to(target, dest).is_ok();
    ctx.out.info(
        target,
        "Your character has been approved — welcome to the game! You now have full access.",
    );
    // Show them their new surroundings. **Render the room directly** rather than
    // re-dispatching `"look"` (08j S2): a re-dispatch from inside a handler has
    // no lens to hand `mush_core::dispatch` but a detached one, so the whole
    // perception chain would run against an explicitly unavailable typed reader.
    // `Ctx::show_room` is the one room-render path and carries this command's
    // live snapshot. The empty-input-stack guard stays: painting a room over an
    // open prompt is wrong even when it cannot be swallowed as body text.
    if moved && ctx.w.input_depth(target) == 0 {
        ctx.show_room(target, dest);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_allowlist_carries_chargen_and_not_the_grid() {
        for verb in ["+chargen", "+stat", "+sheet", "look", "say", "help", "quit"] {
            assert!(is_onboarding_verb(verb), "{verb} should be allowed");
        }
        assert!(is_onboarding_verb("LOOK"), "the test is case-insensitive");
        for verb in [
            "+roll",
            "@dig",
            "home",
            "page",
            "+spec",
            "@invoke",
            "@legal_action",
        ] {
            assert!(!is_onboarding_verb(verb), "{verb} should be gated");
        }
    }

    #[test]
    fn the_query_allowlist_carries_chargen_and_core_read_only_queries() {
        for query in [
            "room",
            "who",
            "commands.list",
            "tacsim.sheet",
            "comsys.state",
        ] {
            assert!(is_onboarding_query(query), "{query} should be allowed");
        }
        assert!(
            is_onboarding_query("ROOM"),
            "query check is case-insensitive"
        );
        assert!(
            is_onboarding_query("WHO"),
            "query check is case-insensitive"
        );
        for query in [
            "tacsim.content",
            "arena.map",
            "mind.market",
            "mind.holdings",
            "tactics.contacts",
            "port.trade",
            "comsys.backlog",
        ] {
            assert!(!is_onboarding_query(query), "{query} should be gated");
        }
    }

    #[test]
    fn onboarding_gate_blocks_invokes_live_queries_and_legal_actions_until_approved() {
        use mush_core::action::{PerceivedRef, PrincipalId};
        use mush_core::{Effects, Engine, Inbound, Outbox};
        use mush_proto::{InvokeRequest, MindRequest, OutputEvent};

        let cfg = OnboardingConfig {
            chargen_required: true,
            start_room: mush_core::LIMBO,
            start_room_unapproved: mush_core::LIMBO,
        };
        let schema = Arc::new(crate::sheet::schema::sheet_schema());
        let mut world = World::new_minimal();
        let nick = world
            .create(
                ObjKind::Player,
                "Nick",
                mush_core::GOD,
                Some(mush_core::LIMBO),
            )
            .expect("player");
        let mut reg = Registry::new();
        reg.add_command_gate(Arc::new(OnboardingGate::new(cfg, Arc::clone(&schema))));
        struct DummyQuery;
        impl mush_core::QueryHandler for DummyQuery {
            fn query(
                &self,
                _w: &World,
                _actor: ObjId,
                _params: &serde_json::Value,
            ) -> Result<OutputEvent, mush_proto::RichText> {
                Ok(OutputEvent::Sys(mush_proto::RichText::plain("ok")))
            }
        }
        struct DummyInvoke;
        impl mush_core::module::InvokeHandler for DummyInvoke {
            fn invoke(
                &self,
                _ctx: &mut Ctx<'_>,
                _conn: mush_core::output::ConnId,
                _id: u64,
                request: &InvokeRequest,
            ) -> mush_proto::InvokeResult {
                request.invalid()
            }
        }
        {
            let mut r = mush_core::Registrar::new(&mut reg, "test");
            r.query_handler("room", Arc::new(DummyQuery)).expect("room");
            r.query_handler("who", Arc::new(DummyQuery)).expect("who");
            r.query_handler("tacsim.content", Arc::new(DummyQuery))
                .expect("tacsim.content");
            r.query_handler("arena.map", Arc::new(DummyQuery))
                .expect("arena.map");
            r.query_handler("tacsim.sheet", Arc::new(DummyQuery))
                .expect("tacsim.sheet");
            r.query_handler("commands.list", Arc::new(DummyQuery))
                .expect("commands.list");
            r.query_handler("comsys.state", Arc::new(DummyQuery))
                .expect("comsys.state");
            r.invoke_handler("mind", Arc::new(DummyInvoke))
                .expect("mind");
        }
        let mut engine = Engine::new(world, reg);
        engine.step(
            Inbound::ConnUp {
                conn: 1,
                actor: nick,
                caps_structured: true,
                from_linkdead: false,
            },
            &mut Outbox::new(),
            &mut Effects::new(),
        );

        // 1. Pre-onboarding player is blocked from `mind` Invoke.
        let mut out = Outbox::new();
        engine.step(
            Inbound::Invoke {
                conn: 1,
                actor: nick,
                id: 10,
                request: InvokeRequest::Mind(MindRequest::List {}),
            },
            &mut out,
            &mut Effects::new(),
        );
        assert!(
            matches!(
                out.events(),
                [(
                    _,
                    OutputEvent::InvokeResult {
                        id: 10,
                        result: mush_proto::InvokeResult::Mind(mush_proto::MindResponse::Error {
                            code,
                            ..
                        }),
                    }
                )] if code == "unavailable"
            ),
            "expected unavailable refusal for gated invoke: {:?}",
            out.events()
        );

        // 2. Pre-onboarding player is blocked from live-grid/TacSim/arena queries,
        //    while core read-only (`room`, `who`) and chargen hydration queries pass.
        for blocked in ["tacsim.content", "arena.map"] {
            let mut out = Outbox::new();
            engine.step(
                Inbound::Query {
                    conn: 1,
                    actor: nick,
                    id: 11,
                    name: blocked.into(),
                    args: serde_json::json!({}),
                },
                &mut out,
                &mut Effects::new(),
            );
            assert!(
                matches!(
                    out.events(),
                    [(
                        _,
                        OutputEvent::Reply {
                            id: 11,
                            event,
                        }
                    )] if matches!(event.as_ref(), OutputEvent::Sys(rt) if rt.to_plain_text().contains("finish creating your character"))
                ),
                "expected onboarding refusal for `{blocked}` query: {:?}",
                out.events()
            );
        }

        for allowed in [
            "room",
            "who",
            "tacsim.sheet",
            "commands.list",
            "comsys.state",
        ] {
            let mut out = Outbox::new();
            engine.step(
                Inbound::Query {
                    conn: 1,
                    actor: nick,
                    id: 12,
                    name: allowed.into(),
                    args: serde_json::json!({}),
                },
                &mut out,
                &mut Effects::new(),
            );
            assert!(
                matches!(
                    out.events(),
                    [(
                        _,
                        OutputEvent::Reply {
                            id: 12,
                            event,
                        }
                    )] if matches!(event.as_ref(), OutputEvent::Sys(rt) if rt.to_plain_text() == "ok")
                ),
                "allowed query `{allowed}` should pass onboarding gate: {:?}",
                out.events()
            );
        }

        // 3. Pre-onboarding player is blocked from `legal_action_query` (and `project_legal_actions`).
        assert!(
            engine
                .legal_action_query(
                    PrincipalId::Object { id: nick },
                    nick,
                    PerceivedRef::SelfActor,
                )
                .is_err(),
            "gated player must be refused legal_action_query"
        );
        let direct_query = mush_core::LegalActionQuery::new(
            PrincipalId::Object { id: nick },
            nick,
            mush_core::SubjectKey::Object { id: nick },
            PerceivedRef::SelfActor,
            1,
            None,
        )
        .expect("direct query");
        let projected = engine
            .project_legal_actions(&direct_query)
            .expect("empty sink for gated actor");
        assert!(projected.is_empty(), "gated actor projects zero offers");

        // 4. Staff (God) is never gated by onboarding.
        assert!(
            engine
                .legal_action_query(
                    PrincipalId::Object { id: mush_core::GOD },
                    mush_core::GOD,
                    PerceivedRef::SelfActor,
                )
                .is_ok(),
            "staff must not be gated"
        );

        // 5. Approving the sheet lifts the gate for queries, invokes, and legal actions.
        let store = SheetStore::new(Arc::clone(&schema));
        let mut doc = store.get_or_default(&engine.world, nick);
        doc.approved = ApprovalState::Approved {
            by: mush_core::GOD,
            at: 1,
        };
        store
            .put(&mut engine.world, nick, &doc)
            .expect("approve sheet");

        assert!(
            engine
                .legal_action_query(
                    PrincipalId::Object { id: nick },
                    nick,
                    PerceivedRef::SelfActor,
                )
                .is_ok(),
            "approved player passes legal_action_query"
        );
        let mut out = Outbox::new();
        engine.step(
            Inbound::Query {
                conn: 1,
                actor: nick,
                id: 13,
                name: "tacsim.content".into(),
                args: serde_json::json!({}),
            },
            &mut out,
            &mut Effects::new(),
        );
        assert!(
            matches!(
                out.events(),
                [(
                    _,
                    OutputEvent::Reply { id: 13, event }
                )] if matches!(event.as_ref(), OutputEvent::Sys(rt) if rt.to_plain_text() == "ok")
            ),
            "approved player passes tacsim.content query: {:?}",
            out.events()
        );
    }
}
