//! D3: exhaustive codec round-trip coverage for the boundary protocol.
//!
//! Every [`ToServer`] and [`ToGateway`] variant must survive an
//! encode→decode round trip byte-for-byte-equivalent (`PartialEq`), including
//! the `serde_json::Value`-carrying arms ([`WireInbound::Query`],
//! [`ToGateway::Deliver`] of an [`OutputEvent::Panel`]); a framed multi-message
//! stream must decode in order; a truncated frame must read as
//! needs-more-bytes, never a panic; and the version field must be present.

use mush_core::{
    ActionOfferId, ControlMsg as CoreControl, DisconnectKind, Inbound, ObjId, PerceivedRef,
    WorldCommand,
};
use mush_ipc::codec::{decode_frame, encode_frame_into};
use mush_ipc::{
    AttachRec, CanonicalHash, FrameDecoder, GatewayAction, GatewayInput, IPC_VERSION,
    MAX_FRAME_LEN, MAX_IN_FLIGHT_GATEWAY_INPUT_BYTES, MAX_IN_FLIGHT_GATEWAY_INPUTS,
    MAX_IN_FLIGHT_LIFECYCLE_OBLIGATIONS, MAX_IN_FLIGHT_SERVICE_REQUESTS, ToGateway, ToServer,
    WireInbound, WireInboundError, encode_frame,
};

mod samples;
use samples::{every_to_gateway, every_to_server};

/// Encode one message and decode it back, asserting equality.
fn round_trip_to_server(msg: &ToServer) {
    let bytes = encode_frame(msg).expect("encode ToServer");
    let (got, consumed): (ToServer, usize) = decode_frame(&bytes)
        .expect("decode ToServer")
        .expect("a whole frame");
    assert_eq!(consumed, bytes.len(), "the frame consumes the whole buffer");
    assert_eq!(&got, msg, "ToServer round-trips");
}

fn round_trip_to_gateway(msg: &ToGateway) {
    let bytes = encode_frame(msg).expect("encode ToGateway");
    let (got, consumed): (ToGateway, usize) = decode_frame(&bytes)
        .expect("decode ToGateway")
        .expect("a whole frame");
    assert_eq!(consumed, bytes.len(), "the frame consumes the whole buffer");
    assert_eq!(&got, msg, "ToGateway round-trips");
}

#[test]
fn every_to_server_variant_round_trips() {
    for msg in every_to_server() {
        round_trip_to_server(&msg);
    }
}

#[test]
fn every_to_gateway_variant_round_trips() {
    for msg in every_to_gateway() {
        round_trip_to_gateway(&msg);
    }
}

#[test]
fn version_is_present_and_travels_in_hello() {
    // Name-only attributed removal and its pending reply (9), authoritative
    // world health (10) and mirrored reply settings with reply-completion
    // fences (11) extend the closed vocabulary. An older peer cannot decode or
    // honor them; deploy both halves together.
    assert_eq!(IPC_VERSION, 12, "the boundary protocol version is pinned");
    assert_eq!(
        MAX_IN_FLIGHT_SERVICE_REQUESTS, 256,
        "service-request custody is a fixed cross-boundary contract"
    );
    assert_eq!(
        MAX_IN_FLIGHT_LIFECYCLE_OBLIGATIONS, 256,
        "connection-lifecycle custody is a fixed cross-boundary contract"
    );
    assert_eq!(MAX_IN_FLIGHT_GATEWAY_INPUTS, 1_024);
    assert_eq!(MAX_IN_FLIGHT_GATEWAY_INPUT_BYTES, 8 * 1024 * 1024);
    let hello = ToGateway::Hello {
        proto: IPC_VERSION,
        server: "mushd".to_string(),
        features: vec![],
        panels: vec![],
    };
    let bytes = encode_frame(&hello).unwrap();
    let (got, _): (ToGateway, usize) = decode_frame(&bytes).unwrap().unwrap();
    match got {
        ToGateway::Hello { proto, .. } => assert_eq!(proto, IPC_VERSION),
        other => panic!("expected Hello, got {other:?}"),
    }
}

