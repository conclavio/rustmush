//! **WS03-S3 — the tick window against the kernel's canonical byte budget.**
//!
//! `TacticalIntentDoc::validate` already bounds a station or channel token
//! because "the whole invocation is retained inside the window post-image, so
//! an unbounded one is a stored record that could never have been written"
//! (fold B10). The kernel now finishes that thought: `ActionWindowRecord`
//! charges the canonical bytes of its own post-image, holding back
//! `WINDOW_TERMINAL_RESERVE_BYTES` so the close and the consume a tick still
//! owes are always affordable.
//!
//! This is the measurement of what that costs the tick, in the idiom of the
//! `measurement_report` table in `tick.rs`: with WORST-CASE order documents —
//! station and channel tokens at `MAX_STATION_TOKEN_BYTES`, every channel in
//! the seat, saturated numbers, and both documents carried as ESCAPED JSON
//! strings inside the invocation — a tick window reaches the kernel's input
//! budget WELL SHORT of its own 256-slot ceiling. On a maximally loaded domain
//! the binding constraint is bytes, not slots.
//!
//! That is a refusal, not a loss. Submits past the budget answer
//! `BoundsExceeded` (the arm beside `CapacityReached` in the tick's submit
//! path); before the charge the same submits were ACCEPTED into a window whose
//! close post-image the state family would then refuse — losing the whole
//! tick, not the last order in it. If the intent document grows, the number
//! this test prints says what it costs a full domain, and the honest answers
//! are a smaller document or a smaller unit cap.

use mush_core::{
    ActionInvocation, ActionInvocationDraft, ActionOfferId, ActionWindow, ActionWindowDraft,
    ActionWindowError, ActionWindowRecord, CanonicalHash, CanonicalId, ClockStamp, CommitToken,
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, DefinitionRegistryBuilder,
    IdempotencyKey, IdempotencyKeyText, IdempotencyScope, MAX_WINDOW_INPUT_RECORD_BYTES,
    MAX_WINDOW_RECORD_BYTES, ObjId, OwnerId, PerceivedRef, PrincipalId, RequestUid,
    ScopedIdempotencyKey, SubjectKey, WindowIntent, WindowMode, WindowOrderKey,
    WindowRestartPolicy, WindowTimeoutPolicy,
};
use mush_sim::tick::{
    IntentKind, MAX_INTENTS_PER_TICK, MAX_STATION_TOKEN_BYTES, OrderChannel, OrderValue, SeatDoc,
    TacticalIntentDoc, pulse_clock, tick_window_id,
};
use serde_json::json;

const NS: &str = "sim";

/// The tick's own window shape: one domain-tick span, the domain's intent
/// ceiling, mirroring `tick::open_window`, which is crate-private.
fn open_window(owner: &OwnerId, domain: u64, generation: u64, tick: i64) -> ActionWindow {
    let clock = pulse_clock(owner, mush_sim::data::DomainId(domain), generation)
        .expect("a domain tick clock");
    ActionWindow::new(ActionWindowDraft {
        id: tick_window_id(owner, mush_sim::data::DomainId(domain), generation, tick)
            .expect("a window identity"),
        clock: clock.clone(),
        opens: ClockStamp {
            clock: clock.clone(),
            at: tick,
        },
        closes: ClockStamp {
            clock,
            at: tick + 1,
        },
        generation,
        mode: WindowMode::CollectSimultaneous,
        capacity: MAX_INTENTS_PER_TICK,
        restart: WindowRestartPolicy::CloseIfElapsed,
        timeout: WindowTimeoutPolicy::Close,
        max_catch_up: 1,
        resolution_secret_version: Some(1),
    })
    .expect("the tick's own window shape")
}

/// The `(unit, kind)` slot the submit path mints, mirroring
/// `TacticalIntentProvider::slot_subject`.
fn slot_subject(owner: &OwnerId, unit: ObjId, kind: IntentKind) -> SubjectKey {
    SubjectKey::Scoped {
        owner: owner.clone(),
        class: CanonicalId::new("tactical_order").expect("static class"),
        id: IdempotencyKeyText::parse_canonical(format!("{}.{}", unit.0, kind.token()))
            .expect("a canonical slot id"),
    }
}

