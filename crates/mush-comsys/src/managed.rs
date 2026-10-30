//! Roster-owned IC channels share comsys history and Propagation delivery.
//! Membership changes commit with the source roster, never on a later sweep.
use crate::{Channel, ChannelFlags, ChannelId, ChannelPerms, ComsysState, Membership};
use mush_core::participant::{
    BoundParticipant, OwnerParticipant, OwnerPreparation, OwnerPreparationBinding,
    OwnerPrepareContext, ParticipantError,
};
use mush_core::{
    AuthoritativeKey, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, LockExpr,
    ModError, ObjId, OwnerId, Registrar, ReplayWrite,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub key: String,
    pub active: bool,
}
#[derive(Clone, Debug)]
pub struct Roster {
    pub key: String,
    pub members: BTreeSet<ObjId>,
}
#[derive(Clone, Debug)]
pub struct Request {
    pub close: Option<String>,
    pub open: Option<Roster>,
    pub join: Option<ObjId>,
}
pub struct ManagedChannels;
pub struct Prepared {
    binding: OwnerPreparationBinding,
}
impl Prepared {
    pub fn binding(&self) -> &OwnerPreparationBinding {
        &self.binding
    }
}
pub struct Patch {
    allocator: Option<(u64, u64)>,
    channels: Vec<(Option<Channel>, Channel)>,
    members: Vec<Membership>,
}
pub struct Bound<'a> {
    slot: &'a mut ComsysState,
    patch: Patch,
}
impl BoundParticipant for Bound<'_> {
    fn apply(self) {
        if let Some((_, after)) = self.patch.allocator {
            self.slot.next_chan_id = after;
        }
        for (_, channel) in self.patch.channels {
            self.slot.channels.insert(channel.id, channel);
        }
        for member in self.patch.members {
            self.slot.insert_member(member);
        }
    }
}
impl Patch {
    fn check(&self, state: &ComsysState) -> Result<(), ParticipantError> {
        if self
            .allocator
            .is_some_and(|(before, _)| state.next_chan_id != before)
        {
            return Err(refused());
        }
        for (before, after) in &self.channels {
            if state.channels.get(&after.id) != before.as_ref() {
                return Err(refused());
            }
            if before.is_none()
                && state.channels.values().any(|c| {
                    c.name.eq_ignore_ascii_case(&after.name)
                        || c.managed
                            .as_ref()
                            .zip(after.managed.as_ref())
                            .is_some_and(|(a, b)| a.key == b.key)
                })
            {
                return Err(refused());
            }
        }
        for m in &self.members {
            if state.members.contains_key(&(m.player, m.channel))
                || state.membership_by_alias(m.player, &m.alias).is_some()
            {
                return Err(refused());
            }
        }
        Ok(())
    }
}
fn member(
    state: &ComsysState,
    actor: ObjId,
    channel: ChannelId,
) -> Result<Membership, ParticipantError> {
    let alias = std::iter::once("dt".to_owned())
        .chain((2..=99).map(|n| format!("dt{n}")))
        .find(|a| state.membership_by_alias(actor, a).is_none())
        .ok_or_else(refused)?;
    Ok(Membership {
        player: actor,
        channel,
        alias: alias.into(),
        title: None,
        gagged: false,
        last_read: 0,
        ic: Some(crate::transit::Knowledge {
            joined: state.channels.get(&channel).map_or(0, |c| c.seq),
            ..Default::default()
        }),
    })
}
fn refused() -> ParticipantError {
    ParticipantError::refused("Roster channel changed or unavailable")
}
fn bad(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
fn descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new("participant").map_err(bad)?,
        DefinitionId::new("roster_channels").map_err(bad)?,
        1,
        json!({"format":1,"purpose":"roster_owned_ic_channels","max_members":6}),
    )
    .map_err(bad)
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    actions: BTreeSet<DefinitionRef>,
) -> Result<(), ModError> {
    if actions.is_empty() {
        return Ok(());
    }
    let descriptor = descriptor()?;
    let expected = mush_core::DefinitionRegistryBuilder::new()
        .register(OwnerId::new(crate::NS).map_err(bad)?, descriptor.clone())
        .map_err(bad)?;
    reg.definition_validator(
        descriptor.kind().clone(),
        move |ctx: &mush_core::DefinitionValidationContext<'_>| {
            if ctx.definition().reference() == &expected && !ctx.definition().is_current() {
                Ok(())
            } else {
                Err("unknown roster channel participant".into())
            }
        },
    )?;
    let definition = reg.definition(descriptor)?;
    reg.owner_participant(definition, actions, ManagedChannels)
}
fn write(key: &str, value: Value) -> Result<ReplayWrite, ParticipantError> {
    Ok(ReplayWrite::ModuleJsonSet {
        namespace: OwnerId::new(crate::NS).map_err(|_| ParticipantError::Encoding)?,
        key: AuthoritativeKey::parse_canonical(key).map_err(|_| ParticipantError::Encoding)?,
        value,
    })
}
fn encode<T: Serialize>(key: &str, value: &T) -> Result<ReplayWrite, ParticipantError> {
    write(
        key,
        serde_json::to_value(value).map_err(|_| ParticipantError::Encoding)?,
    )
}
/// The binding, not a display-name match, identifies a module-owned channel.
pub fn channel(w: &mush_core::World, key: &str) -> Option<ChannelId> {
    w.module_state::<ComsysState>()?
        .channels
        .values()
        .find(|c| c.managed.as_ref().is_some_and(|b| b.key == key && b.active))
        .map(|c| c.id)
}
impl OwnerParticipant for ManagedChannels {
    type State = ComsysState;
    type Request = Request;
    type Output = Prepared;
    type Patch = Patch;
    type Bound<'a> = Bound<'a>;
    fn prepare(
        &self,
        state: &ComsysState,
        ctx: &OwnerPrepareContext<'_>,
        request: Request,
    ) -> Result<OwnerPreparation<Prepared, Patch>, ParticipantError> {
        let mut patch = Patch {
            allocator: None,
            channels: Vec::new(),
            members: Vec::new(),
        };
        let mut writes = Vec::new();
        if let Some(key) = request.close
            && let Some(c) = state
                .channels
                .values()
                .find(|c| c.managed.as_ref().is_some_and(|b| b.key == key))
        {
            let mut after = c.clone();
            if let Some(b) = &mut after.managed {
                b.active = false;
            }
            writes.push(encode(crate::chan_key(c.id).as_str(), &after)?);
            patch.channels.push((Some(c.clone()), after));
        }
        if let Some(roster) = request.open {
            if roster.key.is_empty()
                || roster.key.len() > 96
                || roster.members.is_empty()
                || roster.members.len() > 6
                || roster.members.contains(&ObjId(0))
            {
                return Err(refused());
            }
            if let Some(c) = state
                .channels
                .values()
                .find(|c| c.managed.as_ref().is_some_and(|b| b.key == roster.key))
            {
                if !c.active()
                    || !state
                        .indexed_members(c.id)
                        .all(|m| roster.members.contains(&m.player))
                {
                    return Err(refused());
                }
                patch.channels.push((Some(c.clone()), c.clone()));
                if let Some(actor) = request.join {
                    if !roster.members.contains(&actor) {
                        return Err(refused());
                    }
                    if !state.members.contains_key(&(actor, c.id)) {
                        patch.members.push(member(state, actor, c.id)?);
                    }
                }
            } else {
                // Names are presentation only. Never adopt a pre-created channel.
                let mut next = state.next_chan_id;
                let mut selection = None;
                for _ in 0..16 {
                    let id = ChannelId(next);
                    next = next.checked_add(1).ok_or_else(refused)?;
                    let name = format!("DT-{}", id.0);
                    if id.0 != 0
                        && crate::data::valid_channel_name(&name)
                        && state.channel_by_name(&name).is_none()
                    {
                        selection = Some((id, name));
                        break;
                    }
                }
                let (id, name) = selection.ok_or_else(refused)?;
                let mut c = Channel::new(id, &name);
                c.managed = Some(Binding {
                    key: roster.key,
                    active: true,
                });
                c.description = "Detachment IC communications".into();
                c.flags = ChannelFlags {
                    ic: true,
                    closed: true,
                    logged: true,
                    ..Default::default()
                };
                c.perms = ChannelPerms {
                    join: LockExpr::Const(false),
                    see: LockExpr::Const(false),
                    speak: LockExpr::Const(true),
                    admin: LockExpr::Const(false),
                };
                writes.push(encode(crate::chan_key(id).as_str(), &c)?);
                patch.channels.push((None, c));
                for actor in roster.members {
                    patch.members.push(member(state, actor, id)?);
                }
                patch.allocator = Some((state.next_chan_id, next));
                writes.push(write(crate::META_KEY, json!({"next_chan_id":next}))?);
            }
        }
        for m in &patch.members {
            writes.push(encode(crate::member_key(m.player, m.channel).as_str(), m)?);
        }
        Ok(OwnerPreparation::new(
            Prepared {
                binding: ctx.preparation_binding(),
            },
            patch,
            writes,
        ))
    }
    fn recheck(
        &self,
        state: &ComsysState,
        _: &OwnerPrepareContext<'_>,
        patch: &Patch,
    ) -> Result<(), ParticipantError> {
        patch.check(state)
    }
    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut ComsysState,
        patch: Patch,
    ) -> Result<Bound<'a>, ParticipantError> {
        patch.check(state)?;
        Ok(Bound { slot: state, patch })
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
        if key.as_str() == crate::META_KEY {
            return value.get("next_chan_id").and_then(Value::as_u64).is_some();
        }
        if let Ok(c) = serde_json::from_value::<Channel>(value.clone()) {
            return crate::chan_key(c.id).as_str() == key.as_str()
                && c.managed.is_some()
                && c.flags.ic
                && c.flags.closed;
        }
        serde_json::from_value::<Membership>(value.clone())
            .ok()
            .is_some_and(|m| {
                crate::member_key(m.player, m.channel).as_str() == key.as_str() && m.ic.is_some()
            })
    }
}