/// The lifecycle ceiling also bounds the one-frame restart snapshot. Account
/// and character names use their maximum accepted lengths here, so the full
/// roster cannot discover at restart time that it exceeds `MAX_FRAME_LEN`.
#[test]
fn maximum_lifecycle_reannounce_snapshot_fits_one_frame() {
    let recs = (0..MAX_IN_FLIGHT_LIFECYCLE_OBLIGATIONS)
        .map(|i| AttachRec {
            output_brackets: Default::default(),
            conn: i as u64,
            actor: ObjId(i as u32),
            account: Some("a".repeat(20)),
            char_name: "c".repeat(20),
            caps_structured: i % 2 == 0,
        })
        .collect();
    let bytes = encode_frame(&ToServer::ReAnnounce(recs)).expect("bounded roster encodes");
    assert!(
        bytes.len() <= MAX_FRAME_LEN,
        "bounded roster frame is {} bytes, maximum is {MAX_FRAME_LEN}",
        bytes.len()
    );
}

/// A framed stream of many messages decodes in order regardless of how the
/// bytes are chunked into `feed` calls.
#[test]
fn framed_multi_message_stream_decodes_in_order() {
    let all = every_to_server();

    // One contiguous buffer holding every frame back-to-back.
    let mut stream = Vec::new();
    for msg in &all {
        encode_frame_into(msg, &mut stream).unwrap();
    }

    // Feed the stream in awkward 3-byte chunks; the decoder must reassemble.
    let mut dec = FrameDecoder::new();
    let mut decoded: Vec<ToServer> = Vec::new();
    let mut i = 0;
    while i < stream.len() {
        let end = (i + 3).min(stream.len());
        dec.feed(&stream[i..end]);
        i = end;
        while let Some(msg) = dec.next_message::<ToServer>().unwrap() {
            decoded.push(msg);
        }
    }
    assert_eq!(decoded, all, "the whole stream decodes in order");
    assert_eq!(dec.pending_len(), 0, "no bytes left buffered");
}

/// A truncated frame is needs-more-bytes, and completing it then decodes.
#[test]
fn partial_frame_is_needs_more_bytes_not_a_panic() {
    let msg = ToServer::Inbound(WireInbound::Cmd {
        conn: 1,
        actor: ObjId(1),
        line: "a reasonably long command line to make the body multi-byte".to_string(),
    });
    let full = encode_frame(&msg).unwrap();

    // Every strict prefix is incomplete: decode_frame yields Ok(None).
    for cut in 0..full.len() {
        let partial = &full[..cut];
        let got: Option<(ToServer, usize)> =
            decode_frame(partial).expect("a truncated frame is not an error");
        assert!(got.is_none(), "prefix of {cut} bytes must need more");
    }

    // Streaming: feed all-but-one byte, expect None, then the last byte yields it.
    let mut dec = FrameDecoder::new();
    dec.feed(&full[..full.len() - 1]);
    assert!(
        dec.next_message::<ToServer>().unwrap().is_none(),
        "one byte short => needs more"
    );
    dec.feed(&full[full.len() - 1..]);
    assert_eq!(dec.next_message::<ToServer>().unwrap(), Some(msg));
}

/// Fewer than the 4 length-prefix bytes is also needs-more-bytes.
#[test]
fn sub_prefix_buffer_needs_more_bytes() {
    for buf in [&[][..], &[0][..], &[0, 0][..], &[0, 0, 0][..]] {
        let got: Option<(ToServer, usize)> = decode_frame(buf).unwrap();
        assert!(got.is_none());
    }
}

/// An oversized length prefix is rejected before any allocation.
#[test]
fn oversized_length_prefix_is_rejected() {
    let bogus = ((MAX_FRAME_LEN + 1) as u32).to_be_bytes();
    let err = decode_frame::<ToServer>(&bogus).unwrap_err();
    assert!(matches!(err, mush_ipc::CodecError::FrameTooLarge(_)));
}

