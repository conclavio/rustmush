//! A bounded authoritative handoff into the existing propagation scheduler.
//! Source-owned DT messages acquire no contact or unit-control authority here.
use crate::{
    Envelope, EnvelopeState, PropagationStore, dispatch::DispatchRef, record::DispatchKind,
};
use mush_core::participant::{
    BoundParticipant, OwnerParticipant, OwnerPreparation, OwnerPreparationBinding,
    OwnerPrepareContext, ParticipantError,
};
use mush_core::{
    AuthoritativeKey, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef,
    DefinitionRegistryBuilder, ModError, ObjId, OwnerId, Registrar, ReplayWrite,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc};

/// Per-actor cap on in-flight (`Queued` / `Transmitted`) detachment field replies (HIGH-67):
/// strictly below [`crate::admission::CONTROL_RESERVE`] (16) so a single detachment slot
/// controller cannot exhaust the shared control reserve.
pub const MAX_ACTOR_PENDING_FIELD_REPLIES: usize = 8;

pub struct FieldTransport;
#[derive(Clone, Debug, Serialize)]
pub struct Message {
    pub recipient: ObjId,
    pub reference: DispatchRef,
}
#[derive(Clone, Debug, Serialize)]
pub enum Request {
    Send(Vec<Message>),
    Reply {
        original: u64,
        reference: DispatchRef,
    },
}
pub struct Enqueued {
    binding: OwnerPreparationBinding,
    envelopes: Vec<Envelope>,
}
impl Enqueued {
    pub fn binding(&self) -> &OwnerPreparationBinding {
        &self.binding
    }
    pub fn envelopes(&self) -> &[Envelope] {
        &self.envelopes
    }
}
pub struct Patch {
    request: Request,
    allocator: Value,
    envelopes: Vec<Envelope>,
    original: Option<Envelope>,
}
pub struct Bound<'a> {
    slot: &'a mut PropagationStore,
    after: Box<PropagationStore>,
}
impl BoundParticipant for Bound<'_> {
    fn apply(self) {
        *self.slot = *self.after;
    }
}
fn refused(message: &'static str) -> ParticipantError {
    ParticipantError::refused(message)
}
fn bad(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
pub fn definition_descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(DefinitionKind::new("participant").map_err(bad)?, DefinitionId::new("detachment_transport").map_err(bad)?,1,
        json!({"format":1,"purpose":"source_owned_field_messages","max_messages":6,"scheduler":"propagation.delivery"})).map_err(bad)
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    actions: BTreeSet<DefinitionRef>,
) -> Result<(), ModError> {
    if actions.is_empty() {
        return Ok(());
    }
    let descriptor = definition_descriptor()?;
    let expected = DefinitionRegistryBuilder::new()
        .register(OwnerId::new(crate::NS).map_err(bad)?, descriptor.clone())
        .map_err(bad)?;
    reg.definition_validator(
        descriptor.kind().clone(),
        move |ctx: &mush_core::DefinitionValidationContext<'_>| {
            if ctx.definition().reference() == &expected && !ctx.definition().is_current() {
                Ok(())
            } else {
                Err("unknown field transport participant".into())
            }
        },
    )?;
    let reference = reg.definition(descriptor)?;
    reg.owner_participant(reference, actions, FieldTransport)
}
fn selected(
    state: &PropagationStore,
    ctx: &OwnerPrepareContext<'_>,
    request: &Request,
) -> Result<(Vec<Message>, DispatchKind, Option<Envelope>), ParticipantError> {
    let actor = ctx.invocation().actor();
    if ctx
        .world()
        .get(actor)
        .is_none_or(|o| !o.kind.is_actor() || o.flags.contains(mush_core::Flags::GOING))
    {
        return Err(refused("field sender unavailable"));
    }
    let (messages, kind, original) = match request {
        Request::Send(messages) => {
            if messages.iter().any(|m| {
                state.envelopes().any(|e| {
                    e.kind() == DispatchKind::Detachment
                        && e.dispatch_ref().is_some_and(|r| r.key == m.reference.key)
                })
            }) {
                return Err(refused("duplicate field message"));
            }
            (messages.clone(), DispatchKind::Detachment, None)
        }
        Request::Reply {
            original,
            reference,
        } => {
            let envelope = state
                .envelope(*original)
                .filter(|e| {
                    e.kind() == DispatchKind::Detachment
                        && e.recipient() == actor
                        && matches!(
                            e.state(),
                            EnvelopeState::Delivered | EnvelopeState::Acknowledged
                        )
                })
                .ok_or_else(|| refused("only an actually arrived field message can be answered"))?;
            let prefix = format!(
                "{}:reply",
                envelope
                    .dispatch_ref()
                    .ok_or_else(|| refused("field reference unavailable"))?
                    .key
            );
            let suffix = reference.key.strip_prefix(&prefix);
            if !matches!(suffix, Some(":wilco" | ":negative")) {
                return Err(refused("reply does not name its original field message"));
            }
            // HIGH-67: cap per-actor in-flight field replies strictly below CONTROL_RESERVE (16)
            // so one detachment controller cannot starve unrelated `+orders/ack` control traffic.
            let pending_field_replies = state
                .envelopes()
                .filter(|env| {
                    env.issuer() == actor
                        && env.kind() == DispatchKind::DetachmentReply
                        && matches!(
                            env.state(),
                            EnvelopeState::Queued | EnvelopeState::Transmitted
                        )
                })
                .count();
            if pending_field_replies >= MAX_ACTOR_PENDING_FIELD_REPLIES {
                return Err(refused("too many pending field replies"));
            }
            // HIGH-67: deduplicate field replies per original dispatch key, and treat NEGATIVE
            // as terminal so a slot controller cannot loop WILCO/NEGATIVE to flood the pool.
            if reply_already_recorded(
                state,
                &prefix,
                reference.key.as_str(),
                suffix == Some(":wilco"),
            ) {
                return Err(refused("duplicate or superseded field reply"));
            }
            (
                vec![Message {
                    recipient: envelope.issuer(),
                    reference: reference.clone(),
                }],
                DispatchKind::DetachmentReply,
                Some(envelope.clone()),
            )
        }
    };
    if messages.is_empty()
        || messages.len() > 6
        || messages.iter().any(|m| {
            !m.reference.valid()
                || !m.reference.key.starts_with("dt:")
                || m.reference.origin.is_some()
                || m.reference.fallback.is_some()
                || ctx
                    .world()
                    .get(m.recipient)
                    .is_none_or(|o| !o.kind.is_actor() || o.flags.contains(mush_core::Flags::GOING))
        })
    {
        return Err(refused("invalid field message set"));
    }
    if messages
        .iter()
        .map(|m| m.reference.key.as_str())
        .collect::<BTreeSet<_>>()
        .len()
        != messages.len()
    {
        return Err(refused("duplicate field message"));
    }
    let representation = if crate::routing::enabled(ctx.world()) {
        crate::access::selected(ctx.world(), actor)
            .map_err(|_| refused("field transport unavailable"))?
    } else {
        None
    };
    crate::admission::check(
        ctx.world(),
        actor,
        representation,
        crate::admission::producer(kind, None),
        messages.len(),
    )
    .map_err(|_| refused("field transport queue is full or unavailable"))?;
    Ok((messages, kind, original))
}
fn write(key: &str, value: Value) -> Result<ReplayWrite, ParticipantError> {
    Ok(ReplayWrite::ModuleJsonSet {
        namespace: OwnerId::new(crate::NS).map_err(|_| ParticipantError::Encoding)?,
        key: AuthoritativeKey::parse_canonical(key).map_err(|_| ParticipantError::Encoding)?,
        value,
    })
}
fn after(
    state: &PropagationStore,
    patch: &Patch,
) -> Result<Box<PropagationStore>, ParticipantError> {
    if state.allocator() != patch.allocator
        || patch
            .original
            .as_ref()
            .is_some_and(|e| state.envelope(e.id()) != Some(e))
    {
        return Err(refused("field transport changed"));
    }
    let mut next = Box::new(state.clone());
    for envelope in &patch.envelopes {
        if next.alloc_envelope() != Some(envelope.id()) {
            return Err(refused("field transport allocator changed"));
        }
        next.put_envelope(envelope.clone())
            .map_err(|_| refused("field transport queue is full"))?;
    }
    Ok(next)
}
impl OwnerParticipant for FieldTransport {
    type State = PropagationStore;
    type Request = Request;
    type Output = Enqueued;
    type Patch = Patch;
    type Bound<'a> = Bound<'a>;
    fn prepare(
        &self,
        state: &PropagationStore,
        ctx: &OwnerPrepareContext<'_>,
        request: Request,
    ) -> Result<OwnerPreparation<Enqueued, Patch>, ParticipantError> {
        let (messages, kind, original) = selected(state, ctx, &request)?;
        let actor = ctx.invocation().actor();
        let representation = if crate::routing::enabled(ctx.world()) {
            crate::access::selected(ctx.world(), actor)
                .map_err(|_| refused("field transport unavailable"))?
        } else {
            None
        };
        let mut next = Box::new(state.clone());
        let mut envelopes = Vec::new();
        let mut writes = Vec::new();
        for message in messages {
            let id = next
                .alloc_envelope()
                .ok_or_else(|| refused("field transport allocator exhausted"))?;
            let envelope = Envelope::dispatch(
                id,
                actor,
                message.recipient,
                kind,
                message.reference,
                ctx.world().world_ic_now(),
            )
            .map_err(|_| refused("invalid field message"))?
            .with_representation(representation);
            writes.push(write(
                crate::envelope_key(id).as_str(),
                serde_json::to_value(&envelope).map_err(|_| ParticipantError::Encoding)?,
            )?);
            next.put_envelope(envelope.clone())
                .map_err(|_| refused("field transport queue is full"))?;
            envelopes.push(envelope);
        }
        writes.push(write(crate::ALLOCATOR_KEY, next.allocator())?);
        Ok(OwnerPreparation::new(
            Enqueued {
                binding: ctx.preparation_binding(),
                envelopes: envelopes.clone(),
            },
            Patch {
                request,
                allocator: state.allocator(),
                envelopes,
                original,
            },
            writes,
        ))
    }
    fn recheck(
        &self,
        state: &PropagationStore,
        ctx: &OwnerPrepareContext<'_>,
        patch: &Patch,
    ) -> Result<(), ParticipantError> {
        selected(state, ctx, &patch.request)?;
        after(state, patch).map(|_| ())
    }
    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut PropagationStore,
        patch: Patch,
    ) -> Result<Bound<'a>, ParticipantError> {
        let after = after(state, &patch)?;
        Ok(Bound { slot: state, after })
    }
    fn accepts_write(&self, write: &ReplayWrite) -> bool {
        let ReplayWrite::ModuleJsonSet {
            namespace,
            key,
            value,
        } = write
        else {
            return false;
        };
        if namespace.as_str() != crate::NS {
            return false;
        }
        if key.as_str() == crate::ALLOCATOR_KEY {
            let mut probe = PropagationStore::default();
            probe.install_allocator(value);
            return probe.allocator() == *value;
        }
        serde_json::from_value::<Envelope>(value.clone())
            .ok()
            .is_some_and(|e| {
                e.id() != 0
                    && crate::envelope_key(e.id()).as_str() == key.as_str()
                    && matches!(
                        e.kind(),
                        DispatchKind::Detachment | DispatchKind::DetachmentReply
                    )
                    && e.dispatch_ref().is_some_and(|r| r.key.starts_with("dt:"))
            })
    }
}

