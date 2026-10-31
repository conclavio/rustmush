//! The sample frames shared by `roundtrip.rs` (the codec round trip) and
//! `golden.rs` (the recorded wire contract, doc 33 §4.1): one of every
//! [`ToServer`] and [`ToGateway`] variant, including the `serde_json::Value`
//! arms. A new variant is added HERE, once, and both suites pick it up: the
//! round trip proves it survives the codec, the golden suite refuses to pass
//! until its recorded sample exists.
//!
//! Not a test target of its own: cargo compiles `tests/<name>/mod.rs` only
//! where a `tests/<name>.rs` declares `mod <name>;`.

use mush_core::{
    AccountId, ActionOfferId, ControlMsg as CoreControl, DisconnectKind, ObjId, PerceivedRef,
    ProjectedLegalAction,
};
use mush_ipc::{
    AttachRec, CanonicalHash, CommitToken, CreatePlayerError, DetachCause, DiscordControlOp,
    DiscordInteraction, DiscordInteractionError, GatewayAction, GatewayInput, GatewayRequest,
    GatewayRequestDisposition, IPC_VERSION, LegalActionsError, RelayError, RelaySpeak, RequestUid,
    ServiceReq, ServiceResp, ToGateway, ToServer, WireInbound,
};
use mush_proto::{
    ActionStatusId, InvokeRequest, MindRequest, NarrativeKind, OutputEvent, PanelFrameKind,
    ProjectedActionReceipt, ProjectedActionResult, ProjectedActionStatus, RespondChoice, RichText,
};

/// The closed module request the `Invoke` samples carry.
fn sample_invoke() -> InvokeRequest {
    InvokeRequest::Mind(MindRequest::Read {
        name: "mine".into(),
        revision: Some("3".into()),
    })
}

pub fn sample_status() -> ActionStatusId {
    ActionStatusId::new("k5:toy:brace:0000000000001234:01010101010101010101010101010101")
        .expect("sample action status")
}

pub fn sample_receipt() -> ProjectedActionReceipt {
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

/// One of every `ToServer` variant, including the `serde_json::Value` arm.
pub fn every_to_server() -> Vec<ToServer> {
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
        ToServer::Inbound(WireInbound::Invoke {
            conn: 9,
            actor: ObjId(7),
            id: 45,
            request: sample_invoke(),
        }),
        ToServer::Inbound(WireInbound::Composing {
            conn: 7,
            actor: ObjId(42),
            active: true,
        }),
        ToServer::Inbound(WireInbound::Respond {
            epoch: "epoch-1".to_string(),
            conn: 7,
            actor: ObjId(42),
            id: 3,
            choice: RespondChoice::Confirm(true),
        }),
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
        ToServer::Input(GatewayRequest::new(
            0x1234,
            10,
            GatewayInput::Invoke {
                conn: 9,
                actor: ObjId(7),
                id: 46,
                request: sample_invoke(),
            },
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
                output_brackets: mush_proto::OutputBrackets {
                    prefix: Some("begin".into()),
                    suffix: Some("end".into()),
                },
                conn: 5,
                actor: ObjId(11),
                account: Some("nick".to_string()),
                char_name: "Wraith".to_string(),
                caps_structured: true,
            },
            AttachRec {
                output_brackets: Default::default(),
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
            id: 108,
            req: ServiceReq::ReplyBarrier { conn: 5 },
        },
        ToServer::Request {
            id: 100,
            req: ServiceReq::CreatePlayer {
                // 16b cut F.4: the account rides on the create, so the id has to
                // survive the encoding like every other field of the frame.
                name: "Newbie".to_string(),
                account: AccountId::mint("Nick", 1_700_000_000).ok(),
            },
        },
        ToServer::Request {
            // 16b cut F.4: the compensating half of the create. It is the only
            // thing that keeps the live oracle and the account documents in step
            // when the gateway's own link write fails, so it has to cross.
            id: 107,
            req: ServiceReq::UnlinkPlayer {
                character: ObjId(99),
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
        ToServer::Request {
            id: 109,
            req: ServiceReq::DiscordInteraction(DiscordInteraction {
                user_id: 1_234_567_890,
                account: Some("nick".to_string()),
                actor: Some(ObjId(11)),
                custom_id: "d:oncall:ack:7".to_string(),
                values: vec!["yes".to_string()],
            }),
        },
        ToServer::Request {
            id: 110,
            req: ServiceReq::RelaySpeak(RelaySpeak {
                message_id: Some(99),
                dm_conn: None,
                user_id: 1_234_567_890,
                account: AccountId::mint("Nick", 1_700_000_000).expect("sample account id"),
                actor: ObjId(11),
                channel: "Public".to_string(),
                kind: NarrativeKind::Say,
                text: "hello from Discord".to_string(),
            }),
        },
        ToServer::Control(CoreControl::Shutdown),
        ToServer::Control(CoreControl::Dump),
    ]
}

pub fn every_to_gateway() -> Vec<ToGateway> {
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
        ToGateway::SetOutputBrackets {
            conn: 5,
            brackets: mush_proto::OutputBrackets {
                prefix: Some("begin".into()),
                suffix: Some("end".into()),
            },
        },
        ToGateway::SetOutputBrackets {
            conn: 5,
            brackets: Default::default(),
        },
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
            id: 107,
            resp: ServiceResp::UnlinkPlayer(true),
        },
        ToGateway::Reply {
            id: 107,
            resp: ServiceResp::UnlinkPlayer(false),
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
            id: 109,
            resp: ServiceResp::DiscordInteraction(Ok("Acknowledged.".to_string())),
        },
        ToGateway::Reply {
            id: 109,
            resp: ServiceResp::DiscordInteraction(Err(DiscordInteractionError::UnknownPrefix)),
        },
        ToGateway::Reply {
            id: 110,
            resp: ServiceResp::RelaySpeak(Ok(())),
        },
        ToGateway::Reply {
            id: 110,
            resp: ServiceResp::RelaySpeak(Err(RelayError::AccountMismatch)),
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
        ToGateway::DiscordControl {
            char: 42,
            op: DiscordControlOp::Follow,
        },
        ToGateway::DiscordControl {
            char: 42,
            op: DiscordControlOp::Quiet(true),
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
        ToGateway::WorldHealth {
            world_loop_unixtime: 1_700_000_000,
            loop_lag_ms: 19,
            store_ok: false,
            integrity_ok: true,
        },
        ToGateway::Bye,
    ]
}
