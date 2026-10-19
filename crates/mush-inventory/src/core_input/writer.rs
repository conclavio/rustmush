use super::records::{Census, Head, Terminal, TerminalKind, held, stamp};
use super::registration::{OPERATIONS, ReadRequest, Shared, envelope, key};
use super::*;
use crate::{EndpointRef, InventoryStore, LotId};
use mush_core::participant::{OwnerPreparationBinding, OwnerPrepareContext};
use mush_core::{ClockStamp, ObjId, PerceivedRef, PrepareCtx, PrincipalId};
use std::sync::Arc;

/// Input selected by the real Industry participant after its own policy check.
/// No variant accepts a physical post-image or an assertion of certification.
pub enum CoreInputRequest {
    Reserve {
        identity: CoreInputIdentity,
        lot: LotId,
        endpoint: EndpointRef,
    },
    Release {
        identity: CoreInputIdentity,
    },
    Consume {
        identity: CoreInputIdentity,
        endpoint: EndpointRef,
        expires: ClockStamp,
    },
}
impl CoreInputRequest {
    fn identity(&self) -> CoreInputIdentity {
        match self {
            Self::Reserve { identity, .. }
            | Self::Release { identity }
            | Self::Consume { identity, .. } => *identity,
        }
    }
    fn endpoint(&self) -> Option<EndpointRef> {
        match self {
            Self::Reserve { endpoint, .. } | Self::Consume { endpoint, .. } => Some(*endpoint),
            Self::Release { .. } => None,
        }
    }
    fn action<'a>(&self, binding: &'a CoreInputBinding) -> &'a DefinitionRef {
        match self {
            Self::Reserve { .. } => &binding.accept,
            Self::Release { .. } => &binding.cancel,
            Self::Consume { .. } => &binding.complete,
        }
    }
}
/// Constructible only from a real Core-minted Industry owner preparation.
/// The Industry image must retain this witness and call `recheck` at its actual
/// stage/take gates. It supplies authority; this object has no decode path.
pub struct CoreInputWitness {
    preparation: OwnerPreparationBinding,
    shared: Arc<Shared>,
    request: CoreInputRequest,
    actor: ObjId,
    action: DefinitionRef,
    commit: CommitToken,
    at: ClockStamp,
    utc: i64,
}
fn context(
    ctx: &OwnerPrepareContext<'_>,
    request: &CoreInputRequest,
    shared: &Shared,
) -> Result<(), Error> {
    let inv = ctx.invocation();
    if ctx.action() != request.action(&shared.binding)
        || inv.principal() != &(PrincipalId::Object { id: inv.actor() })
        || inv.subject() != &PerceivedRef::SelfActor
        || !inv.targets().is_empty()
        || inv.idempotency().scope().owner.as_str() != "industry"
    {
        return Err(Error::Unavailable);
    }
    let state = ctx
        .world()
        .module_state::<InventoryStore>()
        .ok_or(Error::Unavailable)?;
    if ctx
        .world()
        .module_state_owner(std::any::TypeId::of::<InventoryStore>())
        .is_none_or(|owner| owner.as_str() != "inventory")
        || !state.is_complete()
    {
        return Err(Error::Unavailable);
    }
    if let Some(endpoint) = request.endpoint() {
        if state.is_quarantined(endpoint)
            || !mush_economy::endpoint_state(ctx.world(), endpoint.0).is_live()
        {
            return Err(Error::Unavailable);
        }
    }
    Ok(())
}
impl RegisteredCoreInputs {
    pub fn capture(
        &self,
        ctx: &OwnerPrepareContext<'_>,
        request: CoreInputRequest,
    ) -> Result<Arc<CoreInputWitness>, Error> {
        context(ctx, &request, &self.handle.shared)?;
        let at = ctx.world().world_ic_now();
        stamp(&at)?;
        let value = Arc::new(CoreInputWitness {
            preparation: ctx.preparation_binding(),
            shared: Arc::clone(&self.handle.shared),
            request,
            actor: ctx.invocation().actor(),
            action: ctx.action().clone(),
            commit: ctx.commit_token(),
            at,
            utc: ctx.world().now(),
        });
        self.recheck(ctx, &value)?;
        Ok(value)
    }
    pub fn recheck(
        &self,
        ctx: &OwnerPrepareContext<'_>,
        witness: &CoreInputWitness,
    ) -> Result<(), Error> {
        if !ctx.matches_preparation(&witness.preparation)
            || !Arc::ptr_eq(&self.handle.shared, &witness.shared)
            || ctx.action() != &witness.action
            || ctx.commit_token() != witness.commit
            || ctx.invocation().actor() != witness.actor
        {
            return Err(Error::Unavailable);
        }
        context(ctx, &witness.request, &self.handle.shared)?;
        // Release is cleanup; it does not depend on the admission clock or on
        // the current state of the old endpoint. Fresh holds/consumption do.
        if !matches!(witness.request, CoreInputRequest::Release { .. }) {
            let now = ctx.world().world_ic_now();
            let expires = match &witness.request {
                CoreInputRequest::Consume { expires, .. } => expires.clone(),
                _ => crate::reservation_deadline(&witness.at).map_err(bad)?,
            };
            if now.clock != witness.at.clock
                || now.at < witness.at.at
                || now.checked_cmp(&expires).map_err(bad)? != std::cmp::Ordering::Less
            {
                return Err(Error::Unavailable);
            }
        }
        Ok(())
    }
    /// Publish only through the exact same preparation's real checked owner
    /// read. This never accepts a raw caller action or a decoded witness.
    pub fn publish(
        &self,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
        w: &CoreInputWitness,
    ) -> Result<CoreInputOperation, Error> {
        if !Arc::ptr_eq(&self.handle.shared, &w.shared)
            || ctx.commit_token() != w.commit
            || txn.owner().as_str() != "industry"
        {
            return Err(Error::Unavailable);
        }
        ctx.require_owner_preparation_binding(txn, &w.preparation)
            .map_err(bad)?;
        let observed = ctx
            .read_owner_preimage(
                txn,
                &self.reader,
                ReadRequest {
                    endpoint: w.request.endpoint(),
                },
            )
            .map_err(bad)?;
        if observed.actor != w.actor || observed.stamp != w.at || observed.utc != w.utc {
            return Err(Error::Unavailable);
        }
        let f = &self.handle.shared;
        let identity = w.request.identity();
        match &w.request {
            CoreInputRequest::Reserve { lot, endpoint, .. } => {
                if self.handle.operation(txn, identity.order())?.is_some() {
                    return Err(Error::Reserved);
                }
                let current = txn
                    .get(&f.lots, lot)
                    .map_err(bad)?
                    .cloned()
                    .ok_or(Error::Unavailable)?;
                if current.id() != *lot || current.endpoint() != *endpoint {
                    return Err(Error::Integrity);
                }
                let previous = self.handle.current(txn, *lot)?;
                let (attempt, prior) = if let Some(old) = &previous {
                    if old.state()? != CoreInputState::Released {
                        return Err(Error::Reserved);
                    }
                    (
                        old.attempt.checked_add(1).ok_or(Error::Capacity)?,
                        Some(old.identity.order()),
                    )
                } else {
                    (1, None)
                };
                let census = txn.get(&f.census, &()).map_err(bad)?.cloned();
                let before = census.clone().map_or_else(Census::empty, Ok)?;
                if before.count >= MAX_OPERATIONS {
                    return Err(Error::Capacity);
                }
                let (scalar, book) = held(identity, *lot, &w.at)?;
                let op = CoreInputOperation {
                    format: 1,
                    identity,
                    sequence: before.count + 1,
                    attempt,
                    previous: prior,
                    lot: current,
                    binding: f.binding.clone(),
                    accepted: w.commit,
                    actor: w.actor,
                    at: w.at.clone(),
                    utc: w.utc,
                    scalar,
                    book,
                    terminal: None,
                };
                op.validate()?;
                require_terminal_room(&op)?;
                let after = before.push(&op)?;
                let head = Head::from_operation(&op);
                txn.insert(&f.operations, &identity.order(), op.clone())
                    .map_err(bad)?;
                if previous.is_some() {
                    txn.replace(&f.heads, lot, head).map_err(bad)?;
                } else {
                    txn.insert(&f.heads, lot, head).map_err(bad)?;
                }
                if census.is_some() {
                    txn.replace(&f.census, &(), after).map_err(bad)?;
                } else {
                    txn.insert(&f.census, &(), after).map_err(bad)?;
                }
                Ok(op)
            }
            CoreInputRequest::Release { .. } | CoreInputRequest::Consume { .. } => {
                let old = self
                    .handle
                    .operation(txn, identity.order())?
                    .ok_or(Error::Unavailable)?;
                if old.identity != identity
                    || self.handle.current(txn, old.lot.id())?.as_ref() != Some(&old)
                {
                    return Err(Error::Integrity);
                }
                let actual = txn.get(&f.lots, &old.lot.id()).map_err(bad)?.cloned();
                if actual.as_ref() != Some(&old.lot) {
                    return Err(Error::Integrity);
                }
                let kind = if let CoreInputRequest::Consume {
                    endpoint, expires, ..
                } = &w.request
                {
                    let [record] = old.book.records() else {
                        return Err(Error::Integrity);
                    };
                    if *endpoint != old.lot.endpoint() || expires != record.expires() {
                        return Err(Error::Integrity);
                    }
                    TerminalKind::Consume
                } else {
                    TerminalKind::Release
                };
                let terminal = Terminal {
                    kind,
                    binding: f.binding.clone(),
                    action: w.action.clone(),
                    commit: w.commit,
                    actor: w.actor,
                    at: w.at.clone(),
                    utc: w.utc,
                };
                let op = old.finish(terminal)?;
                op.validate()?;
                envelope(
                    OPERATIONS,
                    &key(identity.order()),
                    encode(&op).map_err(bad)?,
                )?;
                let census = if kind == TerminalKind::Consume {
                    let row = txn
                        .get(&f.lot_census, &old.lot.endpoint())
                        .map_err(bad)?
                        .copied()
                        .ok_or(Error::Integrity)?;
                    if row.get() == 0 {
                        return Err(Error::Integrity);
                    }
                    Some(row.checked_move(-1).map_err(bad)?)
                } else {
                    None
                };
                txn.replace(&f.operations, &identity.order(), op.clone())
                    .map_err(bad)?;
                if let Some(census) = census {
                    txn.remove(&f.lots, &old.lot.id()).map_err(bad)?;
                    txn.replace(&f.lot_census, &old.lot.endpoint(), census)
                        .map_err(bad)?;
                }
                Ok(op)
            }
        }
    }
}

