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
    ProjectedLegalAction, WorldCommand,
};
use mush_ipc::codec::{decode_frame, encode_frame_into};
use mush_ipc::{
    AttachRec, CanonicalHash, CommitToken, CreatePlayerError, DetachCause, FrameDecoder,
    GatewayAction, GatewayInput, GatewayRequest, GatewayRequestDisposition, IPC_VERSION,
    LegalActionsError, MAX_FRAME_LEN, MAX_IN_FLIGHT_GATEWAY_INPUT_BYTES,
    MAX_IN_FLIGHT_GATEWAY_INPUTS, MAX_IN_FLIGHT_LIFECYCLE_OBLIGATIONS,
    MAX_IN_FLIGHT_SERVICE_REQUESTS, RequestUid, ServiceReq, ServiceResp, ToGateway, ToServer,
    WireInbound, WireInboundError, encode_frame,
};
use mush_proto::{
    ActionStatusId, OutputEvent, PanelFrameKind, ProjectedActionReceipt, ProjectedActionResult,
    ProjectedActionStatus, RichText,
};

fn sample_status() -> ActionStatusId {
    ActionStatusId::new("k5:toy:brace:0000000000001234:01010101010101010101010101010101")
        .expect("sample action status")
}

fn sample_receipt() -> ProjectedActionReceipt {
    ProjectedActionReceipt::new(
        sample_status(),
        CommitToken::from_bytes([2; 16]).to_string(),
        Some(
            ProjectedActionResult::new(
                "brace",
                "Brace committed.",
                serde_json::json!({"braced": true}),
            )
            .expect("sample projected result"),
        ),
    )
    .expect("sample projected receipt")
}

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

/// One of every `ToServer` variant, including the `serde_json::Value` arm.
fn every_to_server() -> Vec<ToServer> {
    let command = GatewayRequest::new(
        0x1234,
        7,
        GatewayInput::Cmd {
            conn: 7,
            actor: ObjId(42),
            line: "say replay me".to_string(),
        },
    );
    vec![
        ToServer::Inbound(WireInbound::Cmd {
            conn: 7,
            actor: ObjId(42),
            line: "look".to_string(),
        }),
        // The serde_json::Value-carrying inbound.
        ToServer::Inbound(WireInbound::Query {
            conn: 9,
            actor: ObjId(3),
            id: 1234,
            name: "who".into(),
            args: serde_json::json!({ "filter": "connected", "limit": 20, "flags": [1, 2, 3] }),
        }),
        ToServer::Inbound(WireInbound::ConnUp {
            conn: 1,
            actor: ObjId(2),
            caps_structured: true,
            from_linkdead: false,
        }),
        ToServer::Inbound(WireInbound::ConnDown {
            conn: 1,
            actor: ObjId(2),
            reason: DisconnectKind::Lost,
        }),
        ToServer::Inbound(WireInbound::SessionGone { actor: ObjId(2) }),
        ToServer::Input(command.clone()),
        ToServer::Input(GatewayRequest::new(
            0x1234,
            8,
            GatewayInput::Query {
                conn: 7,
                actor: ObjId(42),
                id: 55,
                name: "who".into(),
                args: serde_json::json!({"page": 2}),
            },
        )),
        ToServer::Input(GatewayRequest::new(
            0x1234,
            9,
            GatewayInput::Action(Box::new(GatewayAction {
                conn: 7,
                actor: ObjId(42),
                offered_action: ActionOfferId(CanonicalHash::from_bytes([7; 32])),
                subject: PerceivedRef::SelfActor,
                targets: vec![],
                parameters: serde_json::json!({}),
            })),
        )),
        ToServer::GatewayReleased {
            request: command.uid,
        },
        ToServer::Attach {
            conn: 5,
            actor: ObjId(11),
            account: Some("nick".to_string()),
            char_name: "Wraith".to_string(),
            caps_structured: true,
        },
        ToServer::Attach {
            conn: 6,
            actor: ObjId(12),
            account: None,
            char_name: "Anon".to_string(),
            caps_structured: false,
        },
        ToServer::Detach {
            conn: 5,
            actor: ObjId(11),
            reason: DisconnectKind::Quit,
        },
        ToServer::ReAnnounce(vec![
            AttachRec {
                conn: 5,
                actor: ObjId(11),
                account: Some("nick".to_string()),
                char_name: "Wraith".to_string(),
                caps_structured: true,
            },
            AttachRec {
                conn: 6,
                actor: ObjId(12),
                account: None,
                char_name: "Anon".to_string(),
                caps_structured: false,
            },
        ]),
        ToServer::ReAnnounce(vec![]),
        // Each ServiceReq.
        ToServer::Request {
            id: 100,
            req: ServiceReq::CreatePlayer {
                name: "Newbie".to_string(),
            },
        },
        ToServer::Request {
            id: 101,
            req: ServiceReq::NameFree {
                name: "Wraith".to_string(),
            },
        },
        ToServer::Request {
            id: 102,
            req: ServiceReq::InInputContext { actor: ObjId(11) },
        },
        ToServer::Request {
            id: 103,
            req: ServiceReq::ConnectionReady {
                conn: 5,
                actor: ObjId(11),
            },
        },
        ToServer::Request {
            id: 104,
            req: ServiceReq::LegalActions {
                conn: 5,
                actor: ObjId(11),
                subject: PerceivedRef::SelfActor,
            },
        },
        ToServer::Request {
            id: 105,
            req: ServiceReq::ActionStatus {
                conn: 5,
                actor: ObjId(11),
                status: sample_status(),
            },
        },
        ToServer::Request {
            id: 106,
            req: ServiceReq::Barrier,
        },
        ToServer::Control(CoreControl::Shutdown),
        ToServer::Control(CoreControl::Dump),
    ]
}