/// One worst-case order: the real `TacticalIntentDoc`/`SeatDoc` types, their
/// free text at the bound, carried the way the verb carries them — as escaped
/// JSON strings inside the invocation parameters.
fn worst_case_intent(
    window: &ActionWindow,
    owner: &OwnerId,
    action: &DefinitionRef,
    domain: u64,
    generation: u64,
    tick: i64,
    index: usize,
) -> WindowIntent {
    let channels = OrderChannel::all();
    let channel = channels[index % channels.len()];
    let kind = match channel {
        OrderChannel::Heading => IntentKind::Heading,
        OrderChannel::Thrust => IntentKind::Thrust,
        OrderChannel::Speed => IntentKind::Speed,
        OrderChannel::Posture => IntentKind::Posture,
    };
    let value = match channel {
        OrderChannel::Heading => OrderValue::Vector {
            x: -999,
            y: 999,
            z: -999,
        },
        OrderChannel::Thrust | OrderChannel::Speed => OrderValue::Scalar { value: -999 },
        OrderChannel::Posture => OrderValue::Emcon { level: 3 },
    };
    let unit = ObjId(1_000_000 + u32::try_from(index / channels.len()).expect("a unit dbref"));
    let actor = ObjId(2_000_000 + u32::try_from(index).expect("an actor dbref"));
    let station = "s".repeat(MAX_STATION_TOKEN_BYTES);
    let doc = TacticalIntentDoc {
        domain,
        tick,
        unit,
        station: station.clone(),
        epoch: u64::from(u32::MAX),
        channel: channel.token().to_owned(),
        priority: u16::MAX,
        kind,
        value: Some(value),
        contact: None,
        contact_generation: None,
        slot: None,
    };
    let seat = SeatDoc {
        unit,
        station,
        channels: channels.iter().map(|c| c.token().to_owned()).collect(),
        epoch: u64::from(u32::MAX),
        priority: u16::MAX,
        domain,
        tick,
        generation,
    };
    let request = RequestUid::from_bytes(
        u128::try_from(index + 1)
            .expect("an operation index")
            .to_be_bytes(),
    );
    let principal = PrincipalId::Object { id: actor };
    let invocation = ActionInvocation::new(ActionInvocationDraft {
        request,
        principal: principal.clone(),
        actor,
        offered_action: ActionOfferId(CanonicalHash::from_bytes([0x51; 32])),
        subject: PerceivedRef::KnownObject {
            id: unit,
            revision: u64::from(u32::MAX),
        },
        targets: Vec::new(),
        parameters: json!({
            "intent": serde_json::to_string(&doc).expect("the intent document serializes"),
            "seat": serde_json::to_string(&seat).expect("the seat document serializes"),
        }),
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: owner.clone(),
                namespace: CanonicalId::new("tactical_intent").expect("static namespace"),
            },
            generation,
            IdempotencyKey::Request(request),
        ),
    })
    .expect("a well-formed invocation");
    WindowIntent::new(
        window.id(),
        generation,
        slot_subject(owner, unit, kind),
        Vec::new(),
        action.clone(),
        invocation,
        WindowOrderKey::new(vec![
            kind.order_rank(),
            i64::from(unit.0),
            i64::from(u16::MAX),
        ])
        .expect("a bounded order key"),
    )
    .expect("an admissible intent")
}

fn tactical_action(owner: &OwnerId) -> DefinitionRef {
    let mut definitions = DefinitionRegistryBuilder::new();
    definitions
        .register(
            owner.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new("action").expect("static kind"),
                DefinitionId::new("tactical_intent").expect("static id"),
                1,
                json!({}),
            )
            .expect("a static descriptor"),
        )
        .expect("the fixture registers one action")
}

#[test]
fn ws03_s3_a_tick_window_meets_the_byte_budget_before_its_slot_ceiling() {
    let owner = OwnerId::new(NS).expect("static owner");
    let (domain, generation, tick) = (7_u64, 1_u64, 4_096_i64);
    let definition = open_window(&owner, domain, generation, tick);
    let action = tactical_action(&owner);
    let stamp = ClockStamp {
        clock: definition.clock().clone(),
        at: tick,
    };
    let due = ClockStamp {
        clock: definition.clock().clone(),
        at: tick + 1,
    };

    let mut record = ActionWindowRecord::open(definition.clone());
    let mut admitted = 0_usize;
    let mut refused = None;
    for index in 0..MAX_INTENTS_PER_TICK as usize {
        let intent = worst_case_intent(
            &definition,
            &owner,
            &action,
            domain,
            generation,
            tick,
            index,
        );
        match record.checked_submit(record.revision(), &stamp, intent) {
            Ok(transition) => {
                record = transition.into_record();
                admitted += 1;
            }
            Err(ActionWindowError::RecordBytes { attempted, max }) => {
                refused = Some((attempted, max));
                break;
            }
            Err(other) => panic!("a tick slot refused for {other:?}"),
        }
    }
    let filled = record.canonical_bytes().expect("the record measures");
    println!(
        "a worst-case tick window admits {admitted} of {MAX_INTENTS_PER_TICK} slots at {filled} \
         canonical bytes ({}% of the input budget, {} bytes per intent and its receipt); the byte \
         budget answered {refused:?}",
        filled * 100 / MAX_WINDOW_INPUT_RECORD_BYTES,
        filled / admitted.max(1)
    );
    assert!(
        filled <= MAX_WINDOW_INPUT_RECORD_BYTES,
        "the input budget is what admitted the last intent"
    );
    let (attempted, max) = refused.expect(
        "the worst case reaches the kernel's byte budget; if it no longer does, the intent \
         document has SHRUNK and this measurement should be re-cut",
    );
    assert_eq!(max, MAX_WINDOW_INPUT_RECORD_BYTES);
    assert!(attempted > max);
    assert!(
        admitted >= 96,
        "the byte budget bit at {admitted} slots, well under the {MAX_INTENTS_PER_TICK} the \
         domain caps at — the intent document has grown and the sim's own ceiling should be re-cut"
    );

    // Whichever bound bit, the tick still resolves: the close and the consume
    // are what the kernel's terminal reserve is held back for.
    let closed = record
        .checked_close_due(record.revision(), RequestUid::from_bytes([0xB1; 16]), &due)
        .expect("the reserve keeps the close available")
        .into_record();
    let consumed = closed
        .checked_consume(
            closed.revision(),
            RequestUid::from_bytes([0xB2; 16]),
            CommitToken::from_bytes([0xB3; 16]),
            &due,
        )
        .expect("and the consume after it")
        .into_record();
    assert!(consumed.canonical_bytes().expect("it measures") <= MAX_WINDOW_RECORD_BYTES);
}
