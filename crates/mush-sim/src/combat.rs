//! # The tactical command surface — E7 cut 8 B6 (10b §3.8)
//!
//! `+fire` · `+launch` · `+heading` · `+thrust` · `+posture` · `+dc` · `+dock` ·
//! `+eject`, plus the immediate deterministic fallback controller (C30A under
//! K7.0's rule).
//!
//! ## Every verb is a selector, never a mechanic
//!
//! A verb does exactly three things: resolve the actor's **focused** station
//! (cut 5), build one `TacticalIntentDoc`, and stage this module's own
//! `tactical_intent` action through cut 2's `Ctx::submit_authoritative` seam.
//! It writes nothing itself. The intent joins the tick's frozen
//! `ActionWindowRecord`; the deterministic commit that resolves it is the
//! `domaintick` schedule source's, fired by the kernel.
//!
//! ## Where these verbs live, and why
//!
//! 10b §3.8 puts them in `mush-tactics`, beside the `TacticalUnit` family they
//! were to mutate. **That family's migration is deferred to E7k** on cut 6's
//! `PlacementRecord` precedent (§3.8 "As built"), so what a combat verb actually
//! needs is the *sim* side: the unit's domain, its `DomainRecord`'s pulse, the
//! actor's focused station, its channel and its control-lease epoch — all of it
//! `mush-sim`'s. Registering them here therefore adds **no crate edge at all**
//! (`cargo tree -p mush-tactics` is byte-identical to cut 7's), where placing
//! them in `mush-tactics` would have required a new `mush-tactics → mush-sim`
//! edge for types the verbs only read. They move with the unit family when E7k
//! lands it.
//!
//! ## The one thing a verb may not leak
//!
//! `+fire <contact>` names an **opaque** `ContactId` out of the leak-safe
//! `ContactLedgerState` projection. A contact the actor does not hold and one
//! that never existed produce the **byte-identical** refusal
//! [`tick::no_such_contact`](crate::tick::no_such_contact) — the verb never
//! reveals which (§4 rule 4; cut 7's `NO_SUCH_CONTACT`).

use std::sync::Arc;

use mush_core::{
    ArgShape, CmdError, CommandSpec, ContactId, ContactLedgerState, Ctx, IdempotencyKeyText,
    ModError, ObjId, PerceivedRef, Registrar,
};
use serde_json::json;
use smol_str::SmolStr;

use crate::data::{DomainId, SpatialState, StationId};
use crate::tick::{
    IntentKind, OrderValue, TacticalIntentDoc, intent_action_ref, no_such_contact, pulse_clock,
};

/// The most bytes a bay, section or dock token may carry.
pub const MAX_SLOT_TOKEN_BYTES: usize = 32;

