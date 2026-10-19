//! Fresh buyer-selected renewal facts. A lapsed predecessor is valid history;
//! current issuer/Registry/immediate-parent rights are separately re-resolved.
//! Existing paid use/source never calls this fresh-issuance reader.

use super::super::{
    LicenceId, WorldIcSeconds,
    entitlement::{
        history::{self, Birth},
        reader::{EntitlementSaleFacts, EntitlementSaleReader, EntitlementSaleRequest},
    },
    families::CommerceFamilies,
    rating::{RatingCompatibility, RatingFamilies},
    records::{PublicationError, Record},
};
use super::RenewalBindingV1;
use crate::RollStore;
use mush_core::{
    CommitToken, DefinitionDigest, DefinitionRef, Flags, ModError, ObjId, ObjKind, Registrar,
    mechanics::{
        DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
    },
    participant::OwnerPrepareContext,
    read_service::{OwnerReadContext, OwnerReadError, OwnerReadHandle, OwnerReadService},
};
use mush_economy::EconomyStore;
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Serialize)]
pub struct RenewalRequest {
    licence: LicenceId,
}
impl RenewalRequest {
    pub fn own(licence: LicenceId) -> Self {
        Self { licence }
    }
}

/// Only this actual owner reader constructs the observation. The writer binds
/// it to the exact preparation action/commit and retains its current title and
/// clock checks through Root prepare/stage/take. It carries no source bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RenewalFacts {
    binding: RenewalBindingV1,
    predecessor_term_end: WorldIcSeconds,
    prior_quote: CommitToken,
    prior_quote_digest: DefinitionDigest,
    sale: EntitlementSaleFacts,
}
impl RenewalFacts {
    pub fn binding(&self) -> &RenewalBindingV1 {
        &self.binding
    }
    /// Earliest safe replacement time, captured from the actually proved current term.
    pub const fn predecessor_term_end(&self) -> WorldIcSeconds {
        self.predecessor_term_end
    }
    pub const fn prior_quote(&self) -> CommitToken {
        self.prior_quote
    }
    pub const fn prior_quote_digest(&self) -> DefinitionDigest {
        self.prior_quote_digest
    }
    pub fn sale(&self) -> &EntitlementSaleFacts {
        &self.sale
    }
    pub(crate) fn check_fresh(
        &self,
        ctx: &OwnerPrepareContext<'_>,
    ) -> Result<(), PublicationError> {
        if self.binding.holder() != ctx.invocation().actor()
            || self.sale.requester() != self.binding.holder()
            || ctx.world().world_ic_now().at < self.predecessor_term_end.get()
        {
            return Err(PublicationError::Mismatch);
        }
        self.sale.check_fresh(ctx)
    }
}

