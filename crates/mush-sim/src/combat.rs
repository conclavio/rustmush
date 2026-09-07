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
    ModError, ObjId, PerceivedRef, Registrar, World,
};
use serde_json::json;
use smol_str::SmolStr;

use crate::data::SpatialState;
use crate::tick::{
    IntentKind, MAX_ORDER_SCALAR, MIN_ORDER_SCALAR, OrderValue, SeatDoc, TacticalIntentDoc,
    intent_action_ref, no_such_contact,
};

/// The most bytes a bay, section or dock token may carry. The wire re-runs the
/// same bound inside `TacticalIntentDoc::validate` (fold B10), so a document
/// built anywhere carries the same limit the verb does.
pub use crate::tick::MAX_SLOT_TOKEN_BYTES;

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

/// **Resolve the actor's live tactical seat, or the exact reason it refuses.**
///
/// This is the ONE seat predicate (fold D1(a)): the verb calls it so the player
/// gets an actionable refusal, and `TacticalIntentProvider::project_offer` calls
/// it so the *same* seat becomes the offer's fixed parameter and the provider
/// can compare the caller's document against it. A verb and an offer that
/// resolved the seat separately could disagree, which is the whole class of
/// defect the pinned seat exists to remove.
///
/// It takes a `&World` rather than a `&Ctx` because `project_offer` has an
/// `&Engine` and no `Ctx`.
///
/// # Errors
///
/// A player-facing reason: not focused, the unit is not in a domain, the domain
/// is not pulsing, its pulse is anchored on an older world-time generation, or
/// the actor holds no published grant on the station they are focused on.
pub(crate) fn resolve_seat(world: &World, actor: ObjId) -> Result<SeatDoc, CmdError> {
    let (unit, station) = crate::commands::focused_station_of(world, actor).ok_or_else(|| {
        CmdError::msg("You hold no focused station; +focus a billet before issuing orders.")
    })?;
    let state = world
        .module_state::<SpatialState>()
        .ok_or_else(|| CmdError::msg("The simulation is unavailable."))?;
    // **A5:** the epoch, the channel set and the priority all come from the
    // actor's OWN published `ChannelGrant` — the exact record the apply witness
    // compares. Cut 8 pinned `SpatialState.control_gen[unit]`, the unit-wide
    // counter, while `apply_focus_bump_and_refresh` republishes only
    // `ctx.actor`'s links, so any crewmate's `+focus` silently voided every
    // other station's orders AFTER the player had been told "submitted".
    let grant = world
        .channel_grant(actor, unit, crate::NS, station.as_str())
        .ok_or_else(|| {
            CmdError::msg(
                "You hold no live control grant on that station; +focus it again (a crew change \
                 may have republished it).",
            )
        })?;
    // **A6:** the whole granted set travels, and the kind decides which channel
    // the order rides. A station granting two channels can now issue on both,
    // and a gunner can no longer steer.
    let channels: Vec<String> = grant
        .channels
        .iter()
        .map(|channel| channel.as_str().to_owned())
        .collect();
    if channels.is_empty() {
        return Err(CmdError::msg(
            "That station grants no control channel; it cannot issue orders.",
        ));
    }
    let epoch = grant.epoch;
    let priority = grant.priority;
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
    let now = world.world_ic_now();
    let tick = record.clock.tick_at(&now).ok_or_else(|| {
        CmdError::msg(
            "That domain's pulse is anchored on an older world time generation; staff must \
             re-promote it (@domain/tick).",
        )
    })?;
    Ok(SeatDoc {
        unit,
        station: station.as_str().to_owned(),
        channels,
        epoch,
        priority,
        domain: domain.0,
        tick,
        generation,
    })
}

fn seat(ctx: &Ctx<'_>) -> Result<SeatDoc, CmdError> {
    let seat = resolve_seat(ctx.w, ctx.actor)?;
    Ok(seat)
}

/// The channel one kind rides, refused with its reason when the focused station
/// does not grant it (fold A6).
fn channel_for(seat: &SeatDoc, kind: IntentKind) -> Result<String, CmdError> {
    let wanted = kind.required_channel();
    if seat.channels.iter().any(|held| held == wanted) {
        Ok(wanted.to_owned())
    } else {
        Err(CmdError::msg(format!(
            "Your station ({}) does not grant the {wanted} channel; {} rides it.",
            seat.station,
            kind.token()
        )))
    }
}

