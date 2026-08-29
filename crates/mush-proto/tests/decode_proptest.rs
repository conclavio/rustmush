//! Fuzz-style property tests for the wire decode path (M9 hardening).
//!
//! [`ClientCommand`] and [`Envelope`] are decoded from bytes an untrusted
//! client (or a malformed proxy) puts on the WebSocket. The decode must never
//! panic and must stay bounded: arbitrary bytes, arbitrary UTF-8, arbitrary
//! JSON, and pathologically nested JSON all have to come back as `Ok`/`Err`,
//! never a crash or a stack overflow. Anything we *emit* must also round-trip
//! back to an equal value.

use mush_proto::{
    ClientCaps, ClientCommand, ClientKind, ColorDepth, Envelope, OutputEvent, Query, SmolStr,
};
use proptest::prelude::*;

/// Decoding arbitrary bytes as either wire type must never panic.
fn decode_all(bytes: &[u8]) {
    // from_slice over raw bytes (the WS binary path).
    let _ = serde_json::from_slice::<ClientCommand>(bytes);
    let _ = serde_json::from_slice::<Envelope>(bytes);
    // from_str when the bytes happen to be UTF-8 (the WS text path).
    if let Ok(s) = std::str::from_utf8(bytes) {
        let _ = serde_json::from_str::<ClientCommand>(s);
        let _ = serde_json::from_str::<Envelope>(s);
    }
}

/// A strategy over every `ClientCommand` variant with representative payloads.
fn client_command_strategy() -> impl Strategy<Value = ClientCommand> {
    let caps = (any::<u16>(), any::<u16>(), any::<bool>(), any::<bool>()).prop_map(
        |(width, height, utf8, structured)| {
            ClientCommand::Caps(ClientCaps {
                kind: ClientKind::Web,
                color: ColorDepth::TrueColor,
                width,
                height,
                utf8,
                structured,
            })
        },
    );
    let query = prop_oneof![
        Just(Query::Who),
        Just(Query::Sheet),
        any::<Option<String>>().prop_map(|folder| Query::MailList { folder }),
        (any::<String>(), any::<Option<u64>>(), any::<u32>()).prop_map(
            |(channel, before_seq, limit)| Query::ChannelBacklog {
                channel: SmolStr::new(&channel),
                before_seq,
                limit,
            }
        ),
        any::<String>().prop_map(|feed| Query::PanelSubscribe {
            feed: SmolStr::new(&feed)
        }),
    ];
    prop_oneof![
        any::<String>().prop_map(ClientCommand::Line),
        (any::<u64>(), query).prop_map(|(id, q)| ClientCommand::Req { id, q }),
        any::<u64>().prop_map(|last_seq| ClientCommand::Resume { last_seq }),
        any::<u64>().prop_map(|nonce| ClientCommand::Pong { nonce }),
        caps,
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    /// Arbitrary bytes never crash the decoder.
    #[test]
    fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..1024)) {
        decode_all(&bytes);
    }

    /// Arbitrary UTF-8 (often *nearly* valid JSON) never crashes the decoder.
    #[test]
    fn arbitrary_text_never_panics(s in ".{0,512}") {
        let _ = serde_json::from_str::<ClientCommand>(&s);
        let _ = serde_json::from_str::<Envelope>(&s);
    }

    /// Arbitrary JSON documents (correct at the JSON layer, wrong at the type
    /// layer) never crash the typed decode.
    #[test]
    fn arbitrary_json_values_never_panic(v in arb_json(4)) {
        let s = v.to_string();
        let _ = serde_json::from_str::<ClientCommand>(&s);
        let _ = serde_json::from_str::<Envelope>(&s);
    }

    /// Deeply nested JSON is rejected, not stack-overflowed: serde_json's
    /// recursion limit turns a nesting bomb into an `Err`.
    #[test]
    fn deep_nesting_is_bounded(depth in 1usize..1024) {
        let bomb = format!("{}{}", "[".repeat(depth), "]".repeat(depth));
        // The only requirement is that this returns rather than crashing.
        let _ = serde_json::from_str::<ClientCommand>(&bomb);
        let obj = format!(
            "{}{}",
            "{\"t\":\"req\",\"d\":{\"id\":0,\"q\":".repeat(depth),
            "}".repeat(depth)
        );
        let _ = serde_json::from_str::<ClientCommand>(&obj);
    }

    /// Everything we can emit round-trips through JSON unchanged.
    #[test]
    fn client_command_round_trips(cmd in client_command_strategy()) {
        let json = serde_json::to_string(&cmd).expect("serialize");
        let back: ClientCommand = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(cmd, back);
    }

    /// Envelopes round-trip, and the flattened `seq`/`ts`/`t`/`d` shape decodes
    /// back to the same event.
    #[test]
    fn envelope_round_trips(seq in any::<u64>(), ts in any::<i64>(), nonce in any::<u64>()) {
        let env = Envelope::new(seq, ts, OutputEvent::Ping { nonce });
        let json = serde_json::to_string(&env).expect("serialize");
        let back: Envelope = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(env, back);
    }
}

/// A bounded arbitrary `serde_json::Value` generator (depth-limited so the
/// strategy itself terminates).
fn arb_json(depth: u32) -> impl Strategy<Value = serde_json::Value> {
    let leaf = prop_oneof![
        Just(serde_json::Value::Null),
        any::<bool>().prop_map(serde_json::Value::Bool),
        any::<i64>().prop_map(|n| serde_json::json!(n)),
        ".{0,16}".prop_map(serde_json::Value::String),
    ];
    leaf.prop_recursive(depth, 32, 8, |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..6).prop_map(serde_json::Value::Array),
            proptest::collection::hash_map(".{0,8}", inner, 0..6)
                .prop_map(|m| serde_json::Value::Object(m.into_iter().collect())),
        ]
    })
}
