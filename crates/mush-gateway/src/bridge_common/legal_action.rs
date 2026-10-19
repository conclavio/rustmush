//! Opaque public offers enter the existing replay/authoritative coordinator.
//! No module invoke, command text, provider selector, or caller actor crosses it.
use super::*;
use mush_core::{ActionOfferId, CanonicalHash};
use mush_proto::{InvokeResult, LegalActionRequest, LegalActionResponse};

impl<A: AuthBackend + 'static> GameBridge<A> {
    pub(super) async fn legal_action_invoke(
        &self,
        conn: ConnId,
        actor: ObjId,
        id: u64,
        request: &LegalActionRequest,
    ) {
        let (response, receipt) = match request {
            LegalActionRequest::SubmitSelf { offer, parameters } => {
                let converted = CanonicalHash::from_hex(offer.as_str())
                    .map(ActionOfferId)
                    .map_err(|_| ())
                    .and_then(|offer| {
                        parameters
                            .to_json()
                            .map(|parameters| (offer, parameters))
                            .map_err(|_| ())
                    });
                let Ok((offered_action, parameters)) = converted else {
                    self.legal_action_reply(conn, actor, id, LegalActionResponse::Invalid {});
                    return;
                };
                let outcome = self
                    .link
                    .submit_action(GatewayAction {
                        conn: conn.0,
                        actor,
                        offered_action,
                        subject: PerceivedRef::SelfActor,
                        targets: Vec::new(),
                        parameters,
                    })
                    .await;
                match outcome {
                    // This is split transport replay custody, not durable ownership.
                    ActionSend::Accepted => (LegalActionResponse::Queued {}, None),
                    ActionSend::Owned(reference) => {
                        (LegalActionResponse::Owned { reference }, None)
                    }
                    ActionSend::Finished(receipt) => (committed(&receipt), Some(receipt)),
                    ActionSend::Busy => (LegalActionResponse::Busy {}, None),
                    ActionSend::Rejected => (LegalActionResponse::Rejected {}, None),
                    // Gone also covers an ambiguous/closed coordinator watch. It
                    // cannot establish NoIntent or authorize automatic resubmission.
                    ActionSend::Gone => {
                        (LegalActionResponse::Unavailable { reference: None }, None)
                    }
                }
            }
            LegalActionRequest::Status { reference } => {
                match self
                    .link
                    .request(ServiceReq::ActionStatus {
                        conn: conn.0,
                        actor,
                        status: reference.clone(),
                    })
                    .await
                {
                    Some(ServiceResp::ActionStatus(ProjectedActionStatus::Committed(receipt))) => {
                        (committed(&receipt), Some(receipt))
                    }
                    Some(ServiceResp::ActionStatus(ProjectedActionStatus::Pending {
                        reference,
                    })) => (LegalActionResponse::Pending { reference }, None),
                    Some(ServiceResp::ActionStatus(ProjectedActionStatus::Unavailable {
                        reference,
                    })) => (
                        LegalActionResponse::Unavailable {
                            reference: Some(reference),
                        },
                        None,
                    ),
                    Some(ServiceResp::ActionStatus(ProjectedActionStatus::Unknown)) => {
                        (LegalActionResponse::Unknown {}, None)
                    }
                    Some(ServiceResp::ActionStatus(ProjectedActionStatus::Expired)) => {
                        (LegalActionResponse::Expired {}, None)
                    }
                    Some(ServiceResp::Busy) => (LegalActionResponse::Busy {}, None),
                    _ => (
                        LegalActionResponse::Unavailable {
                            reference: Some(reference.clone()),
                        },
                        None,
                    ),
                }
            }
        };
        // A response from an old physical attachment cannot reach a replacement
        // actor on the same connection. Server checks the binding independently.
        if self.actor_of(conn) != Some(actor) {
            return;
        }
        if let Some(receipt) = receipt {
            self.action_receipt(conn, actor, &receipt);
        }
        self.legal_action_reply(conn, actor, id, response);
    }

    pub(super) fn legal_action_reply(
        &self,
        conn: ConnId,
        actor: ObjId,
        id: u64,
        response: LegalActionResponse,
    ) {
        if self.actor_of(conn) == Some(actor) {
            send_event_to_conn(
                &self.sessions,
                conn,
                OutputEvent::InvokeResult {
                    id,
                    result: InvokeResult::LegalAction(response).bounded(),
                },
            );
        }
    }
}

fn committed(receipt: &ProjectedActionReceipt) -> LegalActionResponse {
    match ProjectedCommitToken::new(receipt.commit()) {
        Ok(commit) => LegalActionResponse::Committed {
            reference: receipt.status().clone(),
            commit,
        },
        Err(_) => LegalActionResponse::Unavailable {
            reference: Some(receipt.status().clone()),
        },
    }
}
