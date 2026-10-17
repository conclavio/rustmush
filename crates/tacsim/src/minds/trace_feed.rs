//! Source-free trace reads and bounded following with current-world authorization.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use mush_core::output::ConnId;
use mush_core::{Audience, Ctx, ModError, ObjId, PanelFeedHandle, Registrar, World};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::arena::author_trace::{Outcome, THINKS, TRACE_VERSION, ThinkTrace};
use crate::arena_bridge::{ArenaRegistry, mind_traces};

pub(crate) const FEED: &str = "arena.mind";
const MAX_FOLLOWERS: usize = 4096;
const MAX_PAYLOAD_BYTES: usize = 60 * 1024;
const UNAVAILABLE: &str = "No mind trace is available.";
const TRACE_USAGE: &str = "@mind/trace #<unit> [1..64] [FOLLOW|ONCE]";

fn parse_line(text: &str) -> Result<(ObjId, Option<usize>, bool), String> {
    let mut words = text.split_whitespace();
    let unit = words
        .next()
        .and_then(|word| word.parse().ok())
        .ok_or(TRACE_USAGE)?;
    let mut count = None;
    let mut mode = None;
    for word in words {
        if word.eq_ignore_ascii_case("FOLLOW") || word.eq_ignore_ascii_case("ONCE") {
            if mode.replace(word.eq_ignore_ascii_case("FOLLOW")).is_some() {
                return Err(TRACE_USAGE.into());
            }
        } else {
            let value = word
                .parse::<usize>()
                .ok()
                .filter(|n| (1..=THINKS).contains(n))
                .ok_or(TRACE_USAGE)?;
            if count.replace(value).is_some() {
                return Err(TRACE_USAGE.into());
            }
        }
    }
    Ok((unit, count, mode.unwrap_or(false)))
}

/// The line adapter uses the same fresh authorization and selection as Invoke.
pub(crate) fn command(ctx: &mut Ctx<'_>, text: &str) -> Result<(), mush_core::CmdError> {
    let (unit, count, follow) = parse_line(text).map_err(mush_core::CmdError::msg)?;
    select(ctx, unit, count, follow).map_err(mush_core::CmdError::msg)?;
    let rows = mind_traces(ctx.w, ctx.actor, unit, count)
        .ok_or_else(|| mush_core::CmdError::msg(UNAVAILABLE))?;
    if rows.is_empty() {
        ctx.out
            .info(ctx.actor, "This mind has no recorded thinks yet.");
    } else {
        for line in lines(&rows) {
            ctx.out.info(ctx.actor, line);
        }
    }
    Ok(())
}

#[derive(Clone)]
struct Cursor {
    actor: ObjId,
    lease: u64,
    unit: ObjId,
    count: Option<usize>,
    last_ms: i64,
    digest: [u8; 32],
}

#[derive(Clone)]
struct Followers {
    feed: PanelFeedHandle,
    cursors: BTreeMap<ConnId, Cursor>,
}

pub(crate) fn cleared() -> Value {
    // Deliberately not retained: a session resume must query current authority.
    json!({"schema":{"id":FEED,"version":TRACE_VERSION},"available":false,"traces":[]})
}

fn payload(unit: ObjId, mut rows: Vec<ThinkTrace>) -> Value {
    let mut omitted = 0;
    loop {
        let data = json!({"schema":{"id":FEED,"version":TRACE_VERSION},
            "available":true,"unit":unit.0,"omitted_thinks":omitted,"traces":rows});
        if serde_json::to_vec(&data).is_ok_and(|bytes| bytes.len() <= MAX_PAYLOAD_BYTES) {
            return data;
        }
        if rows.is_empty() {
            return cleared();
        }
        rows.remove(0);
        omitted += 1;
    }
}

fn read(w: &World, actor: ObjId, unit: ObjId, count: Option<usize>) -> Option<Value> {
    mind_traces(w, actor, unit, count).map(|rows| payload(unit, rows))
}

fn now_ms(w: &World, unit: ObjId) -> Option<i64> {
    let registry = w.module_state::<ArenaRegistry>()?;
    Some(registry.mirror(registry.match_of_unit(unit)?)?.now_ms)
}

fn digest(data: &Value) -> [u8; 32] {
    Sha256::digest(serde_json::to_vec(data).unwrap_or_default()).into()
}

fn binding(ctx: &Ctx<'_>) -> Option<(ConnId, u64)> {
    let conn = ctx.w.dispatching_conn()?;
    let (actor, lease) = ctx.w.connection_binding(conn)?;
    (actor == ctx.actor).then_some((conn, lease))
}

