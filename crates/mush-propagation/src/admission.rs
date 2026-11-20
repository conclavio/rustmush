//! Admission counts durable source reservations and accepted envelopes once.
//! The physical global cap is unchanged. External data cannot consume tactical
//! headroom; new data cannot consume the control reserve. No client selects pools.
use crate::{Envelope, EnvelopeState, PropagationStore, dispatch, record::DispatchKind};
use mush_core::{ObjId, RelayRepresentation, TransportRefusal, World};
use std::collections::BTreeSet;

pub const MAX_PRINCIPAL_PENDING: usize = 32;
/// Per-principal cap on active pending control traffic (HIGH-67): strictly below
/// [`MAX_PRINCIPAL_PENDING`] (32) and [`crate::routing::MAX_PENDING_ROUTES`] (128) while allowing
/// a recipient to acknowledge up to [`CONTROL_RESERVE`] distinct delivered orders.
pub const MAX_PRINCIPAL_CONTROL_PENDING: usize = 16;
/// Per-principal cap on retained (non-reapable) envelopes in [`PropagationStore`] (HIGH-67):
/// strictly below [`crate::MAX_ENVELOPES`] so one actor cannot exhaust the global pool.
pub const MAX_PRINCIPAL_ENVELOPES: usize = 256;
/// Per-organization cap on retained (non-reapable) envelopes (HIGH-67).
pub const MAX_ORG_ENVELOPES: usize = 512;
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
    let now = w.world_ic_now();
    let retained_actor = book
        .envelopes()
        .filter(|e| e.issuer() == actor && !crate::pass::is_reapable(e, &now))
        .count()
        .saturating_add(
            reserved
                .iter()
                .filter(|r| r.reservation_actor == actor)
                .count(),
        );
    if retained_actor.saturating_add(additional) > MAX_PRINCIPAL_ENVELOPES
        || representation.is_some_and(|r| {
            book.envelopes()
                .filter(|e| {
                    !crate::pass::is_reapable(e, &now)
                        && e.representation()
                            .is_some_and(|other| other.organization == r.organization)
                })
                .count()
                .saturating_add(
                    reserved
                        .iter()
                        .filter(|req| {
                            req.reservation_representation
                                .is_some_and(|other| other.organization == r.organization)
                        })
                        .count(),
                )
                .saturating_add(additional)
                > MAX_ORG_ENVELOPES
        })
    {
        return Err(TransportRefusal::Principal);
    }
    if pool == Producer::Control {
        let control_entries: Vec<_> = entries.iter().filter(|e| e.control).collect();
        if control_entries
            .iter()
            .filter(|e| e.actor == actor)
            .count()
            .saturating_add(additional)
            > MAX_PRINCIPAL_CONTROL_PENDING
            || representation.is_some_and(|r| {
                control_entries
                    .iter()
                    .filter(|e| {
                        e.representation
                            .is_some_and(|other| other.organization == r.organization)
                    })
                    .count()
                    .saturating_add(additional)
                    > MAX_PRINCIPAL_CONTROL_PENDING
            })
        {
            return Err(TransportRefusal::Principal);
        }
    } else {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::DispatchRef;

    fn ref_key(key: &str) -> DispatchRef {
        DispatchRef {
            version: 1,
            key: key.to_owned(),
            bytes: 64,
            origin: None,
            fallback: None,
        }
    }

    struct TestPropModule(Option<PropagationStore>);
    impl mush_core::GameModule for TestPropModule {
        fn name(&self) -> &'static str {
            "propagation"
        }
        fn register(
            &mut self,
            reg: &mut mush_core::Registrar<'_>,
        ) -> Result<(), mush_core::ModError> {
            if let Some(store) = self.0.take() {
                reg.state(store);
            }
            Ok(())
        }
    }

    fn world_with_store(store: PropagationStore) -> World {
        mush_core::Engine::with_modules(
            World::new_minimal(),
            mush_core::Registry::new(),
            vec![Box::new(TestPropModule(Some(store)))],
        )
        .expect("engine")
        .world
    }

    #[test]
    fn high67_control_pool_and_retained_pool_enforce_per_principal_quotas() {
        let base = World::new_minimal();
        let attacker = ObjId(10);
        let bystander = ObjId(20);
        let now = base.world_ic_now();
        let mut store = PropagationStore::default();
        for idx in 1..=MAX_PRINCIPAL_CONTROL_PENDING {
            let id = store.alloc_envelope().expect("alloc");
            let env = Envelope::dispatch(
                id,
                attacker,
                bystander,
                DispatchKind::DetachmentReply,
                ref_key(&format!("dt:1:1:1:{idx}:1:reply:wilco")),
                now.clone(),
            )
            .expect("envelope");
            store.put_envelope(env).expect("put");
        }
        let w = world_with_store(store);

        // Attacker hits per-principal control pending cap well before CONTROL_RESERVE (16) or 128.
        assert_eq!(
            check(&w, attacker, None, Producer::Control, 1),
            Err(TransportRefusal::Principal)
        );
        // Bystander can still use the control reserve unaffected.
        assert_eq!(check(&w, bystander, None, Producer::Control, 1), Ok(()));

        // Delivered detachment envelopes within FIELD_ENVELOPE_TTL_SECS still count toward
        // MAX_PRINCIPAL_ENVELOPES (256), preventing one actor from exhausting MAX_ENVELOPES (4,096).
        let mut delivered_store = PropagationStore::default();
        for idx in 1..=MAX_PRINCIPAL_ENVELOPES {
            let id = delivered_store.alloc_envelope().expect("alloc");
            let env = Envelope::dispatch(
                id,
                attacker,
                bystander,
                DispatchKind::Detachment,
                ref_key(&format!("dt:1:1:1:{idx}:1")),
                now.clone(),
            )
            .expect("envelope")
            .in_state(EnvelopeState::Delivered, now.clone());
            delivered_store.put_envelope(env).expect("put");
        }
        let w = world_with_store(delivered_store);
        assert_eq!(
            check(&w, attacker, None, Producer::Tactical, 1),
            Err(TransportRefusal::Principal)
        );
        assert_eq!(
            check(&w, attacker, None, Producer::Control, 1),
            Err(TransportRefusal::Principal)
        );
        assert_eq!(check(&w, bystander, None, Producer::Tactical, 1), Ok(()));
    }
}
