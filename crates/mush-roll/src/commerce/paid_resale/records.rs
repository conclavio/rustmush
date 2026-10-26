//! Strict structural bindings. Only actual owner reads and opaque Economy
//! outputs can be published; deserializing these rows grants no rights.
use super::super::paid::PaidTermRecord;
use super::super::records::{PublicationError, Record};
use super::super::{CharterId, LicenceId, LineId, TermGeneration, WorldIcSeconds};
use mush_core::{ClockRef, ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId};
use mush_economy::managed_paid_resale::{
    PaidBirthRefV1, PaidHolderRefV1, PaidResaleBasisV1, PaidResaleDecisionKindV1,
    PaidResaleHistoryV1, PaidResaleOutcome, PaidResaleTermBasisV1,
};
use serde::{Deserialize, Serialize};
pub(crate) const MAX_OPERATIONS: usize = 65536;
pub(crate) const MAX_OPERATION_BYTES: usize = 40960;
fn bad(_: impl std::fmt::Display) -> PublicationError {
    PublicationError::Mismatch
}
pub(super) fn bounded<T: Serialize>(row: &T, max: usize) -> Result<Vec<u8>, PublicationError> {
    mush_core::canonical_json_bytes_with_limit(&serde_json::to_value(row).map_err(bad)?, max)
        .map_err(|_| PublicationError::Capacity)
}
pub(super) fn digest<T: Serialize>(
    domain: &str,
    row: &T,
) -> Result<DefinitionDigest, PublicationError> {
    DefinitionDigest::for_canonical_value_with_limit(
        domain,
        &serde_json::to_value(row).map_err(bad)?,
        MAX_OPERATION_BYTES,
    )
    .map_err(bad)
}
fn required<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}
fn decode<T: serde::de::DeserializeOwned + Serialize>(
    bytes: &[u8],
    max: usize,
) -> Result<T, PublicationError> {
    if bytes.is_empty() || bytes.len() > max {
        return Err(PublicationError::Capacity);
    }
    let row: T = serde_json::from_slice(bytes).map_err(bad)?;
    if bounded(&row, max)? != bytes {
        return Err(PublicationError::Invalid);
    }
    Ok(row)
}
macro_rules! checked {
 ($name:ident,$wire:ident,{$($field:ident:$ty:ty),* $(,)?})=>{
  #[derive(Clone,Debug,PartialEq,Eq,Serialize)]
  pub struct $name {$(pub(crate) $field:$ty),*}
  #[derive(Deserialize)]
  #[serde(deny_unknown_fields)]
  struct $wire {$($field:$ty),*}
  impl<'de> Deserialize<'de> for $name {
   fn deserialize<D:serde::Deserializer<'de>>(d:D)->Result<Self,D::Error>{
    let w=$wire::deserialize(d)?;let v=Self{$($field:w.$field),*};
    v.validate().map_err(serde::de::Error::custom)?;Ok(v)
   }
  }
 };
}
checked!(PaidResaleBinding,BindingWire,{
 licence:LicenceId,charter:CharterId,line:LineId,birth:PaidBirthRefV1,ownership:PaidHolderRefV1,
 term:PaidTermRecord,term_clock:ClockRef,economics_digest:DefinitionDigest,royalty_chain_digest:DefinitionDigest,
});
impl PaidResaleBinding {
    pub fn validate(&self) -> Result<(), PublicationError> {
        self.birth.validate().map_err(bad)?;
        self.ownership.validate().map_err(bad)?;
        self.term.validate()?;
        if self.licence != self.term.licence()
            || !self.term.terms().transferable()
            || self.ownership.generation >= 65537
            || !matches!(self.term_clock,ClockRef::WorldIc{generation}if generation>0)
            || (self.ownership.generation == 1
                && (self.ownership.operation != self.birth.operation
                    || self.ownership.digest != self.birth.digest))
        {
            return Err(PublicationError::Mismatch);
        }
        bounded(self, 16384).map(|_| ())
    }
    pub fn native_basis(&self) -> Result<PaidResaleBasisV1, PublicationError> {
        self.validate()?;
        let v = PaidResaleBasisV1 {
            licence: self.licence.get(),
            charter: self.charter.object(),
            line: self.line.get(),
            birth: self.birth.clone(),
            holder: self.ownership,
            term: PaidResaleTermBasisV1 {
                generation: self.term.generation().get(),
                operation: self.term.operation(),
                digest: Self::term_digest(&self.term)?,
                clock: self.term_clock.clone(),
                window: serde_json::from_value(
                    serde_json::to_value(self.term.window()).map_err(bad)?,
                )
                .map_err(bad)?,
                terms_digest: digest("roll.paid_resale.terms.v1", &self.term.terms())?,
                policy_digest: digest("roll.paid_resale.policy.v1", &self.term.policy())?,
            },
            economics_digest: self.economics_digest,
            royalty_chain_digest: self.royalty_chain_digest,
        };
        v.validate().map_err(bad)?;
        Ok(v)
    }
    pub(crate) fn term_digest(t: &PaidTermRecord) -> Result<DefinitionDigest, PublicationError> {
        t.validate()?;
        digest("roll.paid_resale.term.v1", t)
    }
    pub const fn licence(&self) -> LicenceId {
        self.licence
    }
    pub const fn charter(&self) -> CharterId {
        self.charter
    }
    pub const fn line(&self) -> LineId {
        self.line
    }
    pub fn birth(&self) -> &PaidBirthRefV1 {
        &self.birth
    }
    pub fn ownership(&self) -> &PaidHolderRefV1 {
        &self.ownership
    }
    pub fn term(&self) -> &PaidTermRecord {
        &self.term
    }
    pub fn term_clock(&self) -> &ClockRef {
        &self.term_clock
    }
    pub const fn economics_digest(&self) -> DefinitionDigest {
        self.economics_digest
    }
    pub const fn royalty_chain_digest(&self) -> DefinitionDigest {
        self.royalty_chain_digest
    }
}
checked!(PaidResaleRef,ResaleRefWire,{sequence:u64,operation:CommitToken,digest:DefinitionDigest});
impl PaidResaleRef {
    pub fn validate(&self) -> Result<(), PublicationError> {
        if !(1..=65536).contains(&self.sequence) {
            Err(PublicationError::Invalid)
        } else {
            Ok(())
        }
    }
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }
    pub const fn operation(&self) -> CommitToken {
        self.operation
    }
    pub const fn digest(&self) -> DefinitionDigest {
        self.digest
    }
}
checked!(ResaleClaimObservation,ClaimsWire,{generation:u64,next_transition:u64,open_paid:u32});
impl ResaleClaimObservation {
    pub fn validate(&self) -> Result<(), PublicationError> {
        if self.next_transition == 0
            || self.open_paid > 32768
            || (self.generation == 0 && (self.next_transition != 1 || self.open_paid != 0))
        {
            Err(PublicationError::Invalid)
        } else {
            Ok(())
        }
    }
    pub const fn generation(&self) -> u64 {
        self.generation
    }
    pub const fn next_transition(&self) -> u64 {
        self.next_transition
    }
    pub const fn open_paid(&self) -> u32 {
        self.open_paid
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
pub enum PaidResaleState {
    Funded {},
    Settled {
        terminal: CommitToken,
        ownership: PaidHolderRefV1,
    },
    Refunded {
        terminal: CommitToken,
        reason: PaidResaleDecisionKindV1,
    },
}
impl PaidResaleState {
    pub fn terminal(&self) -> Option<CommitToken> {
        match self {
            Self::Funded {} => None,
            Self::Settled { terminal, .. } | Self::Refunded { terminal, .. } => Some(*terminal),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PaidResaleRecord {
    pub(crate) format: u16,
    pub(crate) operation: CommitToken,
    pub(crate) action: DefinitionRef,
    pub(crate) sequence: u64,
    pub(crate) previous: Option<PaidResaleRef>,
    pub(crate) offer: CommitToken,
    pub(crate) offer_digest: DefinitionDigest,
    pub(crate) binding: PaidResaleBinding,
    pub(crate) claim_observation: ResaleClaimObservation,
    pub(crate) economic: PaidResaleHistoryV1,
    pub(crate) state: PaidResaleState,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResaleWire {
    format: u16,
    operation: CommitToken,
    action: DefinitionRef,
    sequence: u64,
    #[serde(deserialize_with = "required")]
    previous: Option<PaidResaleRef>,
    offer: CommitToken,
    offer_digest: DefinitionDigest,
    binding: PaidResaleBinding,
    claim_observation: ResaleClaimObservation,
    economic: PaidResaleHistoryV1,
    state: PaidResaleState,
}
impl<'de> Deserialize<'de> for PaidResaleRecord {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = ResaleWire::deserialize(d)?;
        let row = Self {
            format: w.format,
            operation: w.operation,
            action: w.action,
            sequence: w.sequence,
            previous: w.previous,
            offer: w.offer,
            offer_digest: w.offer_digest,
            binding: w.binding,
            claim_observation: w.claim_observation,
            economic: w.economic,
            state: w.state,
        };
        row.validate().map_err(serde::de::Error::custom)?;
        Ok(row)
    }
}
impl Record for PaidResaleRecord {
    const FAMILY: &'static str = "roll_mind_paid_resales";
    const MAX_BYTES: usize = MAX_OPERATION_BYTES;
    fn key(&self) -> String {
        self.operation.to_hex()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        self.binding.validate()?;
        self.claim_observation.validate()?;
        self.economic.digest().map_err(bad)?;
        let c = self.economic.consent();
        if self.format != 1
            || !(1..=65536).contains(&self.sequence)
            || self.operation != c.operation
            || self.action != c.effect
            || self.offer != c.terms.offer
            || self.offer_digest != c.offer_digest
            || c.terms.offer_action != super::protocol::action(0)?
            || self.action
                != super::protocol::action(
                    if c.mode == mush_economy::managed_paid_resale::PaidResaleModeV1::Immediate {
                        1
                    } else {
                        2
                    },
                )?
            || c.terms.basis != self.binding.native_basis()?
            || self.claim_observation.next_transition != c.claims.next_transition
            || self.claim_observation.open_paid != c.claims.open_paid
        {
            return Err(PublicationError::Mismatch);
        }
        match &self.previous {
            None if self.sequence == 1 => (),
            Some(p)
                if p.sequence.checked_add(1) == Some(self.sequence)
                    && p.operation != self.operation =>
            {
                p.validate()?
            }
            _ => return Err(PublicationError::Mismatch),
        }
        match (&self.state, self.economic.outcome()) {
            (PaidResaleState::Funded {}, PaidResaleOutcome::Funded) => (),
            (
                PaidResaleState::Settled {
                    terminal,
                    ownership,
                },
                PaidResaleOutcome::Settled,
            ) => {
                let expected = PaidHolderRecord::from_history(&self.binding, &self.economic)?;
                if *terminal != expected.operation || *ownership != expected.ownership()? {
                    return Err(PublicationError::Mismatch);
                }
            }
            (PaidResaleState::Refunded { terminal, reason }, PaidResaleOutcome::Refunded) => {
                let actual = self
                    .economic
                    .terminal()
                    .ok_or(PublicationError::Missing)?
                    .decision();
                if *terminal != actual.operation || *reason != actual.reason {
                    return Err(PublicationError::Mismatch);
                }
            }
            _ => return Err(PublicationError::Mismatch),
        }
        if let Some(t) = self.economic.terminal() {
            let action = &t.decision().action;
            let normal =
                super::protocol::action(if t.decision().reason.settles() { 3 } else { 4 })?;
            let registry = matches!(t.decision().reason,PaidResaleDecisionKindV1::RegistryRevoked{sanction,..}if sanction==t.decision().operation)
                && action.key().owner.as_str() == "roll"
                && action.key().kind.as_str() == "action"
                && action.key().id.as_str() == "registry_term_revocation"
                && action.version() == 1;
            // Historical Registry full refs are joined to actual cause/policy by audit.
            if action != &normal && !registry {
                return Err(PublicationError::Mismatch);
            }
        }
        bounded(self, Self::MAX_BYTES).map(|_| ())
    }
}
impl PaidResaleRecord {
    pub fn decode(bytes: &[u8]) -> Result<Self, PublicationError> {
        decode(bytes, Self::MAX_BYTES)
    }
    pub fn validate(&self) -> Result<(), PublicationError> {
        <Self as Record>::validate(self)
    }
    pub fn digest(&self) -> Result<DefinitionDigest, PublicationError> {
        self.validate()?;
        digest("roll.paid_resale.record.v1", self)
    }
    pub fn reference(&self) -> Result<PaidResaleRef, PublicationError> {
        Ok(PaidResaleRef {
            sequence: self.sequence,
            operation: self.operation,
            digest: self.digest()?,
        })
    }
    pub const fn operation(&self) -> CommitToken {
        self.operation
    }
    pub fn action(&self) -> &DefinitionRef {
        &self.action
    }
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }
    pub fn previous(&self) -> Option<&PaidResaleRef> {
        self.previous.as_ref()
    }
    pub const fn offer(&self) -> CommitToken {
        self.offer
    }
    pub const fn offer_digest(&self) -> DefinitionDigest {
        self.offer_digest
    }
    pub fn binding(&self) -> &PaidResaleBinding {
        &self.binding
    }
    pub fn claims(&self) -> &ResaleClaimObservation {
        &self.claim_observation
    }
    pub fn economic(&self) -> &PaidResaleHistoryV1 {
        &self.economic
    }
    pub fn state(&self) -> &PaidResaleState {
        &self.state
    }
}
checked!(PaidResaleHead,HeadWire,{format:u16,licence:LicenceId,current:PaidResaleRef,state:PaidResaleState});
impl Record for PaidResaleHead {
    const FAMILY: &'static str = "roll_mind_paid_resale_heads";
    const MAX_BYTES: usize = 4096;
    fn key(&self) -> String {
        self.licence.key()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        self.current.validate()?;
        if self.format != 1 {
            return Err(PublicationError::Invalid);
        }
        match &self.state {
            PaidResaleState::Settled { ownership, .. } => {
                ownership.validate().map_err(bad)?;
                if ownership.generation < 2 {
                    return Err(PublicationError::Invalid);
                }
            }
            PaidResaleState::Refunded { reason, .. } if reason.settles() => {
                return Err(PublicationError::Invalid);
            }
            _ => (),
        }
        bounded(self, Self::MAX_BYTES).map(|_| ())
    }
}
impl PaidResaleHead {
    pub fn validate(&self) -> Result<(), PublicationError> {
        <Self as Record>::validate(self)
    }
    pub(crate) fn from_record(row: &PaidResaleRecord) -> Result<Self, PublicationError> {
        row.validate()?;
        let h = Self {
            format: 1,
            licence: row.binding.licence,
            current: row.reference()?,
            state: row.state.clone(),
        };
        h.validate()?;
        Ok(h)
    }
    pub fn matches(&self, row: &PaidResaleRecord) -> Result<(), PublicationError> {
        if self != &Self::from_record(row)? {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
    pub const fn licence(&self) -> LicenceId {
        self.licence
    }
    pub fn current(&self) -> &PaidResaleRef {
        &self.current
    }
    pub fn state(&self) -> &PaidResaleState {
        &self.state
    }
}
checked!(HolderTerm,HolderTermWire,{generation:TermGeneration,operation:CommitToken,digest:DefinitionDigest});
impl HolderTerm {
    pub fn validate(&self) -> Result<(), PublicationError> {
        Ok(())
    }
}
checked!(PaidHolderRecord,HolderWire,{
 format:u16,licence:LicenceId,generation:u64,holder:ObjId,birth:PaidBirthRefV1,predecessor:PaidHolderRefV1,
 operation:CommitToken,action:DefinitionRef,acceptance:CommitToken,offer:CommitToken,offer_digest:DefinitionDigest,
 term:HolderTerm,accepted_at_ic:ClockStamp,economic_digest:DefinitionDigest,
});
impl Record for PaidHolderRecord {
    const FAMILY: &'static str = "roll_mind_paid_holders";
    const MAX_BYTES: usize = 32768;
    fn key(&self) -> String {
        Self::key_for(self.licence, self.generation)
    }
    fn validate(&self) -> Result<(), PublicationError> {
        self.birth.validate().map_err(bad)?;
        self.predecessor.validate().map_err(bad)?;
        self.term.validate()?;
        WorldIcSeconds::from_stamp(&self.accepted_at_ic).map_err(bad)?;
        if self.format != 1
            || !(2..=65537).contains(&self.generation)
            || self.predecessor.generation.checked_add(1) != Some(self.generation)
            || self.holder.0 == 0
            || self.operation == self.offer
            || self.acceptance == self.offer
            || self.operation == self.predecessor.operation
            || self.operation == self.birth.operation
            || self.action
                != super::protocol::action(if self.operation == self.acceptance {
                    1
                } else {
                    3
                })?
            || (self.predecessor.generation == 1
                && (self.predecessor.operation != self.birth.operation
                    || self.predecessor.digest != self.birth.digest))
        {
            return Err(PublicationError::Mismatch);
        }
        bounded(self, Self::MAX_BYTES).map(|_| ())
    }
}
impl PaidHolderRecord {
    pub fn validate(&self) -> Result<(), PublicationError> {
        <Self as Record>::validate(self)
    }
    pub fn key_for(id: LicenceId, generation: u64) -> String {
        format!("{}/{generation:016x}", id.key())
    }
    pub fn digest(&self) -> Result<DefinitionDigest, PublicationError> {
        self.validate()?;
        digest("roll.paid_holder.transfer.v1", self)
    }
    pub fn ownership(&self) -> Result<PaidHolderRefV1, PublicationError> {
        Ok(PaidHolderRefV1 {
            generation: self.generation,
            operation: self.operation,
            digest: self.digest()?,
        })
    }
    pub(crate) fn from_history(
        binding: &PaidResaleBinding,
        economic: &PaidResaleHistoryV1,
    ) -> Result<Self, PublicationError> {
        binding.validate()?;
        let economic_digest = economic.digest().map_err(bad)?;
        if economic.outcome() != PaidResaleOutcome::Settled
            || economic.consent().terms.basis != binding.native_basis()?
        {
            return Err(PublicationError::Mismatch);
        }
        let c = economic.consent();
        let (operation, action) = match economic.terminal() {
            Some(t) => (t.decision().operation, t.decision().action.clone()),
            None => (c.operation, c.effect.clone()),
        };
        let r = Self {
            format: 1,
            licence: binding.licence,
            generation: binding
                .ownership
                .generation
                .checked_add(1)
                .ok_or(PublicationError::Capacity)?,
            holder: c.terms.buyer,
            birth: binding.birth.clone(),
            predecessor: binding.ownership,
            operation,
            action,
            acceptance: c.operation,
            offer: c.terms.offer,
            offer_digest: c.offer_digest,
            term: HolderTerm {
                generation: binding.term.generation(),
                operation: binding.term.operation(),
                digest: PaidResaleBinding::term_digest(&binding.term)?,
            },
            accepted_at_ic: c.accepted_at_ic.clone(),
            economic_digest,
        };
        r.validate()?;
        Ok(r)
    }
    pub fn matches_record(&self, row: &PaidResaleRecord) -> Result<(), PublicationError> {
        row.validate()?;
        if self != &Self::from_history(&row.binding, &row.economic)? {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
    pub const fn licence(&self) -> LicenceId {
        self.licence
    }
    pub const fn generation(&self) -> u64 {
        self.generation
    }
    pub const fn holder(&self) -> ObjId {
        self.holder
    }
    pub fn birth(&self) -> &PaidBirthRefV1 {
        &self.birth
    }
    pub fn predecessor(&self) -> &PaidHolderRefV1 {
        &self.predecessor
    }
    pub const fn operation(&self) -> CommitToken {
        self.operation
    }
    pub const fn acceptance(&self) -> CommitToken {
        self.acceptance
    }
    pub const fn term_generation(&self) -> TermGeneration {
        self.term.generation
    }
}