fn emit(ctx: &mut Ctx<'_>, conn: ConnId, cursor: &Cursor, feed: &PanelFeedHandle, data: Value) {
    // A physical connection send bypasses the session resume ring. A reused
    // connection id cannot inherit a prior character or lifetime's selection.
    if ctx.w.connection_binding(conn) == Some((cursor.actor, cursor.lease)) {
        feed.keyframe_to(ctx, Audience::Conn(conn), data);
    }
}

fn clear(ctx: &mut Ctx<'_>) {
    let Some((conn, lease)) = binding(ctx) else {
        return;
    };
    let actor = ctx.actor;
    let removed = ctx.w.module_state_mut::<Followers>().and_then(|state| {
        state
            .cursors
            .remove(&conn)
            .map(|cursor| (state.feed.clone(), cursor))
    });
    if let Some((feed, cursor)) = removed
        && cursor.actor == actor
        && cursor.lease == lease
    {
        emit(ctx, conn, &cursor, &feed, cleared());
    }
}

/// Selectors carry no authority. Refusal also revokes an earlier selection.
pub(crate) fn select(
    ctx: &mut Ctx<'_>,
    unit: ObjId,
    count: Option<usize>,
    follow: bool,
) -> Result<Value, String> {
    let (conn, lease) = binding(ctx).ok_or(UNAVAILABLE)?;
    if count.is_some_and(|n| !(1..=THINKS).contains(&n)) {
        clear(ctx);
        return Err(format!("Trace count must be between 1 and {THINKS}."));
    }
    let Some(data) = read(ctx.w, ctx.actor, unit, count) else {
        clear(ctx);
        return Err(UNAVAILABLE.into());
    };
    let at_ms = now_ms(ctx.w, unit).unwrap_or(0);
    let actor = ctx.actor;
    let cursor = Cursor {
        actor,
        lease,
        unit,
        count,
        last_ms: at_ms,
        digest: digest(&data),
    };
    let state = ctx.w.module_state_mut::<Followers>().ok_or(UNAVAILABLE)?;
    if follow {
        if !state.cursors.contains_key(&conn) && state.cursors.len() >= MAX_FOLLOWERS {
            return Err("Mind trace followers are full; read without following.".into());
        }
        state.cursors.insert(conn, cursor.clone());
    } else {
        state.cursors.remove(&conn);
    }
    let feed = state.feed.clone();
    emit(ctx, conn, &cursor, &feed, data.clone());
    Ok(data)
}

struct Query;
impl mush_core::QueryHandler for Query {
    fn query(
        &self,
        w: &World,
        actor: ObjId,
        _params: &Value,
    ) -> Result<mush_proto::OutputEvent, mush_proto::RichText> {
        // Queries do not carry a physical connection context. Hydrate only
        // the current seat; FOLLOW selectors are never borrowed from another
        // connection of this character. Explicit selects use the typed invoke.
        let selection = w
            .module_state::<ArenaRegistry>()
            .and_then(|registry| registry.seat_of_pilot(actor))
            .map(|(_, unit)| (unit, None));
        let data = selection
            .and_then(|(unit, count)| read(w, actor, unit, count))
            .unwrap_or_else(cleared);
        Ok(mush_proto::OutputEvent::Panel {
            feed: FEED.into(),
            kind: mush_proto::PanelFrameKind::Keyframe,
            data,
        })
    }
}

fn tick(ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<mush_core::SceneId>) {
    let Some((feed, cursors)) = ctx
        .w
        .module_state::<Followers>()
        .map(|state| (state.feed.clone(), state.cursors.clone()))
    else {
        return;
    };
    for (conn, cursor) in cursors {
        if ctx.w.connection_binding(conn) != Some((cursor.actor, cursor.lease)) {
            if let Some(state) = ctx.w.module_state_mut::<Followers>() {
                state.cursors.remove(&conn);
            }
            continue;
        }
        let actor = cursor.actor;
        // ACL is checked even when simulation time is paused or unchanged.
        let data = read(ctx.w, actor, cursor.unit, cursor.count);
        let Some(data) = data else {
            if let Some(state) = ctx.w.module_state_mut::<Followers>() {
                state.cursors.remove(&conn);
            }
            if ctx.w.get(actor).is_some() {
                emit(ctx, conn, &cursor, &feed, cleared());
            }
            continue;
        };
        let at_ms = now_ms(ctx.w, cursor.unit).unwrap_or(cursor.last_ms);
        if at_ms.saturating_sub(cursor.last_ms) < 1_000 {
            continue;
        }
        let next_digest = digest(&data);
        if next_digest == cursor.digest {
            continue;
        }
        if let Some(current) = ctx
            .w
            .module_state_mut::<Followers>()
            .and_then(|state| state.cursors.get_mut(&conn))
        {
            current.last_ms = at_ms;
            current.digest = next_digest;
        }
        emit(ctx, conn, &cursor, &feed, data);
    }
}

pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    let feed = reg.panel_feed(FEED)?;
    reg.state(Followers {
        feed,
        cursors: BTreeMap::new(),
    });
    reg.query_handler(FEED, Arc::new(Query))?;
    reg.tick(
        "mind-traces",
        mush_core::TickCadence::Coarse(Duration::from_secs(1)),
        Arc::new(tick),
    )
}

fn outcome(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Pending => "pending".into(),
        Outcome::Applied => "applied".into(),
        Outcome::Refused { reason } => format!("refused: {reason}"),
        Outcome::Faulted { fault } => format!("fault: {fault:?}"),
        Outcome::Cancelled { reason } => format!("cancelled: {reason:?}"),
    }
}

/// Bounded terminal projection of the same public records the feed carries.
pub(crate) fn lines(rows: &[ThinkTrace]) -> Vec<String> {
    if rows.is_empty() {
        return vec!["No thinks have been recorded yet.".into()];
    }
    let mut lines = Vec::new();
    for row in rows {
        let cost = row.diagnostics.counts().map_or_else(
            || "unmetered native ROM".into(),
            |(ops, fees)| format!("{ops} ops, {fees} fees"),
        );
        lines.push(format!(
            "Think {} at {} ms: {}; {cost}{}.",
            row.think_seq,
            row.t_ms,
            outcome(&row.outcome),
            if row.rom { "; ROM" } else { "" }
        ));
        if let Some(at) = row.landed_ms {
            lines.push(format!("  Landing: {at} ms."));
        }
        if let Some(reason) = row.rom_drop {
            lines.push(format!("  ROM drop: {reason:?}."));
        }
        for intent in &row.intents {
            lines.push(format!(
                "  {}{}: {}.",
                intent.action,
                intent
                    .detail
                    .as_ref()
                    .map_or(String::new(), |text| format!(" {text}")),
                outcome(&intent.outcome)
            ));
        }
        if let Ok(diag) = serde_json::to_value(&row.diagnostics) {
            if let Some(kernels) = diag.get("kernels").and_then(serde_json::Value::as_array) {
                for kernel in kernels {
                    lines.push(format!(
                        "  Kernel {}: {} fees.",
                        kernel
                            .get("kernel")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("?"),
                        kernel.get("fee").unwrap_or(&serde_json::Value::Null)
                    ));
                }
            }
            if let Some(rules) = diag.get("rules").and_then(serde_json::Value::as_array) {
                for rule in rules {
                    lines.push(format!(
                        "  Rule {}: {} = {}.",
                        rule.get("slot").unwrap_or(&serde_json::Value::Null),
                        rule.get("description")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("?"),
                        rule.get("matches").unwrap_or(&serde_json::Value::Null)
                    ));
                    if let Some(terms) = rule.get("terms").and_then(serde_json::Value::as_array) {
                        for term in terms {
                            lines.push(format!(
                                "    {} = {}.",
                                term.get("description")
                                    .and_then(serde_json::Value::as_str)
                                    .unwrap_or("?"),
                                term.get("value").unwrap_or(&serde_json::Value::Null)
                            ));
                        }
                    }
                }
            }
            if let Some(omitted) = diag.get("omitted").and_then(serde_json::Value::as_object) {
                for (kind, n) in omitted {
                    if n.as_u64().is_some_and(|n| n > 0) {
                        lines.push(format!("  Omitted {kind}: {n}."));
                    }
                }
            }
        }
        lines.extend(row.author.iter().map(|text| format!("  Trace: {text}")));
    }
    let mut bytes = 0;
    let all = lines.len();
    let keep = lines
        .iter()
        .take(199)
        .take_while(|line| {
            bytes += line.len();
            bytes <= 39 * 1024
        })
        .count();
    lines.truncate(keep);
    if keep < all {
        lines.push(format!(
            "{} more trace lines omitted; request fewer thinks for full detail.",
            all - keep
        ));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_selectors_are_bounded_and_never_accept_command_text() {
        assert_eq!(parse_line("#42"), Ok((ObjId(42), None, false)));
        assert_eq!(parse_line("#42 64 FOLLOW"), Ok((ObjId(42), Some(64), true)));
        assert_eq!(parse_line("#42 once 1"), Ok((ObjId(42), Some(1), false)));
        for text in [
            "42",
            "#42 0",
            "#42 65",
            "#42 1 2",
            "#42 FOLLOW ONCE",
            "#42;@destroy",
            "#42 junk",
        ] {
            assert!(parse_line(text).is_err(), "{text}");
        }
    }

    fn rig() -> mush_core::Engine {
        let mut world = World::new_minimal();
        let mut registry = mush_core::Registry::new();
        let mut registrar = Registrar::new(&mut registry, crate::store::NS);
        let feed = registrar.panel_feed(FEED).unwrap();
        world.module_state_insert(Followers {
            feed,
            cursors: BTreeMap::new(),
        });
        mush_core::Engine::new(world, registry)
    }
    fn attach(engine: &mut mush_core::Engine, conn: ConnId, actor: ObjId) {
        engine.step(
            mush_core::Inbound::ConnUp {
                conn,
                actor,
                caps_structured: true,
                from_linkdead: false,
            },
            &mut mush_core::Outbox::new(),
            &mut mush_core::Effects::new(),
        );
    }
    fn context(
        engine: &mut mush_core::Engine,
        actor: ObjId,
        run: impl FnOnce(&mut Ctx<'_>),
    ) -> mush_core::Outbox {
        let mut out = mush_core::Outbox::new();
        run(&mut Ctx {
            w: &mut engine.world,
            actor,
            switches: mush_core::SwitchSet::default(),
            args: mush_core::ParsedArgs::default(),
            out: &mut out,
            reg: &engine.registry,
            module: Some(crate::store::NS.into()),
            typed_state: mush_core::ModuleActionState::unavailable(),
        });
        out
    }
    fn cursor(engine: &mush_core::Engine, conn: ConnId, actor: ObjId) -> Cursor {
        Cursor {
            actor,
            lease: engine.world.connection_binding(conn).unwrap().1,
            unit: ObjId(999),
            count: None,
            last_ms: 0,
            digest: [0; 32],
        }
    }

    #[test]
    fn m6_trace_feed_revocation_clears_only_the_bound_connection() {
        let mut engine = rig();
        attach(&mut engine, 7, mush_core::GOD);
        attach(&mut engine, 8, mush_core::GOD);
        let cursor = cursor(&engine, 7, mush_core::GOD);
        let feed = engine
            .world
            .module_state::<Followers>()
            .unwrap()
            .feed
            .clone();
        // Public rows already authorized by read() go only to the selecting
        // connection, even when the character has another live connection.
        let out = context(&mut engine, mush_core::GOD, |ctx| {
            emit(
                ctx,
                7,
                &cursor,
                &feed,
                json!({"available":true,"traces":[]}),
            );
        });
        assert!(matches!(
            out.events(),
            [(Audience::Conn(7), mush_proto::OutputEvent::Panel { .. })]
        ));
        engine
            .world
            .module_state_mut::<Followers>()
            .unwrap()
            .cursors
            .insert(7, cursor);
        // The unit is no longer authorized/available. Revocation is evaluated
        // before the time throttle, and emits only a non-retained clear.
        let out = context(&mut engine, mush_core::GOD, |ctx| tick(ctx, "", None));
        assert!(
            matches!(out.events(), [(Audience::Conn(7), mush_proto::OutputEvent::Panel { data, .. })] if data["available"] == false)
        );
        assert!(
            engine
                .world
                .module_state::<Followers>()
                .unwrap()
                .cursors
                .is_empty()
        );
    }

    #[test]
    fn m6_trace_follow_is_not_inherited_by_reconnect_or_character_switch() {
        for different_character in [false, true] {
            let mut engine = rig();
            attach(&mut engine, 7, mush_core::GOD);
            let prior = cursor(&engine, 7, mush_core::GOD);
            engine
                .world
                .module_state_mut::<Followers>()
                .unwrap()
                .cursors
                .insert(7, prior.clone());
            engine.step(
                mush_core::Inbound::ConnDown {
                    conn: 7,
                    actor: mush_core::GOD,
                    reason: mush_core::DisconnectKind::Lost,
                },
                &mut mush_core::Outbox::new(),
                &mut mush_core::Effects::new(),
            );
            let next = if different_character {
                engine
                    .world
                    .create(
                        mush_core::ObjKind::Player,
                        "Other",
                        mush_core::GOD,
                        Some(mush_core::LIMBO),
                    )
                    .unwrap()
            } else {
                mush_core::GOD
            };
            attach(&mut engine, 7, next);
            assert_ne!(
                engine.world.connection_binding(7),
                Some((prior.actor, prior.lease))
            );
            let out = context(&mut engine, next, |ctx| tick(ctx, "", None));
            assert!(
                out.events().is_empty(),
                "neither old rows nor old clears cross a connection lifetime"
            );
            assert!(
                engine
                    .world
                    .module_state::<Followers>()
                    .unwrap()
                    .cursors
                    .is_empty()
            );
            // A query cannot resurrect another physical connection's selector.
            let data =
                mush_core::QueryHandler::query(&Query, &engine.world, next, &Value::Null).unwrap();
            assert!(
                matches!(data, mush_proto::OutputEvent::Panel { data, .. } if data["available"] == false)
            );
        }
    }
}
