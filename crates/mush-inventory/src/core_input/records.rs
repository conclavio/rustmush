use super::*;
use crate::{Lineage, Lot, LotId};
use mush_core::{
    BoundedScalar, CanonicalHash, ClockRef, ClockStamp, IdempotencyKey, IdempotencyKeyText,
    IdempotencyScope, ObjId, PrincipalId, ReservationBook, ReservationId, ReservationRecord,
    ReservationState, ResourceRef, ScopedIdempotencyKey, SubjectKey,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreInputIdentity {
    order: u64,
    intent: DefinitionDigest,
}
impl CoreInputIdentity {
    pub const fn new(order: u64, intent: DefinitionDigest) -> Self {
        Self { order, intent }
    }
    pub const fn order(self) -> u64 {
        self.order
    }
    pub const fn intent(self) -> DefinitionDigest {
        self.intent
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoreInputState {
    Held,
    Released,
    Consumed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum TerminalKind {
    Release,
    Consume,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Terminal {
    pub(super) kind: TerminalKind,
    pub(super) binding: CoreInputBinding,
    pub(super) action: DefinitionRef,
    pub(super) commit: CommitToken,
    pub(super) actor: ObjId,
    pub(super) at: ClockStamp,
    pub(super) utc: i64,
}

/// Data returned only by actual Inventory readback/publication. Decoding this
/// history never creates a preparation witness or permission to consume a lot.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreInputOperation {
    pub(super) format: u16,
    pub(super) identity: CoreInputIdentity,
    pub(super) sequence: u64,
    pub(super) attempt: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) previous: Option<u64>,
    pub(super) lot: Lot,
    pub(super) binding: CoreInputBinding,
    pub(super) accepted: CommitToken,
    pub(super) actor: ObjId,
    pub(super) at: ClockStamp,
    pub(super) utc: i64,
    pub(super) scalar: BoundedScalar,
    pub(super) book: ReservationBook,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) terminal: Option<Terminal>,
}
pub(super) fn stamp(stamp: &ClockStamp) -> Result<(), Error> {
    if !matches!(stamp.clock, ClockRef::WorldIc { generation } if generation > 0) {
        return Err(Error::Integrity);
    }
    Ok(())
}
fn resource(lot: LotId) -> Result<ResourceRef, Error> {
    Ok(ResourceRef::new(
        crate::record::owner(),
        SubjectKey::Scoped {
            owner: crate::record::owner(),
            class: cid("core_input_lot")?,
            id: IdempotencyKeyText::parse_canonical(format!("lot{:016x}", lot.0)).map_err(bad)?,
        },
        cid("core_input")?,
    ))
}
fn holder(identity: CoreInputIdentity) -> Result<ScopedIdempotencyKey, Error> {
    let owner = mush_core::mechanics::OwnerId::new("industry").map_err(bad)?;
    Ok(ScopedIdempotencyKey::new(
        IdempotencyScope {
            principal: PrincipalId::Scoped {
                owner: owner.clone(),
                class: cid("workorder")?,
                id: IdempotencyKeyText::parse_canonical(format!("wo{:016x}", identity.order))
                    .map_err(bad)?,
            },
            owner,
            namespace: cid("core_input")?,
        },
        0,
        IdempotencyKey::Deterministic(
            CanonicalHash::for_canonical_value(
                "inventory.core_input_holder.v1",
                1,
                &serde_json::json!(identity),
                1024,
            )
            .map_err(bad)?,
        ),
    ))
}
pub(super) fn held(
    identity: CoreInputIdentity,
    lot: LotId,
    at: &ClockStamp,
) -> Result<(BoundedScalar, ReservationBook), Error> {
    stamp(at)?;
    let resource = resource(lot)?;
    let holder = holder(identity)?;
    let scalar = BoundedScalar::new(0, 1, 1, 1).map_err(bad)?;
    let id = ReservationId::derive(&holder, &resource).map_err(bad)?;
    let record = ReservationRecord::held(
        id,
        resource.clone(),
        holder,
        1,
        crate::reservation_deadline(at).map_err(bad)?,
    )
    .map_err(bad)?;
    let book = ReservationBook::new(resource, scalar).map_err(bad)?;
    let book = book
        .checked_hold(book.revision(), scalar, record, at)
        .map_err(bad)?;
    Ok((scalar, book))
}
impl CoreInputOperation {
    pub fn identity(&self) -> CoreInputIdentity {
        self.identity
    }
    pub fn lot(&self) -> &Lot {
        &self.lot
    }
    pub fn accepted(&self) -> CommitToken {
        self.accepted
    }
    pub fn action(&self) -> &DefinitionRef {
        &self.binding.accept
    }
    pub fn terminal_commit(&self) -> Option<CommitToken> {
        self.terminal.as_ref().map(|t| t.commit)
    }
    pub fn terminal_action(&self) -> Option<&DefinitionRef> {
        self.terminal.as_ref().map(|t| &t.action)
    }
    pub fn reservation(&self) -> Result<ReservationId, Error> {
        let [r] = self.book.records() else {
            return Err(Error::Integrity);
        };
        Ok(r.id())
    }
    pub fn state(&self) -> Result<CoreInputState, Error> {
        let [r] = self.book.records() else {
            return Err(Error::Integrity);
        };
        match r.state() {
            ReservationState::Held => Ok(CoreInputState::Held),
            ReservationState::Released => Ok(CoreInputState::Released),
            ReservationState::Consumed { .. } => Ok(CoreInputState::Consumed),
            ReservationState::Expired => Err(Error::Integrity),
        }
    }
    pub(super) fn admission(&self) -> Result<Self, Error> {
        let (scalar, book) = held(self.identity, self.lot.id(), &self.at)?;
        Ok(Self {
            scalar,
            book,
            terminal: None,
            ..self.clone()
        })
    }
    pub(super) fn finish(&self, t: Terminal) -> Result<Self, Error> {
        if self.state()? != CoreInputState::Held || self.terminal.is_some() {
            return Err(Error::Reserved);
        }
        let id = self.reservation()?;
        let (book, scalar) = match t.kind {
            TerminalKind::Release => (
                self.book
                    .checked_release(self.book.revision(), self.scalar, id, 1)
                    .map_err(bad)?,
                self.scalar,
            ),
            TerminalKind::Consume => self
                .book
                .checked_consume(self.book.revision(), self.scalar, id, 1, &t.at, t.commit)
                .map_err(bad)?,
        };
        Ok(Self {
            scalar,
            book,
            terminal: Some(t),
            ..self.clone()
        })
    }
    pub(super) fn validate(&self) -> Result<(), Error> {
        self.binding.validate()?;
        stamp(&self.at)?;
        if self.format != 1
            || !(1..=MAX_OPERATIONS).contains(&self.sequence)
            || !(1..=self.sequence).contains(&self.attempt)
            || (self.attempt == 1) != self.previous.is_none()
            || self.previous == Some(self.identity.order)
            || self.lot.quantity() != 1
            || self.lot.depth() != 0
            || !matches!(self.lot.lineage(), Lineage::Origin { .. })
        {
            return Err(Error::Integrity);
        }
        let initial = self.admission()?;
        let expected = if let Some(t) = &self.terminal {
            t.binding.validate()?;
            stamp(&t.at)?;
            if t.commit == self.accepted
                || &t.action
                    != match t.kind {
                        TerminalKind::Release => &t.binding.cancel,
                        TerminalKind::Consume => &t.binding.complete,
                    }
            {
                return Err(Error::Integrity);
            }
            if t.kind == TerminalKind::Consume
                && (t.at.clock != self.at.clock || t.at.at < self.at.at)
            {
                return Err(Error::Integrity);
            }
            initial.finish(t.clone())?
        } else {
            initial
        };
        if self.scalar != expected.scalar || self.book != expected.book {
            return Err(Error::Integrity);
        }
        encode(self).map_err(bad)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Head {
    pub(super) format: u16,
    pub(super) lot: LotId,
    pub(super) identity: CoreInputIdentity,
    pub(super) sequence: u64,
    pub(super) attempt: u64,
}
impl Head {
    pub(super) fn from_operation(op: &CoreInputOperation) -> Self {
        Self {
            format: 1,
            lot: op.lot.id(),
            identity: op.identity,
            sequence: op.sequence,
            attempt: op.attempt,
        }
    }
    pub(super) fn validate(&self) -> Result<(), Error> {
        if self.format != 1
            || !(1..=MAX_OPERATIONS).contains(&self.sequence)
            || !(1..=self.sequence).contains(&self.attempt)
        {
            return Err(Error::Integrity);
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Census {
    pub(super) format: u16,
    pub(super) count: u64,
    pub(super) chain: DefinitionDigest,
}
impl Census {
    pub(super) fn empty() -> Result<Self, Error> {
        Ok(Self {
            format: 1,
            count: 0,
            chain: digest(
                "inventory.core_input_genesis.v1",
                &serde_json::json!({"format":1}),
            )?,
        })
    }
    pub(super) fn push(&self, op: &CoreInputOperation) -> Result<Self, Error> {
        self.validate()?;
        op.validate()?;
        if self.count >= MAX_OPERATIONS || op.sequence != self.count + 1 {
            return Err(Error::Capacity);
        }
        Ok(Self {
            format: 1,
            count: op.sequence,
            chain: digest(
                "inventory.core_input_chain.v1",
                &serde_json::json!({"previous":self.chain,"admission":op.admission()?}),
            )?,
        })
    }
    pub(super) fn validate(&self) -> Result<(), Error> {
        if self.format != 1 || self.count > MAX_OPERATIONS {
            Err(Error::Integrity)
        } else {
            Ok(())
        }
    }
}