/// One of every `ToGateway` variant, including the `serde_json::Value` arm.
fn every_to_gateway() -> Vec<ToGateway> {
    use mush_ipc::SendTarget;
    let request = RequestUid::from_bytes([1; 16]);
    let legal_offer: ProjectedLegalAction = serde_json::from_value(serde_json::json!({
        "offer": "0707070707070707070707070707070707070707070707070707070707070707",
        "subject": { "kind": "self_actor" },
        "parameters": [],
        "targets": { "kind": "none" },
        "required_channels": [],
        "expires": null,
        "presentation": {
            "verb": "brace",
            "label": "Brace",
            "help": "Adopt a braced tactical stance."
        }
    }))
    .expect("sample legal offer");
    vec![
        ToGateway::Deliver {
            target: SendTarget::Char(42),
            event: OutputEvent::Sys(RichText::plain("hello")),
        },
        ToGateway::Deliver {
            target: SendTarget::Conn(7),
            event: OutputEvent::Narrative {
                kind: mush_proto::NarrativeKind::Say,
                actor: None,
                text: RichText::plain("boo"),
            },
        },
        // The serde_json::Value-carrying delivery: a Panel keyframe.
        ToGateway::Deliver {
            target: SendTarget::All,
            event: OutputEvent::Panel {
                feed: "wod.roll".into(),
                kind: PanelFrameKind::Keyframe,
                data: serde_json::json!({
                    "pool": 7,
                    "successes": 3,
                    "dice": [10, 8, 2, 6, 9, 1, 4],
                    "botch": false,
                }),
            },
        },
        // Each ServiceResp.
        ToGateway::Reply {
            id: 100,
            resp: ServiceResp::CreatePlayer(Ok(ObjId(99))),
        },
        ToGateway::Reply {
            id: 100,
            resp: ServiceResp::CreatePlayer(Err(CreatePlayerError::NameTaken)),
        },
        ToGateway::Reply {
            id: 100,
            resp: ServiceResp::CreatePlayer(Err(CreatePlayerError::World("boom".to_string()))),
        },
        ToGateway::Reply {
            id: 100,
            resp: ServiceResp::CreatePlayer(Err(CreatePlayerError::WorldGone)),
        },
        ToGateway::Reply {
            id: 100,
            resp: ServiceResp::CreatePlayer(Err(CreatePlayerError::StoreUnavailable)),
        },
        ToGateway::Reply {
            id: 101,
            resp: ServiceResp::NameFree(true),
        },
        ToGateway::Reply {
            id: 102,
            resp: ServiceResp::InInputContext(false),
        },
        ToGateway::Reply {
            id: 103,
            resp: ServiceResp::ConnectionReady(true),
        },
        ToGateway::Reply {
            id: 104,
            resp: ServiceResp::LegalActions(Ok(vec![legal_offer])),
        },
        ToGateway::Reply {
            id: 104,
            resp: ServiceResp::LegalActions(Err(LegalActionsError::InvalidSubject)),
        },
        ToGateway::Reply {
            id: 105,
            resp: ServiceResp::ActionStatus(ProjectedActionStatus::Pending {
                reference: sample_status(),
            }),
        },
        ToGateway::Reply {
            id: 106,
            resp: ServiceResp::Barrier,
        },
        ToGateway::Reply {
            id: 104,
            resp: ServiceResp::Busy,
        },
        ToGateway::Reply {
            id: 105,
            resp: ServiceResp::Unavailable,
        },
        ToGateway::RequestDisposition {
            request,
            disposition: GatewayRequestDisposition::Rejected { retryable: true },
        },
        ToGateway::RequestDisposition {
            request,
            disposition: GatewayRequestDisposition::Busy,
        },
        ToGateway::RequestDisposition {
            request,
            disposition: GatewayRequestDisposition::EphemeralFinished,
        },
        ToGateway::RequestDisposition {
            request,
            disposition: GatewayRequestDisposition::LegacyClaimed,
        },
        ToGateway::RequestDisposition {
            request,
            disposition: GatewayRequestDisposition::Owned {
                token: CommitToken::from_bytes([2; 16]),
                status: sample_status(),
            },
        },
        ToGateway::RequestDisposition {
            request,
            disposition: GatewayRequestDisposition::Finished {
                recipient: ObjId(42),
                receipt: sample_receipt(),
            },
        },
        // Server → gateway session control.
        ToGateway::SetFloodExempt {
            char: 42,
            exempt: true,
        },
        ToGateway::SetFloodExempt {
            char: 42,
            exempt: false,
        },
        ToGateway::CloseSession {
            char: 11,
            cause: DetachCause::Booted,
        },
        ToGateway::Detach {
            conn: 5,
            cause: DetachCause::Booted,
        },
        ToGateway::Detach {
            conn: 6,
            cause: DetachCause::SlowClient,
        },
        ToGateway::CloseWebSessions {
            chars: vec![11, 12, 13],
            notice: Some(OutputEvent::Disconnect {
                reason: RichText::tag("system", "log in again"),
            }),
            cause: DetachCause::Booted,
        },
        ToGateway::CloseWebSessions {
            chars: vec![],
            notice: None,
            cause: DetachCause::Booted,
        },
        ToGateway::Hello {
            proto: IPC_VERSION,
            server: "mushd/0.1.0".to_string(),
            features: vec!["reattach".to_string(), "compression".to_string()],
            panels: vec!["who".to_string(), "mail".to_string()],
        },
        ToGateway::Bye,
    ]
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
    assert_eq!(IPC_VERSION, 4, "the boundary protocol version is pinned");
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
