//! Mail's durable source and destination halves. The injected adapter only reads
//! transport; neither side mutates the other's state or owns a second router.
use crate::data::{self, INBOX, MailMsg, MailState, MailStatus, MailboxEntry, SENT};
use mush_core::{
    ClockStamp, CmdError, Ctx, Flags, LockType, ObjId, Role, World, eval_lock, is_ignoring, perm,
};
use mush_proto::OutputEvent;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

pub const MAX_PENDING_LETTERS: usize = 64;
pub const MAX_PENDING_COPIES: usize = 128;
pub const MAX_SETTLE_PER_TICK: usize = 16;
pub const TICK: &str = "mail.handoff";

#[derive(Clone, Debug)]
pub struct Intent {
    /// Original sender owns the reservation through the returning receipt.
    pub reservation_actor: ObjId,
    pub reservation_representation: Option<mush_core::RelayRepresentation>,
    pub representation: Option<mush_core::RelayRepresentation>,
    pub key: String,
    pub from: ObjId,
    pub to: ObjId,
    pub origin: Option<ObjId>,
    pub fallback: Option<ObjId>,
    pub bytes: u32,
    pub at: ClockStamp,
}
#[derive(Clone, Debug)]
pub struct Receipt {
    pub arrived: bool,
    pub origin: Option<ObjId>,
}
pub trait Transport: Send + Sync + std::fmt::Debug {
    fn capacity(
        &self,
        world: &World,
        actor: ObjId,
        representation: Option<mush_core::RelayRepresentation>,
        additional: usize,
    ) -> Result<(), mush_core::TransportRefusal>;
    fn representation(
        &self,
        _world: &World,
        _actor: ObjId,
    ) -> Result<Option<mush_core::RelayRepresentation>, mush_core::TransportRefusal> {
        Ok(None)
    }
    fn endpoints(&self, ctx: &Ctx<'_>, actors: &[ObjId]) -> Vec<Option<ObjId>>;
    fn receipt(&self, world: &World, key: &str) -> Option<Receipt>;
}
#[derive(Clone, Debug)]
pub struct Services(pub Arc<dyn Transport>);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Delivered,
    Refused,
    Full,
    Unavailable,
    Expired,
    Unconfirmed,
}
impl Outcome {
    pub fn text(&self) -> &'static str {
        match self {
            Self::Delivered => "delivered",
            Self::Refused => "refused at destination",
            Self::Full => "mailbox full at arrival",
            Self::Unavailable => "recipient unavailable",
            Self::Expired => "transit expired",
            Self::Unconfirmed => "delivery unconfirmed",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Copy {
    pub to: ObjId,
    pub fallback: Option<ObjId>,
    pub result: Option<Outcome>,
    pub return_origin: Option<ObjId>,
    pub return_at: Option<ClockStamp>,
    pub complete: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dispatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub representation: Option<mush_core::RelayRepresentation>,
    pub version: u32,
    pub id: u64,
    pub from: ObjId,
    pub origin: Option<ObjId>,
    pub at: ClockStamp,
    pub bytes: u32,
    pub urgent: bool,
    pub copies: Vec<Copy>,
}
impl Dispatch {
    pub fn key(&self, to: ObjId, returning: bool) -> String {
        format!(
            "mail:v1:{}:{}:{}",
            self.id,
            to.0,
            if returning { "return" } else { "out" }
        )
    }
    pub fn validate(&self, msg: &MailMsg) -> bool {
        self.version == 1
            && msg.delivery.is_some()
            && self.id == msg.id
            && self.from == msg.from
            && self.bytes as usize == msg.subject.len() + msg.body.len()
            && self.copies.len() == msg.to.len()
            && self.copies.len() <= data::MAX_MAIL_RECIPIENTS
            && self.copies.iter().zip(&msg.to).all(|(c, to)| {
                c.to == *to
                    && c.result.is_some() == c.return_at.is_some()
                    && c.return_at.as_ref().is_none_or(|at| at.at >= self.at.at)
                    && (!c.complete || c.result.is_some())
                    && c.complete
                        == msg
                            .delivery
                            .as_ref()
                            .is_some_and(|known| known.contains_key(to))
            })
    }
}

pub fn intents(world: &World) -> Option<Vec<Intent>> {
    let state = world.module_state::<MailState>()?;
    let mut intents = Vec::new();
    for dispatch in state.pending.values() {
        for copy in &dispatch.copies {
            if copy.complete {
                continue;
            }
            let returning = copy.result.is_some();
            intents.push(Intent {
                reservation_actor: dispatch.from,
                reservation_representation: dispatch.representation,
                representation: if returning {
                    None
                } else {
                    dispatch.representation
                },
                key: dispatch.key(copy.to, returning),
                from: if returning { copy.to } else { dispatch.from },
                to: if returning { dispatch.from } else { copy.to },
                origin: if returning {
                    copy.return_origin
                } else {
                    dispatch.origin
                },
                fallback: if returning {
                    dispatch.origin
                } else {
                    copy.fallback
                },
                bytes: if returning { 64 } else { dispatch.bytes },
                at: copy
                    .return_at
                    .clone()
                    .unwrap_or_else(|| dispatch.at.clone()),
            });
        }
    }
    Some(intents)
}

pub(crate) fn submit(
    ctx: &mut Ctx<'_>,
    to: &[ObjId],
    subject: &str,
    body: &str,
    reply_to: Option<u64>,
    urgent: bool,
) -> Result<(), CmdError> {
    data::validate_message_content(to, subject, body)
        .map_err(|e| CmdError::msg(format!("MAIL: Invalid message: {e}.")))?;
    let service = ctx
        .state::<Services>()
        .cloned()
        .ok_or_else(|| CmdError::msg("MAIL: Transport is unavailable."))?;
    let state = ctx
        .state::<MailState>()
        .ok_or_else(|| CmdError::msg("MAIL: State is unavailable."))?;
    if state.pending.len() >= MAX_PENDING_LETTERS
        || state
            .pending
            .values()
            .map(|d| d.copies.len())
            .sum::<usize>()
            + to.len()
            > MAX_PENDING_COPIES
    {
        return Err(CmdError::msg(
            "MAIL: Transport pending queue is full; try again later.",
        ));
    }
    let from = ctx.actor;
    let representation = service
        .0
        .representation(ctx.w, from)
        .map_err(|e| CmdError::msg(e.message()))?;
    service
        .0
        .capacity(ctx.w, from, representation, to.len())
        .map_err(|e| CmdError::msg(e.message()))?;
    if !perm::has_role(ctx.w, from, Role::Wizard) && state.entry_count(from) >= state.quota() {
        return Err(CmdError::msg(
            "MAIL: Your mailbox is full; free space for the Sent copy.",
        ));
    }
    let actors: Vec<_> = std::iter::once(from).chain(to.iter().copied()).collect();
    let endpoints = service.0.endpoints(ctx, &actors);
    if endpoints.len() != actors.len() {
        return Err(CmdError::msg("MAIL: Transport is unavailable."));
    }
    let origin = endpoints.first().copied().flatten();
    let at = ctx.w.world_ic_now();
    let now = ctx.w.now();
    let id = state.next_id();
    let msg = MailMsg {
        id,
        from,
        to: to.to_vec(),
        subject: subject.into(),
        body: body.into(),
        reply_to,
        sent_at: now,
        delivery: Some(BTreeMap::new()),
    };
    let dispatch = Dispatch {
        representation,
        version: 1,
        id,
        from,
        origin,
        at,
        bytes: (subject.len() + body.len()) as u32,
        urgent,
        copies: to
            .iter()
            .enumerate()
            .map(|(i, &to)| Copy {
                to,
                fallback: endpoints.get(i + 1).copied().flatten(),
                result: None,
                return_origin: None,
                return_at: None,
                complete: false,
            })
            .collect(),
    };
    let op = dispatch_op(&dispatch).map_err(CmdError::msg)?;
    let entry = MailboxEntry {
        owner: from,
        msg: id,
        folder: SENT.into(),
        status: MailStatus::Read,
        read_at: Some(now),
    };
    let Some(state) = ctx.state_mut::<MailState>() else {
        return Err(CmdError::msg("MAIL: State is unavailable."));
    };
    state
        .alloc_id()
        .ok_or_else(|| CmdError::msg("MAIL: Message id space is exhausted."))?;
    state.insert_msg(msg.clone());
    state.box_mut(from).entries.insert(id, entry.clone());
    state.pending.insert(id, dispatch);
    let next = state.next_id();
    for op in [
        data::msg_op(&msg),
        data::entry_op(&entry),
        data::next_id_op(next),
        op,
    ] {
        ctx.persist(op);
    }
    ctx.out.info(
        from,
        format!("MAIL: Message {id} queued for IC transit. Delivery is not yet confirmed."),
    );
    ctx.gate_on_durable_ack();
    Ok(())
}

pub(crate) fn dispatch_key(id: u64) -> smol_str::SmolStr {
    format!("dispatch:{id}").into()
}
fn dispatch_op(dispatch: &Dispatch) -> Result<mush_core::ModuleOp, String> {
    serde_json::to_value(dispatch)
        .map(|value| mush_core::ModuleOp {
            ns: data::NS.into(),
            op: mush_core::ModuleWrite::JsonSet {
                key: dispatch_key(dispatch.id),
                value,
            },
        })
        .map_err(|e| e.to_string())
}

fn refusal(world: &World, state: &MailState, msg: &MailMsg, to: ObjId) -> Option<Outcome> {
    let Some(recipient) = world.get(to).filter(|o| !o.flags.contains(Flags::GOING)) else {
        return Some(Outcome::Unavailable);
    };
    if is_ignoring(world, to, msg.from)
        || recipient
            .locks
            .get(&LockType::Mail)
            .is_some_and(|l| !eval_lock(world, l, msg.from, mush_core::lock::MAX_LOCK_DEPTH))
    {
        return Some(Outcome::Refused);
    }
    if !perm::has_role(world, to, Role::Wizard)
        && state.entry(to, msg.id).is_none()
        && state.entry_count(to) >= state.quota()
    {
        return Some(Outcome::Full);
    }
    None
}

pub(crate) fn settle(ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<mush_core::SceneId>) {
    let Some(service) = ctx.state::<Services>().cloned() else {
        return;
    };
    let Some(state) = ctx.state::<MailState>() else {
        return;
    };
    let mut ids: Vec<_> = state.pending.keys().copied().collect();
    let split = ids.partition_point(|id| *id < state.handoff_cursor);
    ids.rotate_left(split);
    ids.truncate(MAX_SETTLE_PER_TICK);
    for id in ids {
        let Some(state) = ctx.state::<MailState>() else {
            return;
        };
        let Some(mut dispatch) = state.pending.get(&id).cloned() else {
            continue;
        };
        let Some(mut msg) = state.message(id).cloned() else {
            continue;
        };
        let mut ops = Vec::new();
        let mut notices = Vec::new();
        let mut changed = false;
        for i in 0..dispatch.copies.len() {
            let Some(copy) = dispatch.copies.get(i).cloned() else {
                continue;
            };
            if copy.complete {
                continue;
            }
            let Some(receipt) = service
                .0
                .receipt(ctx.w, &dispatch.key(copy.to, copy.result.is_some()))
            else {
                continue;
            };
            let mut updated = copy.clone();
            if let Some(result) = copy.result {
                let learned = if receipt.arrived {
                    result
                } else {
                    Outcome::Unconfirmed
                };
                msg.delivery
                    .get_or_insert_with(BTreeMap::new)
                    .insert(copy.to, learned.clone());
                notices.push((
                    dispatch.from,
                    None,
                    format!("MAIL: Message {id} to {}: {}.", copy.to, learned.text()),
                ));
                updated.complete = true;
            } else {
                let Some(state) = ctx.state::<MailState>() else {
                    return;
                };
                let result = if receipt.arrived {
                    refusal(ctx.w, state, &msg, copy.to).unwrap_or(Outcome::Delivered)
                } else {
                    Outcome::Expired
                };
                if result == Outcome::Delivered {
                    let entry = MailboxEntry {
                        owner: copy.to,
                        msg: id,
                        folder: INBOX.into(),
                        status: if dispatch.urgent {
                            MailStatus::Urgent
                        } else {
                            MailStatus::Unread
                        },
                        read_at: None,
                    };
                    if let Some(state) = ctx.state_mut::<MailState>() {
                        state.box_mut(copy.to).entries.insert(id, entry.clone());
                        notices.push((
                            copy.to,
                            Some(state.unread_counts(copy.to).0),
                            String::new(),
                        ));
                    }
                    ops.push(data::entry_op(&entry));
                }
                updated.result = Some(result);
                updated.return_origin = receipt.origin.or(dispatch.origin);
                updated.return_at = Some(ctx.w.world_ic_now());
            }
            if let Some(slot) = dispatch.copies.get_mut(i) {
                *slot = updated;
            }
            changed = true;
        }
        let complete = dispatch.copies.iter().all(|c| c.complete);
        if changed {
            let Ok(op) = dispatch_op(&dispatch) else {
                return;
            };
            if let Some(state) = ctx.state_mut::<MailState>() {
                state.insert_msg(msg.clone());
                if complete {
                    state.pending.remove(&id);
                } else {
                    state.pending.insert(id, dispatch);
                }
            }
            ops.push(data::msg_op(&msg));
            ops.push(if complete {
                data::del_op(dispatch_key(id))
            } else {
                op
            });
            if complete
                && ctx
                    .state::<MailState>()
                    .is_some_and(|s| !s.msg_referenced(id))
            {
                if let Some(state) = ctx.state_mut::<MailState>() {
                    state.remove_msg(id);
                }
                ops.push(data::del_op(data::msg_key(id)));
            }
            for op in ops {
                ctx.persist(op);
            }
            for (actor, unread, text) in notices {
                if let Some(unread) = unread {
                    ctx.out.to_actor(
                        actor,
                        OutputEvent::MailNotice {
                            from: mush_core::obj_ref(ctx.w, msg.from),
                            subject: msg.subject.clone(),
                            id,
                            unread,
                        },
                    );
                } else {
                    ctx.out.info(actor, text);
                }
            }
        }
        if let Some(state) = ctx.state_mut::<MailState>() {
            state.handoff_cursor = id.saturating_add(1);
        }
        ctx.persist(mush_core::ModuleOp {
            ns: data::NS.into(),
            op: mush_core::ModuleWrite::JsonSet {
                key: "handoff_cursor".into(),
                value: id.saturating_add(1).into(),
            },
        });
    }
}
