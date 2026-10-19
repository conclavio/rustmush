//! Fresh buyer-selected renewal facts. A lapsed predecessor is valid history;
//! current issuer/Registry/immediate-parent rights are separately re-resolved.
//! Existing paid use/source never calls this fresh-issuance reader.

use super::super::{
    LicenceId, WorldIcSeconds,
    entitlement::history::{self, Birth},
    families::CommerceFamilies,
    house_catalogue::{HouseSaleFacts, HouseSaleReader, HouseSaleRequest},
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
#[serde(untagged)]
pub enum RenewalRequest {
    Own { licence: LicenceId },
    Inventory { inventory_after: Option<String> },
    Operation { operation: CommitToken },
    Status { status_licence: LicenceId },
}
impl RenewalRequest {
    pub fn own(licence: LicenceId) -> Self {
        Self::Own { licence }
    }
    pub fn inventory(after: Option<String>) -> Self {
        Self::Inventory {
            inventory_after: after,
        }
    }
    pub fn status(licence: LicenceId) -> Self {
        Self::Status {
            status_licence: licence,
        }
    }
    pub fn operation(operation: CommitToken) -> Self {
        Self::Operation { operation }
    }
}
#[derive(Serialize)]
#[serde(untagged)]
pub enum RenewalRead {
    Facts(RenewalFacts),
    Inventory(RenewalInventory),
    Operation(super::RenewalOperationV1),
}
impl RenewalRead {
    pub fn into_facts(self) -> Result<RenewalFacts, OwnerReadError> {
        match self {
            Self::Facts(v) => Ok(v),
            _ => Err(OwnerReadError::Integrity),
        }
    }
    pub fn into_inventory(self) -> Result<RenewalInventory, OwnerReadError> {
        match self {
            Self::Inventory(v) => Ok(v),
            _ => Err(OwnerReadError::Integrity),
        }
    }
    pub fn into_operation(self) -> Result<super::RenewalOperationV1, OwnerReadError> {
        match self {
            Self::Operation(v) => Ok(v),
            _ => Err(OwnerReadError::Integrity),
        }
    }
}
/// Bounded source-free inventory, not a fresh issuance approval.
#[derive(Serialize)]
pub struct RenewalInventory {
    entries: Vec<RenewalInventoryEntry>,
    next_after: Option<String>,
}
impl RenewalInventory {
    pub fn entries(&self) -> &[RenewalInventoryEntry] {
        &self.entries
    }
    pub fn into_single(mut self) -> Result<RenewalInventoryEntry, OwnerReadError> {
        if self.entries.len() != 1 || self.next_after.is_some() {
            return Err(OwnerReadError::Integrity);
        }
        self.entries.pop().ok_or(OwnerReadError::Integrity)
    }
    pub fn next_after(&self) -> Option<&str> {
        self.next_after.as_deref()
    }
}
#[derive(Serialize)]
pub struct RenewalInventoryEntry {
    licence: LicenceId,
    holder: ObjId,
    generation: super::super::TermGeneration,
    term_end: WorldIcSeconds,
    last_operation: Option<CommitToken>,
    attempted: bool,
}
impl RenewalInventoryEntry {
    pub const fn licence(&self) -> LicenceId {
        self.licence
    }
    pub const fn holder(&self) -> ObjId {
        self.holder
    }
    pub const fn generation(&self) -> super::super::TermGeneration {
        self.generation
    }
    pub const fn term_end(&self) -> WorldIcSeconds {
        self.term_end
    }
    pub const fn last_operation(&self) -> Option<CommitToken> {
        self.last_operation
    }
    pub const fn attempted(&self) -> bool {
        self.attempted
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
    sale: HouseSaleFacts,
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
    pub fn sale(&self) -> &HouseSaleFacts {
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
        self.sale.check_fresh(ctx, false)
    }
}

pub struct RenewalReader {
    families: CommerceFamilies,
    ratings: RatingFamilies,
    sale: HouseSaleReader,
    service: bool,
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
        Self::request_configured(reg, manifest, false)
    }
    pub fn request_configured(
        reg: &mut Registrar<'_>,
        manifest: &DefinitionRef,
        service: bool,
    ) -> Result<OwnerReadHandle<Self>, ModError> {
        let reference = DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new(crate::NS).map_err(error)?,
                configured_descriptor(manifest, service)?,
            )
            .map_err(error)?;
        reg.request_owner_read::<Self>(reference)
    }
}
pub(crate) fn descriptor(manifest: &DefinitionRef) -> Result<DefinitionDescriptor, ModError> {
    super::super::holding_reader::descriptor(manifest)?;
    Ok(DefinitionDescriptor::new(DefinitionKind::new("owner_read").map_err(error)?,
        DefinitionId::new("house_mind_renewal").map_err(error)?,1,
        serde_json::json!({"format":1,"protocol":"house_mind_renewal_v1","personal_pin":true,
            "own_current_predecessor":true,"current_organization_scope_title":true,"immutable_acquisition":true,
            "expired_predecessor_permitted":true,"source_bytes":false,"manifest":manifest}))
        .map_err(error)?.with_references(vec![manifest.clone()]))
}
pub(crate) fn configured_descriptor(
    manifest: &DefinitionRef,
    service: bool,
) -> Result<DefinitionDescriptor, ModError> {
    if !service {
        return descriptor(manifest);
    }
    super::super::holding_reader::descriptor(manifest)?;
    DefinitionDescriptor::new(DefinitionKind::new("owner_read").map_err(error)?,
        DefinitionId::new("house_mind_renewal").map_err(error)?, 2,
        serde_json::json!({"format":2,"protocol":"house_mind_renewal_service_v1","own_current_predecessor":true,"current_organization_scope_title":true,"immutable_acquisition":true,"source_bytes":false,"service_actor":1,"inventory_page":4,"manifest":manifest}))
        .map_err(error).map(|d|d.with_references(vec![manifest.clone()]))
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    families: CommerceFamilies,
    ratings: RatingFamilies,
    manifest: &DefinitionRef,
    expected: RatingCompatibility,
    service: bool,
) -> Result<(), ModError> {
    if reg.module() != crate::NS || manifest.digest() != expected.digest() {
        return Err(ModError::msg("renewal reader composition differs"));
    }
    let definition = reg.definition(configured_descriptor(manifest, service)?)?;
    let sale = HouseSaleReader::for_renewal(families.clone(), ratings.clone(), expected);
    reg.owner_read_service(
        definition,
        BTreeSet::from([OwnerId::new("market").map_err(error)?]),
        BTreeSet::from([
            OwnerId::new("economy").map_err(error)?,
            OwnerId::new("org").map_err(error)?,
            OwnerId::new("tacsim").map_err(error)?,
        ]),
        RenewalReader {
            families,
            ratings,
            sale,
            service,
        },
    )
}
impl OwnerReadService for RenewalReader {
    type State = RollStore;
    type Request = RenewalRequest;
    type Output = RenewalRead;
    fn read(
        &self,
        roll: &RollStore,
        ctx: &OwnerReadContext<'_>,
        request: RenewalRequest,
    ) -> Result<RenewalRead, OwnerReadError> {
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
        let licence = match request {
            RenewalRequest::Own { licence } => licence,
            RenewalRequest::Inventory { inventory_after } if self.service => {
                return self
                    .inventory(roll, ctx, inventory_after)
                    .map(RenewalRead::Inventory);
            }
            RenewalRequest::Operation { operation } if self.service => {
                return self
                    .operation(roll, ctx, operation)
                    .map(RenewalRead::Operation);
            }
            RenewalRequest::Status { status_licence } if self.service => {
                let licence =
                    history::read(ctx, &self.families.paid_licences, status_licence.key())
                        .map_err(integrity)?;
                if licence.holder() != ctx.actor() {
                    return Err(OwnerReadError::Refused);
                }
                let entry = self
                    .inventory_entry(roll, ctx, licence)?
                    .ok_or(OwnerReadError::Refused)?;
                return Ok(RenewalRead::Inventory(RenewalInventory {
                    entries: vec![entry],
                    next_after: None,
                }));
            }
            _ => return Err(OwnerReadError::Refused),
        };
        let licence =
            history::read(ctx, &self.families.paid_licences, licence.key()).map_err(integrity)?;
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
        let Birth::House(birth) = proved.birth() else {
            return Err(OwnerReadError::Refused);
        };
        super::super::registry_revocation::current::require(
            ctx,
            &self.families,
            licence.id(),
            term.generation(),
            Some(WorldIcSeconds::from_stamp(&ctx.world().world_ic_now()).map_err(integrity)?),
        )
        .map_err(integrity)?;
        let predecessor_term_end = term.window().term_end();
        if ctx.world().world_ic_now().at < predecessor_term_end.get() {
            return Err(OwnerReadError::Refused);
        }
        let economics =
            history::read(ctx, &self.families.house_economics, term.key()).map_err(integrity)?;
        let binding = RenewalBindingV1::capture(licence, term, economics, birth.issuer())
            .map_err(integrity)?;
        let filed =
            history::read(ctx, &self.families.filed, binding.filed.key()).map_err(integrity)?;
        let filing = super::super::line_update::audit::birth(ctx, &self.families, filed)
            .map_err(integrity)?;
        if filed.content.revision() != binding.revision
            || filed.content.position().charter != binding.charter
            || (filed.format != 3 && filing.filed != binding.filed)
            || filing.charter != binding.charter
        {
            return Err(OwnerReadError::Integrity);
        }
        let sale = self.sale.read(
            roll,
            ctx,
            HouseSaleRequest::listed(filing.licence, birth.issuer()),
        )?;
        if sale.issuer() != binding.issuer
            || sale.filed() != binding.filed
            || sale.revision() != binding.revision
            || sale.charter() != binding.charter
            || sale.line() != binding.line
            || sale.minimum() > binding.terms.class()
        {
            return Err(OwnerReadError::Integrity);
        }
        Ok(RenewalRead::Facts(RenewalFacts {
            binding,
            predecessor_term_end,
            prior_quote: birth.quote(),
            prior_quote_digest: birth.quote_digest(),
            sale,
        }))
    }
}