/// WireInbound is exactly the cross-boundary subset: the five gateway-origin
/// variants convert both ways; Apply and Control are structurally excluded.
#[test]
fn wire_inbound_is_the_cross_boundary_subset() {
    let cases = [
        Inbound::Cmd {
            conn: 1,
            actor: ObjId(2),
            line: "look".to_string(),
        },
        Inbound::Query {
            conn: 1,
            actor: ObjId(2),
            id: 5,
            name: "who".into(),
            args: serde_json::json!({}),
        },
        Inbound::ConnUp {
            conn: 1,
            actor: ObjId(2),
            caps_structured: true,
            from_linkdead: true,
        },
        Inbound::ConnDown {
            conn: 1,
            actor: ObjId(2),
            reason: DisconnectKind::Boot,
        },
        Inbound::SessionGone { actor: ObjId(2) },
    ];
    for inbound in cases {
        let wire = WireInbound::try_from(inbound.clone()).expect("crosses the boundary");
        // And back — the conversion is lossless.
        assert_eq!(Inbound::from(wire), inbound);
    }

    // The two server-internal variants are rejected with the right reason.
    assert_eq!(
        WireInbound::try_from(Inbound::Apply(WorldCommand::SlowTestDone {
            actor: ObjId(1),
            correlation: 0,
            text: String::new(),
        })),
        Err(WireInboundError::Apply),
    );
    assert_eq!(
        WireInbound::try_from(Inbound::Control(CoreControl::Shutdown)),
        Err(WireInboundError::Control),
    );
}

/// The custody envelope's payload converts to world input for exactly the two
/// replayable player-input kinds. An authoritative action has no `Inbound` form
/// at all — it enters through the coordinator — and the conversion that used to
/// `unreachable!` on it now refuses it and hands the input back, so a caller
/// holding custody of a peer-chosen frame makes a custody decision instead of
/// killing the process.
#[test]
fn a_gateway_action_is_refused_by_the_inbound_conversion() {
    let action = GatewayInput::Action(Box::new(GatewayAction {
        conn: 4,
        actor: ObjId(11),
        offered_action: ActionOfferId(CanonicalHash::from_bytes([9; 32])),
        subject: PerceivedRef::SelfActor,
        targets: vec![PerceivedRef::SelfActor],
        parameters: serde_json::json!({ "stance": "brace" }),
    }));
    assert_eq!(
        Inbound::try_from(action.clone()),
        Err(action),
        "the refused input must come back unchanged, custody intact"
    );

    // The two accepted kinds still convert field-for-field.
    assert_eq!(
        Inbound::try_from(GatewayInput::Cmd {
            conn: 4,
            actor: ObjId(11),
            line: "look".to_owned(),
        }),
        Ok(Inbound::Cmd {
            conn: 4,
            actor: ObjId(11),
            line: "look".to_owned(),
        })
    );
    assert_eq!(
        Inbound::try_from(GatewayInput::Query {
            conn: 4,
            actor: ObjId(11),
            id: 17,
            name: "who".into(),
            args: serde_json::json!({ "scope": "all" }),
        }),
        Ok(Inbound::Query {
            conn: 4,
            actor: ObjId(11),
            id: 17,
            name: "who".into(),
            args: serde_json::json!({ "scope": "all" }),
        })
    );
}

/// The bare `Inbound` (with its new derives) round-trips through the codec too,
/// so the server-internal `Apply`/`Control` arms are still serde-complete
/// (needed for any future in-process persistence of the queue).
#[test]
fn inbound_itself_round_trips_including_internal_arms() {
    for inbound in [
        Inbound::Apply(WorldCommand::ModuleAsyncDone {
            module: "worldgen".into(),
            actor: ObjId(3),
            correlation: 9,
            payload: r#"{"draft":"ok"}"#.to_string(),
        }),
        Inbound::Control(CoreControl::Dump),
    ] {
        let bytes = encode_frame(&inbound).unwrap();
        let (got, _): (Inbound, usize) = decode_frame(&bytes).unwrap().unwrap();
        assert_eq!(got, inbound);
    }
}