/// Whether a reply with `key` (or a terminal `:negative` reply when submitting `:wilco`) is
/// already recorded in `state` for this original dispatch `prefix` (HIGH-67).
fn reply_already_recorded(
    state: &PropagationStore,
    prefix: &str,
    key: &str,
    is_wilco: bool,
) -> bool {
    let negative_key = is_wilco.then(|| format!("{prefix}:negative"));
    state.envelopes().any(|e| {
        e.kind() == DispatchKind::DetachmentReply
            && e.dispatch_ref().is_some_and(|r| {
                r.key == key
                    || negative_key
                        .as_deref()
                        .is_some_and(|neg| r.key.as_str() == neg)
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::{ClockStamp, World};

    fn ref_key(key: &str) -> DispatchRef {
        DispatchRef {
            version: 1,
            key: key.to_owned(),
            bytes: 64,
            origin: None,
            fallback: None,
        }
    }

    #[test]
    fn high67_field_reply_deduplication_and_bounded_ttl() {
        let w = World::new_minimal();
        let now = w.world_ic_now();
        let mut store = PropagationStore::default();
        let prefix = "dt:1:1:1:1:1:reply";
        let wilco = format!("{prefix}:wilco");
        let negative = format!("{prefix}:negative");

        assert!(!reply_already_recorded(&store, prefix, &wilco, true));
        assert!(!reply_already_recorded(&store, prefix, &negative, false));

        // Recording WILCO blocks duplicate WILCO, still allows one NEGATIVE transition.
        let id = store.alloc_envelope().expect("alloc");
        let env = Envelope::dispatch(
            id,
            ObjId(2),
            ObjId(1),
            DispatchKind::DetachmentReply,
            ref_key(&wilco),
            now.clone(),
        )
        .expect("wilco envelope")
        .in_state(EnvelopeState::Delivered, now.clone());
        store.put_envelope(env).expect("put wilco");

        assert!(reply_already_recorded(&store, prefix, &wilco, true));
        assert!(!reply_already_recorded(&store, prefix, &negative, false));

        // Recording NEGATIVE blocks both duplicate NEGATIVE and WILCO flip-flop.
        let id2 = store.alloc_envelope().expect("alloc");
        let env2 = Envelope::dispatch(
            id2,
            ObjId(2),
            ObjId(1),
            DispatchKind::DetachmentReply,
            ref_key(&negative),
            now.clone(),
        )
        .expect("negative envelope")
        .in_state(EnvelopeState::Delivered, now.clone());
        store.put_envelope(env2).expect("put negative");

        assert!(reply_already_recorded(&store, prefix, &negative, false));
        assert!(reply_already_recorded(&store, prefix, &wilco, true));

        // Bounded TTL: detachment replies become reapable after FIELD_ENVELOPE_TTL_SECS (1 IC day),
        // long before the 30-day ENVELOPE_TTL_SECS.
        let before_ttl = ClockStamp {
            clock: now.clock.clone(),
            at: now.at + crate::FIELD_ENVELOPE_TTL_SECS - 1,
        };
        let after_ttl = ClockStamp {
            clock: now.clock,
            at: now.at + crate::FIELD_ENVELOPE_TTL_SECS,
        };
        assert!(!crate::pass::is_reapable(
            store.envelope(id).expect("env"),
            &before_ttl
        ));
        assert!(crate::pass::is_reapable(
            store.envelope(id).expect("env"),
            &after_ttl
        ));
        const {
            assert!(crate::FIELD_ENVELOPE_TTL_SECS < crate::ENVELOPE_TTL_SECS);
        }
    }
}
