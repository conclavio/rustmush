//! IC content and recipient knowledge. Propagation remains the only router.
use crate::data::{self, ChanMsg, ChannelId, ComsysState, Membership};
use mush_core::{
    Audience, ClockStamp, Ctx, Deferred, Flags, ModError, ObjId, Outbox, World, is_ignoring,
};
use mush_proto::{ObjRef, OutputEvent, RichText};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
};

pub const MAX_MEMBERS: usize = 32;
pub const MAX_PENDING: usize = 64;
pub const MAX_COPIES: usize = 128;
pub const MAX_SETTLE: usize = 16;
pub const MAX_TEXT_BYTES: usize = 16_384;
pub const TICK: &str = "comsys.handoff";

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
    /// Physical address only. An unresolved address must not become local.
    fn endpoint(&self, world: &World, actor: ObjId) -> Option<ObjId>;
    /// A committed terminal receipt: true arrived, false expired.
    fn receipt(&self, world: &World, key: &str) -> Option<bool>;
}
#[derive(Clone, Debug)]
pub struct Services(pub Arc<dyn Transport>);
#[derive(Clone, Debug)]
pub struct Intent {
    pub representation: Option<mush_core::RelayRepresentation>,
    pub key: String,
    pub from: ObjId,
    pub to: ObjId,
    pub origin: Option<ObjId>,
    pub at: ClockStamp,
    pub bytes: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Knowledge {
    /// Opaque cache identity for this membership; reveals no source sequence.
    pub epoch: String,
    /// Membership boundary in the private source sequence. A rejoin cannot
    /// receive any earlier intent. It grants no historical traffic.
    pub joined: u64,
    /// Dense local arrival sequence, never the source's global sequence.
    pub sequence: u64,
    pub received: VecDeque<Received>,
}
impl Default for Knowledge {
    fn default() -> Self {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).expect("channel membership epoch requires OS entropy");
        Self {
            epoch: bytes.iter().map(|b| format!("{b:02x}")).collect(),
            joined: 0,
            sequence: 0,
            received: VecDeque::new(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Received {
    pub seq: u64,
    pub message: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub representation: Option<mush_core::RelayRepresentation>,
    pub version: u32,
    pub channel: ChannelId,
    pub msg: ChanMsg,
    pub at: ClockStamp,
    pub origin: Option<ObjId>,
    pub pending: BTreeSet<ObjId>,
}
impl Message {
    pub fn key(&self, to: ObjId) -> String {
        format!("channel:v1:{}:{}:{}", self.channel.0, self.msg.seq, to.0)
    }
    pub fn bytes(&self) -> usize {
        self.msg.text.len()
            + self.msg.actor_name.len()
            + self.msg.title.as_ref().map_or(0, String::len)
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Progress {
    pub cursor: Option<(ChannelId, u64)>,
    pub delivered: u64,
    pub refused: u64,
    pub expired: u64,
    pub visited: usize,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub messages: BTreeMap<(ChannelId, u64), Message>,
    pub pending: BTreeSet<(ChannelId, u64)>,
    pub references: BTreeMap<(ChannelId, u64), usize>,
    pub progress: Progress,
    pub relay: RelayLedger,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelayLedger {
    pub floor: u64,
    pub accepted: BTreeMap<u64, mush_core::CanonicalHash>,
}
pub(crate) fn relay_identity(
    w: &World,
    id: u64,
    actor: ObjId,
    channel: ChannelId,
    kind: mush_proto::NarrativeKind,
    text: &str,
) -> Result<(mush_core::CanonicalHash, bool), mush_core::RelayRefusal> {
    use mush_core::{CanonicalHash, RelayRefusal};
    let fingerprint = CanonicalHash::for_canonical_value(
        "comsys.relay",
        1,
        &serde_json::json!([actor, channel, kind, text]),
        66_048,
    )
    .map_err(|_| RelayRefusal::RetryIdentity)?;
    let st = w
        .module_state::<ComsysState>()
        .ok_or(RelayRefusal::NoSpeaker)?;
    if id == 0 || id <= st.ic.relay.floor {
        return Err(RelayRefusal::RetryIdentity);
    }
    match st.ic.relay.accepted.get(&id) {
        Some(old) if *old != fingerprint => Err(RelayRefusal::RetryIdentity),
        old => Ok((fingerprint, old.is_some())),
    }
}
pub(crate) fn accept_relay(
    w: &mut World,
    out: &mut Outbox,
    id: u64,
    fingerprint: mush_core::CanonicalHash,
) {
    let Some(st) = w.module_state_mut::<ComsysState>() else {
        return;
    };
    st.ic.relay.accepted.insert(id, fingerprint);
    while st.ic.relay.accepted.len() > 256 {
        if let Some((id, _)) = st.ic.relay.accepted.pop_first() {
            st.ic.relay.floor = st.ic.relay.floor.max(id);
        }
    }
    data::persist_json(out, "ic:relay".into(), &st.ic.relay);
}
fn key(channel: ChannelId, id: u64) -> SmolStr {
    format!("ic:msg:{}:{id}", channel.0).into()
}
fn persist(out: &mut Outbox, m: &Message) {
    data::persist_json(out, key(m.channel, m.msg.seq), m);
}
fn progress(out: &mut Outbox, p: &Progress) {
    data::persist_json(out, "ic:progress".into(), p);
}
impl State {
    fn collect(&mut self, out: &mut Outbox, id: (ChannelId, u64)) {
        if !self.pending.contains(&id)
            && !self.references.contains_key(&id)
            && self.messages.remove(&id).is_some()
        {
            data::persist_del(out, key(id.0, id.1));
        }
    }
    pub(crate) fn release(&mut self, out: &mut Outbox, id: (ChannelId, u64)) {
        if let Some(n) = self.references.get_mut(&id) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                self.references.remove(&id);
            }
        }
        self.collect(out, id);
    }
}
pub fn intents(w: &World) -> Option<Vec<Intent>> {
    let st = w.module_state::<ComsysState>()?;
    let mut result = Vec::new();
    for id in &st.ic.pending {
        let m = st.ic.messages.get(id)?;
        if !st
            .channels
            .get(&m.channel)
            .is_some_and(crate::Channel::active)
        {
            continue;
        }
        for &to in &m.pending {
            result.push(Intent {
                representation: m.representation,
                key: m.key(to),
                from: m.msg.actor,
                to,
                origin: m.origin,
                at: m.at.clone(),
                bytes: m.bytes() as u32,
            });
        }
    }
    Some(result)
}

/// Source admission and durable intent, called by every IC speaking path.
pub(crate) fn submit(
    w: &mut World,
    out: &mut Outbox,
    channel: ChannelId,
    mut msg: ChanMsg,
) -> Result<(), &'static str> {
    if msg.text.len() > MAX_TEXT_BYTES {
        return Err("IC channel line is too long.");
    }
    let service = w
        .module_state::<Services>()
        .cloned()
        .ok_or("IC transport is unavailable.")?;
    let st = w
        .module_state::<ComsysState>()
        .ok_or("Comsys is unavailable.")?;
    let c = st.channels.get(&channel).ok_or("No such channel.")?;
    let members: Vec<_> = st.indexed_members(channel).take(MAX_MEMBERS + 1).collect();
    if members.len() > MAX_MEMBERS {
        return Err("IC channel has too many members for bounded transport.");
    }
    let recipients: BTreeSet<_> = members
        .into_iter()
        .filter(|m| {
            !m.gagged
                && c.hears(w, m.player)
                && !is_ignoring(w, m.player, msg.actor)
                && w.get(m.player)
                    .is_some_and(|o| !o.flags.contains(Flags::GOING))
        })
        .map(|m| m.player)
        .collect();
    let copies: usize = st
        .ic
        .pending
        .iter()
        .filter_map(|id| st.ic.messages.get(id))
        .map(|m| m.pending.len())
        .sum();
    if st.ic.pending.len() >= MAX_PENDING || copies + recipients.len() > MAX_COPIES {
        return Err("IC transport pending queue is full; try again later.");
    }
    let representation = service
        .0
        .representation(w, msg.actor)
        .map_err(|e| e.message())?;
    service
        .0
        .capacity(w, msg.actor, representation, recipients.len())
        .map_err(|e| e.message())?;
    msg.seq = c.seq.checked_add(1).ok_or("Channel sequence exhausted.")?;
    let m = Message {
        representation,
        version: 1,
        channel,
        origin: service.0.endpoint(w, msg.actor),
        at: w.world_ic_now(),
        pending: recipients,
        msg,
    };
    if m.bytes() > 66_048 || m.bytes() == 0 {
        return Err("IC channel content is too long.");
    }
    let actor = m.msg.actor;
    let st = w
        .module_state_mut::<ComsysState>()
        .ok_or("Comsys is unavailable.")?;
    let c = st.channels.get_mut(&channel).ok_or("No such channel.")?;
    c.seq = m.msg.seq;
    data::persist_channel(out, c);
    if !m.pending.is_empty() {
        let id = (channel, m.msg.seq);
        persist(out, &m);
        st.ic.pending.insert(id);
        st.ic.messages.insert(id, m);
    }
    out.defer(Deferred::DurableGate {
        actor,
        strict: false,
    });
    out.info(actor, "Channel message queued for IC transit.");
    Ok(())
}

/// Arrival application and source withdrawal share one comsys-owned batch.
pub(crate) fn settle(ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<mush_core::SceneId>) {
    let now_ms = ctx.w.now().saturating_mul(1000);
    let Some(service) = ctx.state::<Services>().cloned() else {
        return;
    };
    let Some(st) = ctx.state::<ComsysState>() else {
        return;
    };
    let cursor = st.ic.progress.cursor;
    let ids: Vec<_> = st
        .ic
        .pending
        .iter()
        .filter(|id| cursor.is_none_or(|c| **id > c))
        .chain(
            st.ic
                .pending
                .iter()
                .filter(|id| cursor.is_some_and(|c| **id <= c)),
        )
        .take(MAX_SETTLE)
        .copied()
        .collect();
    if ids.is_empty() {
        return;
    }
    let mut visited = 0;
    for id in ids {
        let Some(message) = ctx
            .state::<ComsysState>()
            .and_then(|s| s.ic.messages.get(&id))
            .cloned()
        else {
            continue;
        };
        let mut changed = false;
        for &to in &message.pending {
            visited += 1;
            let active = ctx
                .state::<ComsysState>()
                .and_then(|s| s.channels.get(&id.0))
                .is_some_and(crate::Channel::active);
            let alive = ctx
                .w
                .get(to)
                .is_some_and(|o| !o.flags.contains(Flags::GOING));
            // Withdrawal is cancellation, not a delivery claim. A destroyed
            // recipient cannot consume content; no result is exposed remotely.
            let Some(arrived) = (if alive && active {
                service.0.receipt(ctx.w, &message.key(to))
            } else {
                Some(false)
            }) else {
                continue;
            };
            changed = true;
            let authorized = arrived
                && alive
                && ctx.state::<ComsysState>().is_some_and(|s| {
                    s.channels
                        .get(&id.0)
                        .is_some_and(|c| c.flags.ic && c.hears(ctx.w, to))
                        && s.members.get(&(to, id.0)).is_some_and(|m| {
                            !m.gagged && m.ic.as_ref().is_some_and(|k| message.msg.seq > k.joined)
                        })
                })
                && !is_ignoring(ctx.w, to, message.msg.actor);
            let live = ctx.w.has_live_conns(to);
            let recipient_name = mush_core::output::obj_ref(ctx.w, to).name;
            let Some(st) = ctx.w.module_state_mut::<ComsysState>() else {
                return;
            };
            if authorized {
                if let (Some(c), Some(m)) =
                    (st.channels.get(&id.0), st.members.get_mut(&(to, id.0)))
                    && let Some(k) = m.ic.as_mut()
                {
                    k.sequence = k.sequence.saturating_add(1);
                    let seq = k.sequence;
                    let epoch = k.epoch.clone();
                    if c.flags.logged {
                        k.received.push_back(Received {
                            seq,
                            message: message.msg.seq,
                        });
                        *st.ic.references.entry(id).or_default() += 1;
                        while k.received.len() > c.history.cap as usize {
                            if let Some(old) = k.received.pop_front() {
                                st.ic.release(ctx.out, (id.0, old.message));
                            }
                        }
                    }
                    if live {
                        m.last_read = seq;
                    }
                    data::persist_membership(ctx.out, m);
                    let msg = &message.msg;
                    if live {
                        ctx.out.push(
                            Audience::Actor(to),
                            OutputEvent::Channel {
                                channel: c.name.clone(),
                                epoch: Some(epoch),
                                seq,
                                actor: ObjRef {
                                    id: msg.actor.to_string(),
                                    name: msg.actor_name.clone(),
                                },
                                title: msg.title.clone(),
                                kind: msg.kind,
                                text: RichText::plain(&msg.text),
                                ts: msg.ts,
                            },
                        );
                        if to != msg.actor
                            && msg.text.split(|c: char| !c.is_alphanumeric()).any(|word| {
                                !word.is_empty() && word.eq_ignore_ascii_case(&recipient_name)
                            })
                        {
                            ctx.out.notice_info_to_actor(
                                to,
                                now_ms,
                                "mention",
                                format!("{} on {}: {}", msg.actor_name, m.alias, msg.text),
                                Some(("Reply", &format!("{} ", m.alias))),
                            );
                        }
                    }
                    st.ic.progress.delivered = st.ic.progress.delivered.saturating_add(1);
                }
            } else if arrived {
                st.ic.progress.refused = st.ic.progress.refused.saturating_add(1);
            } else {
                st.ic.progress.expired = st.ic.progress.expired.saturating_add(1);
            }
            if let Some(m) = st.ic.messages.get_mut(&id) {
                m.pending.remove(&to);
            }
        }
        let Some(st) = ctx.w.module_state_mut::<ComsysState>() else {
            return;
        };
        if let Some(m) = st.ic.messages.get(&id) {
            if m.pending.is_empty() {
                st.ic.pending.remove(&id);
            }
            if changed {
                persist(ctx.out, m);
            }
        }
        st.ic.collect(ctx.out, id);
        st.ic.progress.cursor = Some(id);
    }
    if let Some(st) = ctx.w.module_state_mut::<ComsysState>() {
        st.ic.progress.visited = visited;
        progress(ctx.out, &st.ic.progress);
    }
}

pub(crate) fn sequence(c: &crate::Channel, m: &Membership) -> u64 {
    if c.flags.ic {
        m.ic.as_ref().map_or(0, |k| k.sequence)
    } else {
        c.seq
    }
}
pub(crate) fn history<'a>(st: &'a ComsysState, m: &'a Membership) -> Vec<ChanMsg> {
    if st.channels.get(&m.channel).is_some_and(|c| c.flags.ic) {
        m.ic.as_ref().map_or_else(Vec::new, |k| {
            k.received
                .iter()
                .filter_map(|r| {
                    let mut msg = st.ic.messages.get(&(m.channel, r.message))?.msg.clone();
                    msg.seq = r.seq;
                    Some(msg)
                })
                .collect()
        })
    } else {
        st.rings
            .get(&m.channel)
            .map_or_else(Vec::new, |r| r.iter().cloned().collect())
    }
}

/// Rebuild bounded pending work and reference counts; migrate only legacy IC
/// history that was already readable. Corrupt new handoffs fail closed.
pub(crate) fn load(
    st: &mut ComsysState,
    out: &mut Outbox,
    docs: &[(SmolStr, serde_json::Value)],
    at: ClockStamp,
) -> Result<(), ModError> {
    let bad = || ModError::msg("comsys: invalid IC content/receipt record");
    for (name, value) in docs {
        if name == "ic:progress" {
            st.ic.progress = serde_json::from_value(value.clone()).map_err(|_| bad())?;
        } else if name == "ic:relay" {
            st.ic.relay = serde_json::from_value(value.clone()).map_err(|_| bad())?;
            if st.ic.relay.accepted.len() > 256
                || st
                    .ic
                    .relay
                    .accepted
                    .keys()
                    .any(|id| *id <= st.ic.relay.floor)
            {
                return Err(bad());
            }
        } else if name.starts_with("ic:msg:") {
            let m: Message = serde_json::from_value(value.clone()).map_err(|_| bad())?;
            if *name != key(m.channel, m.msg.seq)
                || m.version != 1
                || m.msg.text.len() > MAX_TEXT_BYTES
                || m.bytes() > 66_048
                || m.bytes() == 0
                || m.pending.len() > MAX_MEMBERS
                || !st
                    .channels
                    .get(&m.channel)
                    .is_some_and(|c| c.flags.ic && m.msg.seq <= c.seq)
            {
                return Err(bad());
            }
            let id = (m.channel, m.msg.seq);
            if !m.pending.is_empty() {
                st.ic.pending.insert(id);
            }
            st.ic.messages.insert(id, m);
        }
    }
    if st.ic.pending.len() > MAX_PENDING
        || st
            .ic
            .pending
            .iter()
            .filter_map(|id| st.ic.messages.get(id))
            .map(|m| m.pending.len())
            .sum::<usize>()
            > MAX_COPIES
    {
        return Err(bad());
    }
    for m in st.members.values_mut() {
        let Some(c) = st.channels.get(&m.channel) else {
            continue;
        };
        if !c.flags.ic {
            continue;
        }
        if m.ic.is_none() {
            if st
                .ic
                .pending
                .iter()
                .any(|(channel, _)| *channel == m.channel)
            {
                return Err(bad());
            }
            let mut knowledge = Knowledge {
                joined: c.seq,
                ..Knowledge::default()
            };
            let mut read = 0;
            for msg in st.rings.get(&m.channel).into_iter().flatten() {
                knowledge.sequence += 1;
                if msg.seq <= m.last_read {
                    read = knowledge.sequence;
                }
                knowledge.received.push_back(Received {
                    seq: knowledge.sequence,
                    message: msg.seq,
                });
                let id = (m.channel, msg.seq);
                if let std::collections::btree_map::Entry::Vacant(e) = st.ic.messages.entry(id) {
                    let message = Message {
                        representation: None,
                        version: 1,
                        channel: m.channel,
                        msg: msg.clone(),
                        at: at.clone(),
                        origin: None,
                        pending: BTreeSet::new(),
                    };
                    persist(out, &message);
                    e.insert(message);
                }
            }
            m.last_read = read;
            m.ic = Some(knowledge);
            data::persist_membership(out, m);
        }
        let Some(k) = &m.ic else { return Err(bad()) };
        if k.epoch.len() != 32
            || !k
                .epoch
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || k.joined > c.seq
            || m.last_read > k.sequence
            || k.received.len() > data::MAX_HISTORY_CAP as usize
        {
            return Err(bad());
        }
        let mut prev = 0;
        let mut seen = BTreeSet::new();
        for r in &k.received {
            let id = (m.channel, r.message);
            if r.seq <= prev
                || r.seq > k.sequence
                || !seen.insert(r.message)
                || !st.ic.messages.contains_key(&id)
                || st
                    .ic
                    .messages
                    .get(&id)
                    .is_some_and(|msg| msg.pending.contains(&m.player))
            {
                return Err(bad());
            }
            prev = r.seq;
            *st.ic.references.entry(id).or_default() += 1;
        }
    }
    let ids: Vec<_> = st.ic.messages.keys().copied().collect();
    for id in ids {
        st.ic.collect(out, id);
    }
    Ok(())
}
