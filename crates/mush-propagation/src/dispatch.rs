//! Durable cross-owner handoff. Sources retain intents until consuming a terminal
//! envelope; envelopes retain receipts until the source withdraws that intent.
//! The existing delivery tick is the only transport scheduler.
use crate::{Envelope, PropagationStore, record::DispatchKind};
use mush_core::{ClockStamp, Ctx, ModuleOp, ModuleWrite, ObjId, World};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, sync::Arc};

pub const MAX_DISPATCHES: usize = 256;
pub const MAX_ACCEPT_PER_TICK: usize = 16;
pub const SHORT_MESSAGE_BYTES: u32 = 1024;
pub const MAX_CONTENT_BYTES: u32 = 66_048;

/// Versioned reference and frozen accounting/endpoint inputs; never content or
/// authority to read content. An absent origin/destination remains unresolved.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DispatchRef {
    pub version: u32,
    pub key: String,
    pub bytes: u32,
    pub origin: Option<ObjId>,
    pub fallback: Option<ObjId>,
}
impl DispatchRef {
    pub(crate) fn valid_for(&self, kind: DispatchKind) -> bool {
        self.valid()
            && match kind {
                DispatchKind::Mail | DispatchKind::Channel => true,
                DispatchKind::Detachment | DispatchKind::DetachmentReply => {
                    self.key.starts_with("dt:") && self.origin.is_none() && self.fallback.is_none()
                }
                _ => false,
            }
    }
    pub fn valid(&self) -> bool {
        self.version == 1
            && !self.key.is_empty()
            && self.key.len() <= 96
            && self
                .key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b':' || b == b'_')
            && (1..=MAX_CONTENT_BYTES).contains(&self.bytes)
    }
}

#[derive(Clone, Debug)]
pub struct Request {
    /// Source reservation ownership can differ from a returning receipt issuer.
    pub reservation_actor: ObjId,
    pub reservation_representation: Option<mush_core::RelayRepresentation>,
    pub representation: Option<mush_core::RelayRepresentation>,
    pub reference: DispatchRef,
    pub kind: DispatchKind,
    pub issuer: ObjId,
    pub recipient: ObjId,
    pub issued: ClockStamp,
}
pub trait Sources: Send + Sync + std::fmt::Debug {
    /// Complete, bounded snapshot. None means unavailable: retain all receipts.
    fn requests(&self, world: &World) -> Option<Vec<Request>>;
}
#[derive(Clone, Debug)]
pub struct Services(pub Arc<dyn Sources>);

pub fn find<'a>(world: &'a World, key: &str) -> Option<&'a Envelope> {
    world
        .module_state::<PropagationStore>()?
        .envelopes()
        .find(|e| e.dispatch_ref().is_some_and(|d| d.key == key))
}

pub(crate) fn reconcile(ctx: &mut Ctx<'_>) {
    let Some(services) = ctx.state::<Services>().cloned() else {
        return;
    };
    let Some(mut requests) = services.0.requests(ctx.w) else {
        return;
    };
    if requests.len() > MAX_DISPATCHES
        || requests.iter().any(|r| {
            !r.reference.valid() || !matches!(r.kind, DispatchKind::Mail | DispatchKind::Channel)
        })
    {
        return;
    }
    requests.sort_by(|a, b| a.reference.key.cmp(&b.reference.key));
    if requests
        .windows(2)
        .any(|r| r[0].reference.key == r[1].reference.key)
    {
        return;
    }
    let wanted: BTreeSet<_> = requests.iter().map(|r| r.reference.key.as_str()).collect();
    let Some(mut book) = ctx.state::<PropagationStore>().cloned() else {
        return;
    };
    // Withdrawal is the source's durable acknowledgement, including cancellation.
    let released: Vec<_> = book
        .envelopes()
        .filter(|e| matches!(e.kind(), DispatchKind::Mail | DispatchKind::Channel))
        .filter(|e| {
            e.dispatch_ref()
                .is_some_and(|d| !wanted.contains(d.key.as_str()))
        })
        .map(Envelope::id)
        .collect();
    for id in released {
        book.drop_envelope(id);
        ctx.persist(ModuleOp {
            ns: crate::NS.into(),
            op: ModuleWrite::Del {
                key: crate::envelope_key(id),
            },
        });
    }
    requests.sort_by(|a, b| (a.issued.at, &a.reference.key).cmp(&(b.issued.at, &b.reference.key)));
    let now = ctx.w.world_ic_now();
    let mut accepted = 0;
    // Requests that cannot yet route are still accepted into the bounded queue;
    // routing/expiry, including unknown positions, belongs to the existing pass.
    for request in requests {
        if accepted >= MAX_ACCEPT_PER_TICK {
            break;
        }
        if book.envelopes().any(|e| {
            e.dispatch_ref()
                .is_some_and(|d| d.key == request.reference.key)
        }) {
            continue;
        }
        if book
            .envelopes()
            .filter(|e| {
                matches!(e.kind(), DispatchKind::Mail | DispatchKind::Channel)
                    && e.dispatch_ref().is_some()
            })
            .count()
            >= MAX_DISPATCHES
            || crate::routing::admission(&book, None, None).is_err()
            || book.envelopes().count() >= crate::MAX_ENVELOPES
        {
            break;
        }
        let Some(id) = book.alloc_envelope() else {
            break;
        };
        let mut issued = request.issued;
        issued.clock = now.clock.clone();
        let Ok(envelope) = Envelope::dispatch(
            id,
            request.issuer,
            request.recipient,
            request.kind,
            request.reference,
            issued,
        ) else {
            continue;
        };
        let envelope = envelope.with_representation(request.representation);
        let Ok(value) = serde_json::to_value(&envelope) else {
            continue;
        };
        if book.put_envelope(envelope).is_err() {
            break;
        }
        ctx.persist(ModuleOp {
            ns: crate::NS.into(),
            op: ModuleWrite::JsonSet {
                key: crate::envelope_key(id),
                value,
            },
        });
        accepted += 1;
    }
    if accepted > 0 {
        ctx.persist(ModuleOp {
            ns: crate::NS.into(),
            op: ModuleWrite::JsonSet {
                key: crate::ALLOCATOR_KEY.into(),
                value: book.allocator(),
            },
        });
    }
    if let Some(slot) = ctx.state_mut::<PropagationStore>() {
        *slot = book;
    }
}