/// Stage one intent through the `tactical_intent` action.
///
/// Exactly ONE `Ctx::submit_authoritative` per command (cut 2's fold C4), under
/// a key that is deterministic over `(unit, station, tick, kind)` — so a
/// double-typed order in one tick is a HIT, and a *different* order on the same
/// channel in the same tick REPLACES the slot in the window (S2A's
/// latest-order-wins) rather than consuming a second seat.
fn submit(ctx: &mut Ctx<'_>, seat: &SeatDoc, doc: TacticalIntentDoc) -> Result<(), CmdError> {
    let action = intent_action_ref(ctx.reg)
        .ok_or_else(|| CmdError::msg("The tactical ingress is not composed on this server."))?;
    let kind = doc.kind;
    // **A2:** the stamp is no longer a parameter at all. Cut 8 declared
    // `observed_at` as `ProjectedParameterKind::Text` and submitted
    // `serde_json::to_value(&ClockStamp)` — an OBJECT — so `validate_parameter_value`
    // refused every one of the eight verbs with `OfferResolveError::InvalidParameters`
    // at `resolve_and_stage_offer` and no intent ever reached a window through
    // the command path. The seat carries `(domain, generation, tick)` as a fixed
    // parameter and the provider derives the stamp, so the schema and the verb
    // cannot disagree by construction.
    let document = serde_json::to_string(&doc)
        .map_err(|_| CmdError::msg("That order could not be encoded."))?;
    let key = IdempotencyKeyText::parse_canonical(format!(
        "sim.intent.{}.{}.{}.{}",
        seat.unit.0,
        seat.station,
        seat.tick.max(0),
        kind.token()
    ))
    .map_err(|_| CmdError::msg("That order could not be keyed."))?;
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        Vec::new(),
        json!({ "intent": document }),
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
    let channel = channel_for(&seat, kind)?;
    let doc = TacticalIntentDoc {
        domain: seat.domain,
        tick: seat.tick,
        unit: seat.unit,
        station: seat.station.clone(),
        epoch: seat.epoch,
        priority: seat.priority,
        channel,
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
    let channel = channel_for(&seat, kind)?;
    let doc = TacticalIntentDoc {
        domain: seat.domain,
        tick: seat.tick,
        unit: seat.unit,
        station: seat.station.clone(),
        epoch: seat.epoch,
        priority: seat.priority,
        channel,
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

/// **The documented band is enforced at the verb** (fold B10).
///
/// `+thrust 5000` used to parse, consume the actor's one authoritative submit
/// slot and come back asynchronously as "that tactical intent's value does not
/// match its kind" — a refusal that names neither the band nor the verb.
const fn in_band(value: i32) -> bool {
    value >= MIN_ORDER_SCALAR && value <= MAX_ORDER_SCALAR
}

fn parse_vector(raw: &str) -> Option<OrderValue> {
    let mut parts = raw.split(',');
    let x: i32 = parts.next()?.trim().parse().ok()?;
    let y: i32 = parts.next()?.trim().parse().ok()?;
    let z: i32 = parts.next()?.trim().parse().ok()?;
    if !in_band(x) || !in_band(y) || !in_band(z) {
        return None;
    }
    parts
        .next()
        .is_none()
        .then_some(OrderValue::Vector { x, y, z })
}

fn cmd_heading(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let value = parse_vector(ctx.args.raw()).ok_or_else(|| {
        CmdError::msg(format!(
            "Usage: +heading <x>,<y>,<z> (each {MIN_ORDER_SCALAR}..={MAX_ORDER_SCALAR})."
        ))
    })?;
    continuous(ctx, IntentKind::Heading, value)
}

fn cmd_thrust(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let usage = || {
        CmdError::msg(format!(
            "Usage: +thrust <n> ({MIN_ORDER_SCALAR}..={MAX_ORDER_SCALAR})."
        ))
    };
    let value: i32 = ctx.args.raw().trim().parse().map_err(|_| usage())?;
    if !in_band(value) {
        return Err(usage());
    }
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

/// The fallback order a linkdead or unmanned station **would** issue.
///
/// **NOT BUILT — recorded honestly** (fold A11). This function and
/// [`fallback_key`] have no caller anywhere in the tree: the only uses are this
/// module's own tests. `station::fire_station_fallback` stages a **control
/// handoff**, not a tactical order, so a linkdead or unmanned station seats no
/// fallback order at all — its continuous orders simply persist (which is what
/// "carry on" means in practice) and its discrete vocabulary lapses.
///
/// The safety claim cut 8 recorded here is also **false and is withdrawn**: a
/// `ScopedIdempotencyKey` is scoped by `IdempotencyScope.principal`, and the
/// kernel's invocation constructor enforces that the scope's principal equals
/// the invoking principal — so a fallback submitted by any *other* principal is
/// a SECOND submission, not a collision, and it would land in its own window
/// slot beside the human's. If this is ever wired, it must submit under the
/// **same principal** as the human order for the keys to collide as claimed.
///
/// The mapping itself is fixed and deliberate: hold the last standing order, and
/// reduce the discrete vocabulary to the one act that is never an attack. A
/// controller that must *choose* is K7's.
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

/// The deterministic idempotency key a fallback submission would take.
///
/// Identical in shape to a human order's — but see [`fallback_order`]: the keys
/// only collide when both submissions share one `IdempotencyScope.principal`,
/// which nothing in the tree arranges yet.
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