impl RenewalReader {
    fn operation(
        &self,
        roll: &RollStore,
        ctx: &OwnerReadContext<'_>,
        operation: CommitToken,
    ) -> Result<super::RenewalOperationV1, OwnerReadError> {
        let row = history::read(
            ctx,
            &self.families.house_renewal_operations,
            operation.to_hex(),
        )
        .map_err(integrity)?;
        if row.operation() != operation
            || (ctx.actor() != mush_core::GOD && row.binding().holder() != ctx.actor())
        {
            return Err(OwnerReadError::Refused);
        }
        let e = ctx
            .world()
            .module_state::<EconomyStore>()
            .ok_or(OwnerReadError::Unavailable)?;
        row.verify_history(
            &e.house_renewal_audit()
                .map_err(integrity)?
                .get(operation)
                .map_err(integrity)?,
        )
        .map_err(integrity)?;
        for root in super::audit::required(row).map_err(integrity)? {
            roll.check_required_root(&root).map_err(integrity)?;
        }
        Ok(row.clone())
    }
    fn inventory(
        &self,
        roll: &RollStore,
        ctx: &OwnerReadContext<'_>,
        after: Option<String>,
    ) -> Result<RenewalInventory, OwnerReadError> {
        if after.as_ref().is_some_and(|a| a.len() > 32) {
            return Err(OwnerReadError::Limit);
        }
        let page = ctx.scan_action_state(&self.families.paid_licences, after.as_deref(), 4)?;
        let mut entries = Vec::new();
        for entry in page.entries() {
            let licence = entry.value();
            licence.validate().map_err(integrity)?;
            if entry.encoded_key() != licence.key() {
                return Err(OwnerReadError::Integrity);
            }
            if ctx.actor() != mush_core::GOD && licence.holder() != ctx.actor() {
                continue;
            }
            if let Some(item) = self.inventory_entry(roll, ctx, licence)? {
                entries.push(item);
            }
        }
        Ok(RenewalInventory {
            entries,
            next_after: page.next_after().map(ToString::to_string),
        })
    }
    fn inventory_entry(
        &self,
        roll: &RollStore,
        ctx: &OwnerReadContext<'_>,
        licence: &super::super::paid::PersonalPaidLicence,
    ) -> Result<Option<RenewalInventoryEntry>, OwnerReadError> {
        if !matches!(
            history::resolve(ctx, &self.families, licence.operation()).map_err(integrity)?,
            Birth::House(_)
        ) {
            return Ok(None);
        }
        let term = history::read(
            ctx,
            &self.families.paid_terms,
            format!("{}/{}", licence.id().key(), licence.term().key()),
        )
        .map_err(integrity)?;
        let current = history::current(ctx, &self.families, &self.ratings, licence, term)
            .map_err(integrity)?;
        if !matches!(current.birth(), Birth::House(_)) {
            return Ok(None);
        }
        let head = ctx.action_state(&self.families.house_renewal_heads, &licence.id().key())?;
        let (last_operation, attempted) = if let Some(head) = head {
            head.validate().map_err(integrity)?;
            let last = self.operation(roll, ctx, head.last_operation())?;
            if head != &super::RenewalHeadV1::for_operation(&last).map_err(integrity)?
                || head.current() != licence.term()
                || !last.binding().matches_birth(licence)
            {
                return Err(OwnerReadError::Integrity);
            }
            (
                Some(last.operation()),
                head.attempted_predecessor == licence.term(),
            )
        } else {
            (None, false)
        };
        Ok(Some(RenewalInventoryEntry {
            licence: licence.id(),
            holder: licence.holder(),
            generation: licence.term(),
            term_end: term.window().term_end(),
            last_operation,
            attempted,
        }))
    }
}
