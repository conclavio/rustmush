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
        Request::Send(messages) => (messages.clone(), DispatchKind::Detachment, None),
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
            if !matches!(
                reference.key.strip_prefix(&prefix),
                Some(":wilco" | ":negative")
            ) {
                return Err(refused("reply does not name its original field message"));
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