/// Use exact reconstructed K terminal bodies, then widen only numeric terminal
/// fields for complete-envelope sizing. This is an internal size probe, never
/// an authority constructor or persisted record. Action locators are fixed and
/// their full digests are fixed-width, so a later configured action has this size.
pub(super) fn require_terminal_room(op: &CoreInputOperation) -> Result<[usize; 3], Error> {
    let admitted = envelope(
        OPERATIONS,
        &key(op.identity.order()),
        encode(op).map_err(bad)?,
    )?;
    let mut commit = CommitToken::from_bytes([255; 16]);
    if commit == op.accepted {
        commit = CommitToken::from_bytes([254; 16]);
    }
    let mut sizes = [admitted, 0, 0];
    for (index, kind) in [TerminalKind::Release, TerminalKind::Consume]
        .into_iter()
        .enumerate()
    {
        let finished = op.finish(Terminal {
            kind,
            binding: op.binding.clone(),
            action: match kind {
                TerminalKind::Release => op.binding.cancel.clone(),
                TerminalKind::Consume => op.binding.complete.clone(),
            },
            commit,
            actor: ObjId(u32::MAX),
            at: op.at.clone(),
            utc: i64::MIN,
        })?;
        let mut v = serde_json::to_value(&finished).map_err(bad)?;
        let terminal = v
            .get_mut("terminal")
            .and_then(Value::as_object_mut)
            .ok_or(Error::Integrity)?;
        terminal.insert(
            "at".into(),
            serde_json::to_value(ClockStamp {
                clock: mush_core::ClockRef::WorldIc {
                    generation: u64::MAX,
                },
                at: i64::MIN,
            })
            .map_err(bad)?,
        );
        bytes(&v, MAX_OPERATION_BYTES)?;
        sizes[index + 1] = envelope(OPERATIONS, &key(op.identity.order()), v)?;
    }
    Ok(sizes)
}
