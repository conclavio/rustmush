//! Admission counts durable source reservations and accepted envelopes once.
//! The physical global cap is unchanged. External data cannot consume tactical
//! headroom; new data cannot consume the control reserve. No client selects pools.
use crate::{Envelope, EnvelopeState, PropagationStore, dispatch, record::DispatchKind};
use mush_core::{ObjId, RelayRepresentation, TransportRefusal, World};
use std::collections::BTreeSet;

pub const MAX_PRINCIPAL_PENDING: usize = 32;
pub const MAX_PRODUCER_PENDING: usize = 48;
pub const MAX_EXTERNAL_DATA: usize = 64;
pub const CONTROL_RESERVE: usize = 16;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Producer {
    Mail,
    Channel,
    Tactical,
    Control,
}
impl Producer {
    fn external(self) -> bool {
        matches!(self, Self::Mail | Self::Channel)
    }
}
pub fn producer(kind: DispatchKind, key: Option<&str>) -> Producer {
    match kind {
        DispatchKind::Acknowledgement | DispatchKind::DetachmentReply => Producer::Control,
        DispatchKind::Mail if key.is_some_and(|k| k.ends_with(":return")) => Producer::Control,
        DispatchKind::Mail => Producer::Mail,
        DispatchKind::Channel => Producer::Channel,
        _ => Producer::Tactical,
    }
}
fn active(e: &Envelope) -> bool {
    matches!(
        e.state(),
        EnvelopeState::Queued | EnvelopeState::Transmitted
    )
}
#[derive(Clone, Copy)]
struct Entry {
    actor: ObjId,
    representation: Option<RelayRepresentation>,
    producer: Producer,
    control: bool,
}
/// Includes not-yet-handed-off mail/channel copies, preventing between-fire
/// overbooking by orders as well as by either external producer. Existing work
/// is grandfathered on upgrade and never dropped merely for exceeding a new quota.
pub fn check(
    w: &World,
    actor: ObjId,
    representation: Option<RelayRepresentation>,
    pool: Producer,
    additional: usize,
) -> Result<(), TransportRefusal> {
    let book = w
        .module_state::<PropagationStore>()
        .ok_or(TransportRefusal::Unavailable)?;
    let requests = match w.module_state::<dispatch::Services>() {
        Some(s) => s.0.requests(w).ok_or(TransportRefusal::Unavailable)?,
        None => vec![],
    };
    if requests.len() > dispatch::MAX_DISPATCHES {
        return Err(TransportRefusal::Unavailable);
    }
    let accepted: BTreeSet<_> = book
        .envelopes()
        .filter_map(|e| e.dispatch_ref().map(|d| d.key.as_str()))
        .collect();
    let reserved: Vec<_> = requests
        .iter()
        .filter(|r| !accepted.contains(r.reference.key.as_str()))
        .collect();
    // Source reservations remain charged through terminal receipt consumption.
    // In particular, mail owns ONE slot through outbound -> return -> completion;
    // changing the envelope key cannot free the sender's or producer's allowance.
    let source_keys: BTreeSet<_> = requests.iter().map(|r| r.reference.key.as_str()).collect();
    let mut entries: Vec<_> = book
        .envelopes()
        .filter(|e| {
            active(e)
                && e.dispatch_ref()
                    .is_none_or(|d| !source_keys.contains(d.key.as_str()))
        })
        .map(|e| Entry {
            actor: e.issuer(),
            representation: e.representation(),
            producer: producer(e.kind(), None),
            control: producer(e.kind(), e.dispatch_ref().map(|r| r.key.as_str()))
                == Producer::Control,
        })
        .collect();
    entries.extend(requests.iter().map(|r| Entry {
        actor: r.reservation_actor,
        representation: r.reservation_representation,
        producer: producer(r.kind, None),
        control: producer(r.kind, Some(&r.reference.key)) == Producer::Control,
    }));
    if entries.len().saturating_add(additional) > crate::routing::MAX_PENDING_ROUTES
        || book
            .envelopes()
            .count()
            .saturating_add(reserved.len())
            .saturating_add(additional)
            > crate::MAX_ENVELOPES
        || (pool.external()
            && book
                .envelopes()
                .filter(|e| {
                    matches!(e.kind(), DispatchKind::Mail | DispatchKind::Channel)
                        && e.dispatch_ref().is_some()
                })
                .count()
                .saturating_add(reserved.len())
                .saturating_add(additional)
                > dispatch::MAX_DISPATCHES)
    {
        return Err(TransportRefusal::Global);
    }
    if pool != Producer::Control {
        let data: Vec<_> = entries
            .iter()
            .filter(|e| e.producer != Producer::Control)
            .collect();
        if data
            .iter()
            .filter(|e| e.actor == actor)
            .count()
            .saturating_add(additional)
            > MAX_PRINCIPAL_PENDING
            || representation.is_some_and(|r| {
                data.iter()
                    .filter(|e| {
                        e.representation
                            .is_some_and(|other| other.organization == r.organization)
                    })
                    .count()
                    .saturating_add(additional)
                    > MAX_PRINCIPAL_PENDING
            })
        {
            return Err(TransportRefusal::Principal);
        }
        if entries
            .iter()
            .filter(|e| !e.control)
            .count()
            .saturating_add(additional)
            > crate::routing::MAX_PENDING_ROUTES - CONTROL_RESERVE
        {
            return Err(TransportRefusal::ControlReserve);
        }
        if pool.external()
            && (data
                .iter()
                .filter(|e| e.producer == pool)
                .count()
                .saturating_add(additional)
                > MAX_PRODUCER_PENDING
                || data
                    .iter()
                    .filter(|e| e.producer.external())
                    .count()
                    .saturating_add(additional)
                    > MAX_EXTERNAL_DATA)
        {
            return Err(TransportRefusal::Producer);
        }
    }
    Ok(())
}