#[test]
fn typed_invoke_round_trips_under_custody_with_independent_size_limit() {
    let request = mush_proto::InvokeRequest::Mind(mush_proto::MindRequest::Read {
        name: "mine".into(),
        revision: Some("3".into()),
    });
    let input = GatewayInput::Invoke {
        conn: 9,
        actor: ObjId(7),
        id: 44,
        request: request.clone(),
    };
    assert_eq!(input.conn(), 9);
    assert!(input.logical_bytes().is_ok());
    round_trip_to_server(&ToServer::Inbound(WireInbound::Invoke {
        conn: 9,
        actor: ObjId(7),
        id: 44,
        request: request.clone(),
    }));
    let inbound = Inbound::try_from(input.clone()).expect("closed invocation");
    assert_eq!(
        WireInbound::try_from(inbound).expect("wire"),
        WireInbound::Invoke {
            conn: 9,
            actor: ObjId(7),
            id: 44,
            request
        }
    );
    let illegal = GatewayInput::Invoke {
        conn: 9,
        actor: ObjId(7),
        id: 44,
        request: mush_proto::InvokeRequest::Mind(mush_proto::MindRequest::Create {
            name: "x".repeat(mush_proto::MAX_INVOKE_BYTES),
            from: None,
        }),
    };
    assert!(
        illegal.logical_bytes().is_err(),
        "bounded before world dispatch"
    );
}

#[test]
fn licence_source_request_uses_v8_custody_and_closed_v7_request_cannot_decode_it() {
    let request = mush_proto::InvokeRequest::Mind(mush_proto::MindRequest::FiledSource {
        licence: "2".into(),
        first: 1,
        last: 200,
    });
    round_trip_to_server(&ToServer::Inbound(WireInbound::Invoke {
        conn: 9,
        actor: ObjId(7),
        id: 45,
        request: request.clone(),
    }));
    // A deliberately narrow old vocabulary proves the relevant decoding
    // difference, while the existing complete old golden/roundtrip suite pins
    // every unchanged request. This is not an old executable compatibility run.
    #[derive(Debug, serde::Deserialize)]
    #[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
    enum OldMind {
        Read {
            name: String,
            revision: Option<String>,
        },
    }
    let old = serde_json::json!({"op":"read","name":"mine","revision":null});
    let OldMind::Read { name, revision } =
        serde_json::from_value::<OldMind>(old).expect("old read");
    assert_eq!(name, "mine");
    assert!(revision.is_none());
    let value = serde_json::to_value(request).expect("new request");
    assert!(
        serde_json::from_value::<OldMind>(value.get("request").expect("nested request").clone())
            .is_err()
    );
    assert_eq!(IPC_VERSION, 12);
}

#[test]
fn attributed_remove_requires_new_closed_invoke_vocabulary()
-> Result<(), Box<dyn std::error::Error>> {
    #[derive(serde::Deserialize)]
    #[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
    enum OldMind {
        Read {
            name: String,
            revision: Option<String>,
        },
    }
    let request = mush_proto::InvokeRequest::Mind(mush_proto::MindRequest::Remove {
        name: "mine".into(),
    });
    let wire = serde_json::to_value(&request)?;
    assert_eq!(
        serde_json::from_value::<mush_proto::InvokeRequest>(wire.clone())?,
        request
    );
    assert!(
        serde_json::from_value::<OldMind>(wire.get("request").ok_or("request")?.clone()).is_err()
    );
    let old: OldMind =
        serde_json::from_value(serde_json::json!({"op":"read","name":"mine","revision":null}))?;
    let OldMind::Read { name, revision } = old;
    assert_eq!(name, "mine");
    assert!(revision.is_none());
    let reply = mush_proto::InvokeResult::Mind(mush_proto::MindResponse::RemovalRequested {
        name: "mine".into(),
    });
    let bytes = serde_json::to_vec(&reply)?;
    assert_eq!(
        serde_json::from_slice::<mush_proto::InvokeResult>(&bytes)?,
        reply
    );
    assert_eq!(IPC_VERSION, 12);
    Ok(())
}