/// One registered verb: its name, argument shape, help and handler.
type Verb = (
    &'static str,
    ArgShape,
    &'static str,
    fn(&mut Ctx<'_>) -> Result<(), CmdError>,
);

fn info(ctx: &mut Ctx<'_>, line: impl Into<String>) {
    crate::commands::info(ctx, line);
}

/// Everything a verb must know before it can build an intent.
struct Seat {
    unit: ObjId,
    station: StationId,
    channel: String,
    epoch: u64,
    domain: DomainId,
    tick: i64,
    generation: u64,
}

/// Resolve the actor's live tactical seat, or the exact reason it refuses.
///
/// Every arm is a refusal a player can act on: not focused, the unit is not in a
/// domain, the domain is not pulsing, or the station grants no single channel.
fn seat(ctx: &Ctx<'_>) -> Result<Seat, CmdError> {
    let (unit, station) = crate::commands::focused_station(ctx, ctx.actor).ok_or_else(|| {
        CmdError::msg("You hold no focused station; +focus a billet before issuing orders.")
    })?;
    let state = ctx
        .w
        .module_state::<SpatialState>()
        .ok_or_else(|| CmdError::msg("The simulation is unavailable."))?;
    let channel = state
        .stations
        .get(&unit)
        .and_then(|host| host.stations.get(&station))
        .and_then(|spec| {
            (spec.channels.len() == 1)
                .then(|| spec.channels.iter().next().map(|c| c.as_str().to_owned()))
        })
        .flatten()
        .ok_or_else(|| {
            CmdError::msg(
                "That station does not grant exactly one control channel; it cannot issue orders.",
            )
        })?;
    let domain = crate::graph::effective_domain(state, unit)
        .ok_or_else(|| CmdError::msg("That unit is not placed in a simulation domain."))?;
    let record = state
        .domains
        .get(&domain)
        .ok_or_else(|| CmdError::msg("That unit is not placed in a simulation domain."))?;
    let generation = record.clock.generation().ok_or_else(|| {
        CmdError::msg(
            "That domain is passive; a staff member promotes it (@domain/promote) before tactical \
             orders resolve.",
        )
    })?;
    let now = ctx.w.world_ic_now();
    let tick = record.clock.tick_at(&now).ok_or_else(|| {
        CmdError::msg(
            "That domain's pulse is anchored on an older world time generation; staff must \
             re-promote it (@domain/tick).",
        )
    })?;
    let epoch = state.control_gen.get(&unit).copied().unwrap_or(0);
    Ok(Seat {
        unit,
        station,
        channel,
        epoch,
        domain,
        tick,
        generation,
    })
}

/// Stage one intent through the `tactical_intent` action.
///
/// Exactly ONE `Ctx::submit_authoritative` per command (cut 2's fold C4), under
/// a key that is deterministic over `(unit, station, tick, kind)` — so a
/// double-typed order in one tick is a HIT, and a *different* order on the same
/// channel in the same tick REPLACES the slot in the window (S2A's
/// latest-order-wins) rather than consuming a second seat.
fn submit(ctx: &mut Ctx<'_>, seat: &Seat, doc: TacticalIntentDoc) -> Result<(), CmdError> {
    let action = intent_action_ref(ctx.reg)
        .ok_or_else(|| CmdError::msg("The tactical ingress is not composed on this server."))?;
    let clock = pulse_clock(
        &crate::tick::owner().map_err(|e| CmdError::msg(e.to_string()))?,
        seat.domain,
        seat.generation,
    )
    .ok_or_else(|| CmdError::msg("That domain's tick clock is unavailable."))?;
    let observed_at = mush_core::ClockStamp {
        clock,
        at: seat.tick,
    };
    let kind = doc.kind;
    let document = serde_json::to_string(&doc)
        .map_err(|_| CmdError::msg("That order could not be encoded."))?;
    let key = IdempotencyKeyText::parse_canonical(format!(
        "sim.intent.{}.{}.{}.{}",
        seat.unit.0,
        seat.station.as_str(),
        seat.tick.max(0),
        kind.token()
    ))
    .map_err(|_| CmdError::msg("That order could not be keyed."))?;
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        Vec::new(),
        json!({
            "intent": document,
            "observed_at": serde_json::to_value(&observed_at).unwrap_or(serde_json::Value::Null),
        }),
        key,
    );
    Ok(())
}

/// The actor's held contact for `token`, or the byte-identical refusal.
///
/// The token is an opaque `ContactId` hex the holder already has from
/// `+contacts`; nothing here maps it to a unit, and no arm of this function
/// distinguishes "not yours" from "does not exist".
fn held_contact(ctx: &Ctx<'_>, token: &str) -> Result<ContactId, CmdError> {
    let refusal = || CmdError::msg(no_such_contact());
    let wanted = ContactId::from_hex(token.trim()).map_err(|_| refusal())?;
    let ledger = ctx
        .w
        .module_state::<ContactLedgerState>()
        .ok_or_else(refusal)?;
    let holder = mush_core::PrincipalId::Object { id: ctx.actor };
    ledger
        .rows()
        .iter()
        .any(|row| row.holder == holder && row.contact == wanted && row.active)
        .then_some(wanted)
        .ok_or_else(refusal)
}

fn slot_token(raw: &str) -> Result<String, CmdError> {
    let token = raw.trim().to_ascii_lowercase();
    if token.is_empty() || token.len() > MAX_SLOT_TOKEN_BYTES {
        return Err(CmdError::msg(format!(
            "Name a bay, section or dock of at most {MAX_SLOT_TOKEN_BYTES} characters."
        )));
    }
    Ok(token)
}

fn continuous(ctx: &mut Ctx<'_>, kind: IntentKind, value: OrderValue) -> Result<(), CmdError> {
    let seat = seat(ctx)?;
    let doc = TacticalIntentDoc {
        domain: seat.domain.0,
        tick: seat.tick,
        unit: seat.unit,
        station: seat.station.as_str().to_owned(),
        epoch: seat.epoch,
        channel: seat.channel.clone(),
        kind,
        value: Some(value.clone()),
        contact: None,
        slot: None,
    };
    submit(ctx, &seat, doc)?;
    let rendered = value.render();
    let tick = seat.tick;
    info(
        ctx,
        format!(
            "{} {rendered} — submitted into tick {tick}. It persists until changed; a second \
             order this tick replaces it.",
            kind.token()
        ),
    );
    Ok(())
}