pub struct RenewalReader {
    families: CommerceFamilies,
    ratings: RatingFamilies,
    sale: EntitlementSaleReader,
}
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
fn integrity(_: impl std::fmt::Display) -> OwnerReadError {
    OwnerReadError::Integrity
}
impl RenewalReader {
    pub fn request(
        reg: &mut Registrar<'_>,
        manifest: &DefinitionRef,
    ) -> Result<OwnerReadHandle<Self>, ModError> {
        let reference = DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new(crate::NS).map_err(error)?,
                descriptor(manifest)?,
            )
            .map_err(error)?;
        reg.request_owner_read::<Self>(reference)
    }
}
pub(crate) fn descriptor(manifest: &DefinitionRef) -> Result<DefinitionDescriptor, ModError> {
    super::super::holding_reader::descriptor(manifest)?;
    Ok(DefinitionDescriptor::new(DefinitionKind::new("owner_read").map_err(error)?,
        DefinitionId::new("personal_mind_renewal").map_err(error)?,1,
        serde_json::json!({"format":1,"protocol":"personal_mind_renewal_v1","personal_pin":true,
            "own_current_predecessor":true,"current_issuer_parent_resale":true,"immutable_acquisition":true,
            "expired_predecessor_permitted":true,"source_bytes":false,"manifest":manifest}))
        .map_err(error)?.with_references(vec![manifest.clone()]))
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    families: CommerceFamilies,
    ratings: RatingFamilies,
    manifest: &DefinitionRef,
    expected: RatingCompatibility,
) -> Result<(), ModError> {
    if reg.module() != crate::NS || manifest.digest() != expected.digest() {
        return Err(ModError::msg("renewal reader composition differs"));
    }
    let definition = reg.definition(descriptor(manifest)?)?;
    let sale = EntitlementSaleReader::for_renewal(families.clone(), ratings.clone(), expected);
    reg.owner_read_service(
        definition,
        BTreeSet::from([OwnerId::new("market").map_err(error)?]),
        BTreeSet::from([
            OwnerId::new("economy").map_err(error)?,
            OwnerId::new("tacsim").map_err(error)?,
        ]),
        RenewalReader {
            families,
            ratings,
            sale,
        },
    )
}
impl OwnerReadService for RenewalReader {
    type State = RollStore;
    type Request = RenewalRequest;
    type Output = RenewalFacts;
    fn read(
        &self,
        roll: &RollStore,
        ctx: &OwnerReadContext<'_>,
        request: RenewalRequest,
    ) -> Result<RenewalFacts, OwnerReadError> {
        if ctx.requester().as_str() != "market"
            || ctx.principal() != &(mush_core::action::PrincipalId::Object { id: ctx.actor() })
        {
            return Err(OwnerReadError::Authority);
        }
        roll.commerce_ready()
            .map_err(|_| OwnerReadError::Unavailable)?;
        if ctx
            .world()
            .get(ctx.actor())
            .is_none_or(|o| o.kind != ObjKind::Player || o.flags.contains(Flags::GOING))
        {
            return Err(OwnerReadError::Refused);
        }
        let licence = history::read(ctx, &self.families.paid_licences, request.licence.key())
            .map_err(integrity)?;
        if licence.holder() != ctx.actor() {
            return Err(OwnerReadError::Refused);
        }
        let term = history::read(
            ctx,
            &self.families.paid_terms,
            format!("{}/{}", licence.id().key(), licence.term().key()),
        )
        .map_err(integrity)?;
        // This proof has no temporal-use predicate. Renewing an expired own
        // predecessor must not invent an extension before the actual Grant.
        let proved = history::current(ctx, &self.families, &self.ratings, licence, term)
            .map_err(integrity)?;
        let Birth::Entitlement(birth) = proved.birth() else {
            return Err(OwnerReadError::Refused);
        };
        let predecessor_term_end = term.window().term_end();
        if ctx.world().world_ic_now().at < predecessor_term_end.get() {
            return Err(OwnerReadError::Refused);
        }
        let economics = proved.economics().ok_or(OwnerReadError::Integrity)?;
        let binding = RenewalBindingV1::capture(licence, term, economics).map_err(integrity)?;
        let filed =
            history::read(ctx, &self.families.filed, binding.filed.key()).map_err(integrity)?;
        let filing = history::read(ctx, &self.families.operations, filed.operation.to_hex())
            .map_err(integrity)?;
        if filed.content.revision() != binding.revision
            || filed.content.position().charter != binding.charter
            || filing.filed != binding.filed
            || filing.charter != binding.charter
        {
            return Err(OwnerReadError::Integrity);
        }
        let economy = ctx
            .world()
            .module_state::<EconomyStore>()
            .ok_or(OwnerReadError::Unavailable)?;
        let title = economy
            .snapshot()
            .asset(ObjId(binding.charter.get()))
            .ok_or(OwnerReadError::Unavailable)?;
        let mush_org::PrincipalId::Object(seller) = &title.title_owner else {
            return Err(OwnerReadError::Refused);
        };
        let sale = self.sale.read(
            roll,
            ctx,
            EntitlementSaleRequest::listed(*seller, filing.licence),
        )?;
        if sale.filed() != binding.filed
            || sale.revision() != binding.revision
            || sale.charter() != binding.charter
            || sale.line() != binding.line
            || sale.rating() != birth.rating()
            || sale.minimum() > binding.terms.class()
            || sale.royalty().reference().map_err(integrity)? != binding.grant.royalty_chain()
            || !sale.materialization().creates().is_empty()
        {
            return Err(OwnerReadError::Integrity);
        }
        Ok(RenewalFacts {
            binding,
            predecessor_term_end,
            prior_quote: birth.quote(),
            prior_quote_digest: birth.quote_digest(),
            sale,
        })
    }
}
