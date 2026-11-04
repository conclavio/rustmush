//! Fuzz-style property tests for the wire decode path (M9 hardening).
//!
//! [`ClientCommand`] and [`Envelope`] are decoded from bytes an untrusted
//! client (or a malformed proxy) puts on the WebSocket. The decode must never
//! panic and must stay bounded: arbitrary bytes, arbitrary UTF-8, arbitrary
//! JSON, and pathologically nested JSON all have to come back as `Ok`/`Err`,
//! never a crash or a stack overflow. Anything we *emit* must also round-trip
//! back to an equal value.

use mush_proto::{
    ActionStatusId, ClientCaps, ClientCommand, ClientKind, ColorDepth, ContentStudioRequest,
    Envelope, InvokeRequest, LegalActionRequest, MindRequest, OutputEvent, PictureKind,
    ProjectedCommitToken, Query, RespondChoice, SmolStr, UnitDesignRequest,
};
use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::TestRunner;

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
///
/// Exhaustive by construction (doc 43 cut TS.4): [`variant_tag`] matches every
/// `ClientCommand`, `Query` and `InvokeRequest` variant with no wildcard, so a
/// new variant fails to compile here, and `strategy_reaches_every_variant`
/// fails until this strategy generates it.
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
        Just(Query::Commands),
        (any::<bool>(), any::<Option<u128>>()).prop_map(|(own, after)| {
            Query::MarketOriginalMinds(mush_proto::MarketOriginalMindsRequest {
                kind: if own {
                    mush_proto::MarketOriginalMindsKind::Own
                } else {
                    mush_proto::MarketOriginalMindsKind::Listings
                },
                after: after.map(|n| {
                    mush_proto::ProjectedCommitToken::new(format!("{n:032x}"))
                        .expect("canonical generated cursor")
                }),
            })
        }),
        (any::<Option<u64>>(), any::<Option<u16>>())
            .prop_map(|(run, page)| Query::MindBulk { run, page }),
        // 17g cut C3a: the player market's two panel reads.
        any::<Option<u32>>().prop_map(|after| {
            Query::MindMarket(mush_proto::MindMarketRequest::Listings {
                after: after.map(|n| {
                    mush_proto::ListingCode::try_from(format!("{n:08x}"))
                        .expect("canonical generated code")
                }),
            })
        }),
        (any::<bool>(), any::<Option<u64>>()).prop_map(|(paid, after)| {
            Query::MindMarket(mush_proto::MindMarketRequest::Holdings {
                section: if paid {
                    mush_proto::MindHoldingsSection::Paid
                } else {
                    mush_proto::MindHoldingsSection::Owner
                },
                after,
            })
        }),
        any::<Option<String>>().prop_map(|folder| Query::MailList { folder }),
        (any::<String>(), any::<Option<u64>>(), any::<u32>()).prop_map(
            |(channel, before_seq, limit)| Query::ChannelBacklog {
                channel: SmolStr::new(&channel),
                before_seq,
                limit,
            }
        ),
        (any::<String>(), any::<Option<String>>()).prop_map(|(feed, word)| {
            Query::PanelSubscribe {
                feed: SmolStr::new(&feed),
                params: word.map(|word| serde_json::json!({ "family": word })),
            }
        }),
        (any::<Option<u64>>(), any::<u32>())
            .prop_map(|(before_seq, limit)| Query::NarrativeBacklog { before_seq, limit }),
    ];
    prop_oneof![
        any::<String>().prop_map(ClientCommand::Line),
        (any::<u64>(), query).prop_map(|(id, q)| ClientCommand::Req { id, q }),
        any::<u64>().prop_map(|last_seq| ClientCommand::Resume { last_seq }),
        any::<u64>().prop_map(|nonce| ClientCommand::Pong { nonce }),
        caps,
        (any::<u64>(), invoke_strategy())
            .prop_map(|(id, request)| ClientCommand::Invoke { id, request }),
        any::<u128>().prop_map(|n| ClientCommand::Ack {
            commit: ProjectedCommitToken::new(format!("{n:032x}"))
                .expect("canonical generated token"),
        }),
        any::<bool>().prop_map(|active| ClientCommand::Composing { active }),
        (any::<String>(), any::<u64>(), respond_choice_strategy())
            .prop_map(|(epoch, id, choice)| ClientCommand::Respond { epoch, id, choice }),
        (any::<bool>(), any::<i64>(), any::<bool>()).prop_map(|(on, look_mdeg, overhead)| {
            ClientCommand::Picture {
                on,
                look_mdeg,
                kind: if overhead {
                    PictureKind::Overhead
                } else {
                    PictureKind::Cockpit
                },
            }
        }),
    ]
}