fn discrete(
    ctx: &mut Ctx<'_>,
    kind: IntentKind,
    contact: Option<ContactId>,
    slot: Option<String>,
) -> Result<(), CmdError> {
    let seat = seat(ctx)?;
    let doc = TacticalIntentDoc {
        domain: seat.domain.0,
        tick: seat.tick,
        unit: seat.unit,
        station: seat.station.as_str().to_owned(),
        epoch: seat.epoch,
        channel: seat.channel.clone(),
        kind,
        value: None,
        contact,
        slot,
    };
    submit(ctx, &seat, doc)?;
    let tick = seat.tick;
    info(
        ctx,
        format!(
            "{} — submitted into tick {tick}. It resolves when the tick closes.",
            kind.token()
        ),
    );
    Ok(())
}

fn parse_vector(raw: &str) -> Option<OrderValue> {
    let mut parts = raw.split(',');
    let x = parts.next()?.trim().parse().ok()?;
    let y = parts.next()?.trim().parse().ok()?;
    let z = parts.next()?.trim().parse().ok()?;
    parts
        .next()
        .is_none()
        .then_some(OrderValue::Vector { x, y, z })
}

fn cmd_heading(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let value = parse_vector(ctx.args.raw())
        .ok_or_else(|| CmdError::msg("Usage: +heading <x>,<y>,<z> (each -1000..=1000)."))?;
    continuous(ctx, IntentKind::Heading, value)
}

fn cmd_thrust(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let value: i32 = ctx
        .args
        .raw()
        .trim()
        .parse()
        .map_err(|_| CmdError::msg("Usage: +thrust <n> (-1000..=1000)."))?;
    continuous(ctx, IntentKind::Thrust, OrderValue::Scalar { value })
}

fn cmd_posture(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let level: u8 = ctx
        .args
        .raw()
        .trim()
        .parse()
        .map_err(|_| CmdError::msg("Usage: +posture <0-3> (emission control level)."))?;
    if level > 3 {
        return Err(CmdError::msg(
            "Usage: +posture <0-3> (emission control level).",
        ));
    }
    continuous(ctx, IntentKind::Posture, OrderValue::Emcon { level })
}

fn cmd_fire(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let contact = held_contact(ctx, ctx.args.raw())?;
    discrete(ctx, IntentKind::Fire, Some(contact), None)
}

fn cmd_launch(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let bay = slot_token(ctx.args.lhs())?;
    let contact = held_contact(ctx, ctx.args.rhs().unwrap_or_default())?;
    discrete(ctx, IntentKind::Launch, Some(contact), Some(bay))
}

fn cmd_dc(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let section = slot_token(ctx.args.raw())?;
    discrete(ctx, IntentKind::DamageControl, None, Some(section))
}

fn cmd_dock(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let contact = held_contact(ctx, ctx.args.raw())?;
    discrete(ctx, IntentKind::Dock, Some(contact), None)
}

fn cmd_eject(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    discrete(ctx, IntentKind::Eject, None, None)
}

// ----------------------------------------------------------------------
// B6 — the immediate deterministic fallback controller (C30A, K7.0)
// ----------------------------------------------------------------------

/// The fallback order a linkdead or unmanned station issues.
///
/// **Deterministic, and it creates no `PendingDecision`** (08i answer 3,
/// D-E7-38): nothing asynchronous races it, so under K7.0's rule it submits
/// directly on the action-idempotency scope. Its key is the *same* shape a human
/// order takes on that `(unit, station, tick, kind)`, so if an LLM decision ever
/// competes for that slot the two collide on one key and the existing decision's
/// submission wins — which is exactly "reuse that decision's record and key
/// rather than minting a second".
///
/// The mapping is fixed: hold the last standing order. A controller that must
/// *choose* is K7's, and every such row stays a `#[ignore]` pin naming K7.
#[must_use]
pub fn fallback_order(kind: IntentKind) -> IntentKind {
    // Cut 8's declared fallback is "carry on": the continuous channels keep
    // their standing order (which needs no intent at all) and the discrete
    // vocabulary reduces to damage control, the one act that is never an attack.
    match kind {
        IntentKind::Fire | IntentKind::Launch | IntentKind::Dock | IntentKind::Eject => {
            IntentKind::DamageControl
        }
        other => other,
    }
}

/// The deterministic idempotency key a fallback submission takes.
///
/// Identical in shape to a human order's, so the two cannot both occupy one
/// `(unit, station, tick, kind)` slot.
#[must_use]
pub fn fallback_key(unit: ObjId, station: &str, tick: i64, kind: IntentKind) -> Option<String> {
    IdempotencyKeyText::parse_canonical(format!(
        "sim.intent.{}.{}.{}.{}",
        unit.0,
        station,
        tick.max(0),
        kind.token()
    ))
    .ok()
    .map(|key| key.as_str().to_owned())
}

