//! Booking-scoped private facts. Only committed receipts can satisfy escrow.
use super::store::{Booking, BookingKey, Phase};
use mush_core::action_state::StateFamilyHandle;
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::read_service::{
    OwnerReadContext, OwnerReadError, OwnerReadHandle, OwnerReadService,
};
use mush_core::{ModError, ObjId, Registrar, World};
use mush_economy::{
    ContractFactScope, ContractId, EconomyStore, ExternalRef, FactProvider, FactPurpose, FactState,
    ScopedFactContext,
};
use mush_org::PrincipalId;
use serde::Serialize;
use std::collections::BTreeSet;
use std::sync::{Arc, OnceLock};

pub const PROVIDER: &str = "tacsim.passenger";
pub const DELIVERED: &str = "tacsim.passenger.delivered";
pub const CANCELLED: &str = "tacsim.passenger.cancelled";
pub(crate) const READER_ID: &str = "passenger_contract_facts";

#[derive(Clone, Default)]
pub struct PassengerFactProvider(Arc<OnceLock<OwnerReadHandle<Reader>>>);
#[derive(Clone)]
struct AttachmentReader(OwnerReadHandle<Reader>);
impl std::fmt::Debug for PassengerFactProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PassengerFactProvider")
            .finish_non_exhaustive()
    }
}
impl PassengerFactProvider {
    fn scoped(&self, ctx: &ScopedFactContext<'_>, fact: &ExternalRef) -> Option<Reply> {
        let query = || Query {
            scope: ctx.scope().clone(),
            fact: fact.clone(),
        };
        if let Ok(reply) = ctx.read_owner(self.0.get()?, query()) {
            return Some(reply);
        }
        if ctx.scope().purpose() != FactPurpose::Attach {
            return None;
        }
        ctx.read_owner(&ctx.world().module_state::<AttachmentReader>()?.0, query())
            .ok()
    }
}
impl FactProvider for PassengerFactProvider {
    fn provider(&self) -> &str {
        PROVIDER
    }
    fn requires_scope(&self, _: &ExternalRef) -> bool {
        true
    }
    fn resolve(&self, _: &World, _: &str, _: &str, _: Option<u64>) -> Option<FactState> {
        None
    }
    fn authorize_scope(&self, ctx: &ScopedFactContext<'_>, fact: &ExternalRef) -> bool {
        self.scoped(ctx, fact).is_some()
    }
    fn resolve_scoped(&self, ctx: &ScopedFactContext<'_>, fact: &ExternalRef) -> Option<FactState> {
        self.scoped(ctx, fact)?.truth.map(|t| FactState {
            satisfied: true,
            actor: Some(t.actor),
            at: t.at_utc,
        })
    }
    fn register_scoped(&self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        if reg.module() != "economy" {
            return Err(error("passenger facts require Economy registration"));
        }
        let definition = DefinitionRegistryBuilder::new()
            .register(OwnerId::new(crate::NS).map_err(error)?, descriptor()?)
            .map_err(error)?;
        let handle = reg.request_owner_read::<Reader>(definition)?;
        self.0
            .set(handle)
            .map_err(|_| error("passenger facts already registered"))
    }
}
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
pub(crate) fn descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(DefinitionKind::new("owner_read").map_err(error)?,DefinitionId::new(READER_ID).map_err(error)?,1,
        serde_json::json!({"format":1,"purpose":"passenger_contract_fact","scope":"exact_booking_contract","truth":"committed_arrival_or_cancellation","public":false})).map_err(error)
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    bookings: StateFamilyHandle<BookingKey, Booking>,
    economy_wired: bool,
) -> Result<(), ModError> {
    let definition = reg.definition(descriptor()?)?;
    if !economy_wired {
        return Ok(());
    }
    reg.owner_read_service(
        definition.clone(),
        BTreeSet::from([
            OwnerId::new("economy").map_err(error)?,
            OwnerId::new(crate::NS).map_err(error)?,
        ]),
        BTreeSet::from([OwnerId::new("economy").map_err(error)?]),
        Reader { bookings },
    )?;
    let handle = reg.request_owner_read::<Reader>(definition)?;
    reg.state(AttachmentReader(handle));
    Ok(())
}
#[derive(Serialize)]
struct Query {
    scope: ContractFactScope,
    fact: ExternalRef,
}
#[derive(Serialize)]
struct Reply {
    truth: Option<Truth>,
}
#[derive(Serialize)]
struct Truth {
    actor: ObjId,
    at_utc: i64,
}
struct Reader {
    bookings: StateFamilyHandle<BookingKey, Booking>,
}
impl OwnerReadService for Reader {
    type State = crate::TacsimStore;
    type Request = Query;
    type Output = Reply;
    fn read(
        &self,
        _: &Self::State,
        ctx: &OwnerReadContext<'_>,
        Query { scope, fact }: Query,
    ) -> Result<Reply, OwnerReadError> {
        let refuse = OwnerReadError::Unavailable;
        if !(ctx.requester().as_str() == "economy"
            || (ctx.requester().as_str() == crate::NS && scope.purpose() == FactPurpose::Attach))
            || !scope.matches_read(ctx, &fact)
            || scope.generation() != 0
            || fact.provider != PROVIDER
            || !matches!(fact.kind.as_str(), DELIVERED | CANCELLED)
            || fact.revision != Some(1)
        {
            return Err(refuse);
        }
        let (id_text, digest) = fact.id.split_once('/').ok_or(refuse)?;
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(refuse);
        }
        let id = id_text
            .parse::<u64>()
            .ok()
            .filter(|id| *id <= i64::MAX as u64 && id.to_string() == id_text)
            .ok_or(refuse)?;
        if scope.contract() != ContractId(id) {
            return Err(refuse);
        }
        let candidate;
        let row = if scope.purpose() == FactPurpose::Attach {
            candidate =
                super::economy::attachment_candidate(ctx.world(), scope.actor(), ContractId(id))
                    .ok_or(refuse)?;
            &candidate
        } else {
            ctx.action_state(&self.bookings, &BookingKey(id))?
                .ok_or(refuse)?
        };
        row.check().map_err(|_| OwnerReadError::Integrity)?;
        if row.id != id
            || row.contract != id
            || super::economy::fact_ref(row, fact.kind == CANCELLED) != fact
            || scope.issuer() != &PrincipalId::Object(row.captain)
        {
            return Err(refuse);
        }
        if scope.purpose() == FactPurpose::Attach {
            if scope.actor() != row.captain
                || scope.consent_hash()
                    != mush_economy::consent_hash(
                        &PrincipalId::Object(row.captain),
                        &super::economy::expected_consent(row),
                    )
            {
                return Err(refuse);
            }
            return Ok(Reply { truth: None });
        }
        if !super::economy::valid(ctx.world(), row)
            || scope.attachment_actor(&fact) != Some(row.captain)
            || (scope.purpose() == FactPurpose::Read
                && !mush_economy::acts_for(
                    ctx.world(),
                    scope.actor(),
                    &PrincipalId::Object(row.captain),
                )
                && !mush_economy::acts_for(
                    ctx.world(),
                    scope.actor(),
                    &PrincipalId::Object(row.npc),
                ))
        {
            return Err(refuse);
        }
        let current = ctx
            .world()
            .module_state::<EconomyStore>()
            .and_then(|s| s.snapshot().contract(ContractId(id)))
            .ok_or(refuse)?;
        if scope.consent_hash() != current.consent_hash {
            return Err(refuse);
        }
        let funded =
            super::economy::active(ctx.world(), row) || super::economy::settled(ctx.world(), row);
        let stamp = if !funded {
            None
        } else {
            match fact.kind.as_str() {
                DELIVERED if row.phase == Phase::Delivered => row.delivered.as_ref(),
                CANCELLED if row.phase == Phase::Cancelled => row.cancelled.as_ref(),
                _ => None,
            }
        };
        Ok(Reply {
            truth: stamp.map(|s| Truth {
                actor: row.captain,
                at_utc: s.at_utc,
            }),
        })
    }
}