fn respond_choice_strategy() -> impl Strategy<Value = RespondChoice> {
    prop_oneof![
        any::<bool>().prop_map(RespondChoice::Confirm),
        any::<String>().prop_map(|id| RespondChoice::Choose(SmolStr::new(&id))),
        any::<String>().prop_map(RespondChoice::Text),
        Just(RespondChoice::Cancel),
    ]
}

/// One representative request per `InvokeRequest` family; the families' own
/// payload grammars are pinned by their crates' tests.
fn invoke_strategy() -> impl Strategy<Value = InvokeRequest> {
    prop_oneof![
        any::<String>().prop_map(|name| InvokeRequest::Mind(MindRequest::Remove { name })),
        (any::<u32>(), any::<u32>()).prop_map(|(first, last)| {
            InvokeRequest::Mind(MindRequest::FiledSource {
                licence: "7".to_owned(),
                first,
                last,
            })
        }),
        Just(InvokeRequest::LegalAction(LegalActionRequest::Status {
            reference: ActionStatusId::new(
                "k5:toy:brace:0000000000000007:01010101010101010101010101010101"
            )
            .expect("canonical status id"),
        })),
        (any::<String>(), any::<u32>()).prop_map(|(id, version)| {
            InvokeRequest::UnitDesign(UnitDesignRequest::Check {
                id,
                version,
                rules: serde_json::json!({ "hull": version }),
            })
        }),
        Just(InvokeRequest::ContentStudio(
            ContentStudioRequest::Catalog {}
        )),
        any::<String>()
            .prop_map(|id| InvokeRequest::ContentStudio(ContentStudioRequest::Read { id })),
    ]
}

/// Every `ClientCommand`, `Query` and `InvokeRequest` variant's tag. No
/// wildcard arm: a new variant is a compile error until it is listed here, and
/// listing it here fails `strategy_reaches_every_variant` until the strategy
/// generates it.
fn variant_tag(cmd: &ClientCommand) -> &'static str {
    match cmd {
        ClientCommand::Line(_) => "line",
        ClientCommand::Invoke { request, .. } => match request {
            InvokeRequest::Mind(_) => "invoke.mind",
            InvokeRequest::LegalAction(_) => "invoke.legal_action",
            InvokeRequest::UnitDesign(_) => "invoke.unit_design",
            InvokeRequest::ContentStudio(_) => "invoke.content_studio",
        },
        ClientCommand::Req { q, .. } => match q {
            Query::MailList { .. } => "req.mail_list",
            Query::Who => "req.who",
            Query::Sheet => "req.sheet",
            Query::Commands => "req.commands",
            Query::ChannelBacklog { .. } => "req.channel_backlog",
            Query::NarrativeBacklog { .. } => "req.narrative_backlog",
            Query::MindBulk { .. } => "req.mind_bulk",
            Query::MarketOriginalMinds(_) => "req.market_original_minds",
            Query::MindMarket(_) => "req.mind_market",
            Query::PanelSubscribe { .. } => "req.panel_subscribe",
        },
        ClientCommand::Resume { .. } => "resume",
        ClientCommand::Ack { .. } => "ack",
        ClientCommand::Composing { .. } => "composing",
        ClientCommand::Respond { .. } => "respond",
        ClientCommand::Pong { .. } => "pong",
        ClientCommand::Picture { .. } => "picture",
        ClientCommand::Caps(_) => "caps",
    }
}

/// Every tag [`variant_tag`] can return; keep in step with its arms.
const ALL_TAGS: &[&str] = &[
    "line",
    "invoke.mind",
    "invoke.legal_action",
    "invoke.unit_design",
    "invoke.content_studio",
    "req.mail_list",
    "req.who",
    "req.sheet",
    "req.commands",
    "req.channel_backlog",
    "req.narrative_backlog",
    "req.mind_bulk",
    "req.market_original_minds",
    "req.mind_market",
    "req.panel_subscribe",
    "resume",
    "ack",
    "composing",
    "respond",
    "pong",
    "picture",
    "caps",
];

/// The round-trip property below covers the whole client vocabulary: drawing
/// from the strategy reaches every tag `variant_tag` knows.
#[test]
fn strategy_reaches_every_variant() {
    let strategy = client_command_strategy();
    let mut runner = TestRunner::deterministic();
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..20_000 {
        let cmd = strategy
            .new_tree(&mut runner)
            .expect("strategy draws")
            .current();
        seen.insert(variant_tag(&cmd));
        if seen.len() == ALL_TAGS.len() {
            break;
        }
    }
    let missing: Vec<_> = ALL_TAGS.iter().filter(|t| !seen.contains(*t)).collect();
    assert!(
        missing.is_empty(),
        "client_command_strategy never generates {missing:?}"
    );
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