// ----------------------------------------------------------------------
// registration
// ----------------------------------------------------------------------

/// Register the eight tactical verbs.
///
/// **K8.5:** the caller declares `CommandCompatibility::Authoritative` before
/// the first of these and restores `LegacyPostImage` after — every one of them
/// stages an authoritative action through cut 2's seam, which is the ceiling
/// class the tree sets for `@station` and `@prop`.
pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    let verbs: [Verb; 8] = [
        (
            "+heading",
            ArgShape::Raw,
            "+heading <x>,<y>,<z>   Set a heading. It persists until changed.",
            cmd_heading,
        ),
        (
            "+thrust",
            ArgShape::Raw,
            "+thrust <n>   Set thrust. It persists until changed.",
            cmd_thrust,
        ),
        (
            "+posture",
            ArgShape::Raw,
            "+posture <0-3>   Set emission-control posture. It persists until changed.",
            cmd_posture,
        ),
        (
            "+fire",
            ArgShape::Raw,
            "+fire <contact>   Fire at a contact you hold. One bounded intent this tick.",
            cmd_fire,
        ),
        (
            "+launch",
            ArgShape::EqPair,
            "+launch <bay>=<contact>   Launch from a bay at a contact you hold.",
            cmd_launch,
        ),
        (
            "+dc",
            ArgShape::Raw,
            "+dc <section>   Damage control on a section. One bounded intent this tick.",
            cmd_dc,
        ),
        (
            "+dock",
            ArgShape::Raw,
            "+dock <contact>   Dock with a contact you hold.",
            cmd_dock,
        ),
        (
            "+eject",
            ArgShape::None,
            "+eject   Abandon the unit. One bounded intent this tick.",
            cmd_eject,
        ),
    ];
    for (name, shape, help, handler) in verbs {
        reg.command(
            CommandSpec {
                name: SmolStr::new(name),
                shape,
                help: help.to_string(),
                ..CommandSpec::default()
            },
            Arc::new(handler),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A heading parses only as an exact three-component vector; anything else
    /// refuses rather than being partly read.
    #[test]
    fn e7c8_b6_a_heading_parses_exactly_three_components_or_refuses() {
        assert_eq!(
            parse_vector("1, 2 ,3"),
            Some(OrderValue::Vector { x: 1, y: 2, z: 3 })
        );
        assert_eq!(parse_vector("1,2"), None);
        assert_eq!(parse_vector("1,2,3,4"), None);
        assert_eq!(parse_vector("a,b,c"), None);
        assert_eq!(parse_vector(""), None);
    }

    /// The fallback's key is byte-identical to the key a human order takes on
    /// the same `(unit, station, tick, kind)` slot, which is what makes a
    /// competing decision a collision rather than a second submission (K7.0).
    #[test]
    fn e7c8_b6_the_fallback_key_is_the_same_slot_a_human_order_takes() {
        let human =
            fallback_key(ObjId(12), "helm", 7, IntentKind::DamageControl).expect("a keyable order");
        let fallback = fallback_key(ObjId(12), "helm", 7, fallback_order(IntentKind::Fire))
            .expect("a keyable fallback");
        assert_eq!(human, fallback);
        // And a different tick is a different slot, so a fallback never silently
        // overwrites the next tick's order.
        assert_ne!(
            human,
            fallback_key(ObjId(12), "helm", 8, IntentKind::DamageControl).expect("keyable")
        );
    }

    /// The declared fallback never attacks: every offensive kind reduces to the
    /// one act that is not an attack, and the continuous channels are untouched.
    #[test]
    fn e7c8_b6_the_declared_fallback_covers_every_intent_kind_and_never_attacks() {
        for kind in IntentKind::all() {
            let fallen = fallback_order(kind);
            assert!(
                !matches!(
                    fallen,
                    IntentKind::Fire | IntentKind::Launch | IntentKind::Dock | IntentKind::Eject
                ),
                "{kind:?} fell back to an offensive act"
            );
            if kind.channel().is_some() {
                assert_eq!(fallen, kind, "a continuous channel must be left alone");
            }
        }
    }

    /// A slot token is bounded and lower-cased, and an empty or oversized one
    /// refuses rather than being truncated.
    #[test]
    fn e7c8_b6_a_slot_token_is_bounded_and_never_truncated() {
        assert_eq!(slot_token(" Bay-1 ").expect("ok"), "bay-1");
        assert!(slot_token("").is_err());
        assert!(slot_token(&"x".repeat(MAX_SLOT_TOKEN_BYTES + 1)).is_err());
    }
}
