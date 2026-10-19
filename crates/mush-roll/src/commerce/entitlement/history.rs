//! Actual immutable birth routing. Missing or ambiguous old/new allocations
//! never become a legacy fallback, including refunded and interrupted work.
//! This module performs no current issuer/ancestor eligibility decision.

use super::super::{
    WorldIcSeconds,
    families::CommerceFamilies,
    paid::{PaidTermRecord, PersonalPaidLicence},
    purchase::{PaidAllocation, PurchaseParticipants, PurchasePhase},
    rating::RatingFamilies,
    read::CommerceRead,
    records::{PublicationError, Record, RevisionPolicy},
};
use super::{EntitlementAllocationV1, PaidTermEconomicsV1};
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKey, DefinitionKind, DefinitionRegistryBuilder,
    OwnerId,
};
use mush_core::{CommitToken, DefinitionDigest, DefinitionRef, StateFamilyHandle};
use mush_economy::{
    EconomyStore,
    managed_entitlement::{EntitlementProtocol, EntitlementPurchase},
    managed_purchase::PurchaseHistory,
};

fn invalid(_: impl std::fmt::Display) -> PublicationError {
    PublicationError::Mismatch
}

#[derive(Clone, Copy)]
pub(crate) enum Birth<'a> {
    Legacy(&'a PaidAllocation),
    Entitlement(&'a EntitlementAllocationV1),
    House(&'a super::super::house_purchase::HouseAllocationV1),
}
impl Birth<'_> {
    pub(crate) fn operation(&self) -> CommitToken {
        match self {
            Self::Legacy(a) => a.operation(),
            Self::Entitlement(a) => a.operation(),
            Self::House(a) => a.operation(),
        }
    }
    pub(crate) fn rating(&self) -> super::super::rating::RatingRef {
        match self {
            Self::Legacy(a) => a.rating(),
            Self::Entitlement(a) => a.rating(),
            Self::House(a) => a.rating(),
        }
    }
    pub(crate) fn phase(&self) -> PurchasePhase {
        match self {
            Self::Legacy(a) => a.phase(),
            Self::Entitlement(a) => a.phase(),
            Self::House(a) => a.phase(),
        }
    }
    pub(crate) fn licence(&self) -> super::super::LicenceId {
        match self {
            Self::Legacy(a) => a.licence(),
            Self::Entitlement(a) => a.licence(),
            Self::House(a) => a.licence(),
        }
    }
    pub(crate) fn term(&self) -> super::super::TermGeneration {
        match self {
            Self::Legacy(a) => a.term(),
            Self::Entitlement(a) => a.term(),
            Self::House(a) => a.term(),
        }
    }
    pub(crate) fn holder(&self) -> mush_core::ObjId {
        match self {
            Self::Legacy(a) => a.holder(),
            Self::Entitlement(a) => a.holder(),
            Self::House(a) => a.holder(),
        }
    }
    pub(crate) fn charter(&self) -> super::super::CharterId {
        match self {
            Self::Legacy(a) => a.charter(),
            Self::Entitlement(a) => a.charter(),
            Self::House(a) => a.charter(),
        }
    }
    pub(crate) fn line(&self) -> super::super::LineId {
        match self {
            Self::Legacy(a) => a.line(),
            Self::Entitlement(a) => a.line(),
            Self::House(a) => a.line(),
        }
    }
    pub(crate) fn filed(&self) -> super::super::FiledRevisionId {
        match self {
            Self::Legacy(a) => a.filed(),
            Self::Entitlement(a) => a.filed(),
            Self::House(a) => a.filed(),
        }
    }
    pub(crate) fn revision(&self) -> super::super::RevisionIdentity {
        match self {
            Self::Legacy(a) => a.revision(),
            Self::Entitlement(a) => a.revision(),
            Self::House(a) => a.revision(),
        }
    }
    pub(crate) fn terms(&self) -> super::super::paid::PaidTerms {
        match self {
            Self::Legacy(a) => a.terms(),
            Self::Entitlement(a) => a.terms(),
            Self::House(a) => a.terms(),
        }
    }
    pub(crate) fn product_digest(&self) -> DefinitionDigest {
        match self {
            Self::Legacy(a) => a.product_digest(),
            Self::Entitlement(a) => a.product_digest(),
            Self::House(a) => a.product_digest(),
        }
    }
}

pub(crate) fn read<'a, R: Record>(
    ctx: &'a impl CommerceRead,
    handle: &StateFamilyHandle<String, R>,
    key: String,
) -> Result<&'a R, PublicationError> {
    optional(ctx, handle, key)?.ok_or(PublicationError::Missing)
}
pub(crate) fn optional<'a, R: Record>(
    ctx: &'a impl CommerceRead,
    handle: &StateFamilyHandle<String, R>,
    key: String,
) -> Result<Option<&'a R>, PublicationError> {
    let row = ctx
        .get(handle, &key)
        .map_err(|_| PublicationError::Unavailable)?;
    if let Some(row) = row {
        row.validate()?;
        if row.key() != key {
            return Err(PublicationError::Mismatch);
        }
    }
    Ok(row)
}
pub(crate) fn resolve<'a>(
    ctx: &'a impl CommerceRead,
    f: &CommerceFamilies,
    operation: CommitToken,
) -> Result<Birth<'a>, PublicationError> {
    let legacy = optional(ctx, &f.paid_allocations, operation.to_hex())?;
    let entitlement = optional(ctx, &f.entitlement_allocations, operation.to_hex())?;
    let house = optional(ctx, &f.house_allocations, operation.to_hex())?;
    match (legacy, entitlement, house) {
        (Some(row), None, None) => Ok(Birth::Legacy(row)),
        (None, Some(row), None) => Ok(Birth::Entitlement(row)),
        (None, None, Some(row)) => Ok(Birth::House(row)),
        (None, None, None) => Err(PublicationError::Missing),
        _ => Err(PublicationError::Mismatch),
    }
}

/// Exact historical fixed action; this does not register or grant that action.
fn funding_ref(
    protocol: EntitlementProtocol,
    compatibility: DefinitionDigest,
    p: &PurchaseParticipants,
) -> Result<DefinitionRef, PublicationError> {
    let id = match protocol {
        EntitlementProtocol::OriginalV3 => "purchase_original_mind",
        EntitlementProtocol::DerivedV1 => "purchase_derived_mind",
    };
    exact_fund(id, protocol.version(), protocol.name(), compatibility, p)
}
fn legacy_funding_ref(
    format: u16,
    compatibility: DefinitionDigest,
    p: &PurchaseParticipants,
) -> Result<DefinitionRef, PublicationError> {
    let protocol = match format {
        1 => "original_mind_purchase_v1",
        2 => "original_mind_purchase_v2",
        _ => return Err(PublicationError::Invalid),
    };
    exact_fund(
        "purchase_original_mind",
        u32::from(format),
        protocol,
        compatibility,
        p,
    )
}
fn exact_fund(
    id: &str,
    version: u32,
    protocol: &str,
    compatibility: DefinitionDigest,
    p: &PurchaseParticipants,
) -> Result<DefinitionRef, PublicationError> {
    p.validate()?;
    let manifest = DefinitionRef::new(
        DefinitionKey {
            owner: OwnerId::new("tacsim").map_err(invalid)?,
            kind: DefinitionKind::new("registry_manifest").map_err(invalid)?,
            id: DefinitionId::new("combat").map_err(invalid)?,
        },
        1,
        compatibility,
    )
    .map_err(invalid)?;
    let descriptor = DefinitionDescriptor::new(
        DefinitionKind::new("action").map_err(invalid)?,
        DefinitionId::new(id).map_err(invalid)?,
        version,
        serde_json::json!({"format":version,"protocol":protocol,"transition":"fund"}),
    )
    .map_err(invalid)?
    .with_references(vec![manifest, p.economy().clone(), p.retention().clone()]);
    DefinitionRegistryBuilder::new()
        .register(OwnerId::new("market").map_err(invalid)?, descriptor)
        .map_err(invalid)
}

/// Historical facts from the actual old E4 recipe, with no reinterpretation as
/// new economics. Existing original records and their bytes stay unchanged.
pub(crate) fn verify_legacy_history(
    row: &PaidAllocation,
    actual: &PurchaseHistory,
    compatibility: DefinitionDigest,
) -> Result<(), PublicationError> {
    row.validate()?;
    let economic = actual
        .original()
        .map_err(invalid)?
        .ok_or(PublicationError::Mismatch)?;
    if actual.operation() != row.operation
        || actual.funding() != &row.funding
        || actual.effect() != &legacy_funding_ref(row.format, compatibility, &row.participants)?
        || economic.quote != row.quote
        || economic.quote_digest != row.quote_digest
        || economic.product_digest != row.product_digest
        || economic.buyer != row.holder
        || economic.charter != mush_core::ObjId(row.charter.get())
        || !actual.matches_purchase(&economic, &row.funding)
    {
        return Err(PublicationError::Mismatch);
    }
    let phase = match actual.terminal() {
        None => PurchasePhase::Funded {},
        Some(t) if t.granted() => PurchasePhase::Granted { commit: t.commit() },
        Some(t) => PurchasePhase::Refunded { commit: t.commit() },
    };
    if row.phase != phase {
        return Err(PublicationError::Mismatch);
    }
    Ok(())
}

pub(crate) fn verify_entitlement_history(
    row: &EntitlementAllocationV1,
    actual: &PurchaseHistory,
    compatibility: DefinitionDigest,
) -> Result<EntitlementPurchase, PublicationError> {
    row.validate()?;
    let economic = actual
        .entitlement()
        .map_err(invalid)?
        .ok_or(PublicationError::Mismatch)?;
    if actual.operation() != row.operation
        || actual.funding() != &row.funding
        || actual.effect() != &funding_ref(row.protocol, compatibility, &row.participants)?
        || economic.protocol != row.protocol
        || economic.quote != row.quote
        || economic.quote_digest != row.quote_digest
        || economic.product_digest != row.product_digest
        || economic.buyer != row.holder
        || economic.charter != mush_core::ObjId(row.charter.get())
        || economic.grant_economics_digest != row.economics.grant().digest()?
        || economic.royalty_chain_digest != row.economics.grant().royalty_chain().digest()
        || !actual.matches_entitlement(&economic, &row.funding)
    {
        return Err(PublicationError::Mismatch);
    }
    let phase = match actual.terminal() {
        None => PurchasePhase::Funded {},
        Some(t) if t.granted() => PurchasePhase::Granted { commit: t.commit() },
        Some(t) => PurchasePhase::Refunded { commit: t.commit() },
    };
    if row.phase != phase {
        return Err(PublicationError::Mismatch);
    }
    Ok(economic)
}

pub(crate) fn verify_entitlement_grant(
    row: &EntitlementAllocationV1,
    licence: &PersonalPaidLicence,
    term: &PaidTermRecord,
    economics: &PaidTermEconomicsV1,
    actual: &PurchaseHistory,
    compatibility: DefinitionDigest,
) -> Result<(), PublicationError> {
    verify_entitlement_history(row, actual, compatibility)?;
    if !matches!(row.phase, PurchasePhase::Granted { .. }) {
        return Err(PublicationError::Mismatch);
    }
    let issued = WorldIcSeconds::from_stamp(
        actual
            .terminal()
            .and_then(|t| t.issued_at_ic())
            .ok_or(PublicationError::Missing)?,
    )
    .map_err(invalid)?;
    let expected = PersonalPaidLicence::original(
        row.licence,
        row.charter,
        row.line,
        row.holder,
        licence.term(),
        row.operation,
    )?;
    if licence != &expected
        || economics != &row.economics
        || term.licence() != row.licence
        || term.generation() != row.term
        || term.operation() != row.operation
        || term.policy()
            != (RevisionPolicy::Pin {
                filed: row.filed,
                revision: row.revision,
            })
        || term.terms() != row.terms
        || term.window().issued_at() != issued
        || term.window().eligibility_start() != issued
    {
        return Err(PublicationError::Mismatch);
    }
    Ok(())
}

/// A proved immutable generation, distinct from its licence's current pointer.
/// The birth remains the permanent acquisition/product identity; operation and
/// terminal below name this exact issued generation, including renewal.
#[derive(Clone, Copy)]
pub(crate) struct Granted<'a> {
    birth: Birth<'a>,
    accepted_holder: mush_core::ObjId,
    term: &'a PaidTermRecord,
    economics: Option<&'a PaidTermEconomicsV1>,
    terminal: CommitToken,
}
impl<'a> Granted<'a> {
    pub(crate) const fn birth(self) -> Birth<'a> {
        self.birth
    }
    pub(crate) fn operation(self) -> CommitToken {
        self.term.operation()
    }
    pub(crate) fn birth_operation(self) -> CommitToken {
        self.birth.operation()
    }
    pub(crate) fn terminal(self) -> CommitToken {
        self.terminal
    }
    pub(crate) fn term(self) -> super::super::TermGeneration {
        self.term.generation()
    }
    pub(crate) fn holder(self) -> mush_core::ObjId {
        self.accepted_holder
    }
    pub(crate) fn licence(self) -> super::super::LicenceId {
        self.birth.licence()
    }
    pub(crate) fn charter(self) -> super::super::CharterId {
        self.birth.charter()
    }
    pub(crate) fn line(self) -> super::super::LineId {
        self.birth.line()
    }
    pub(crate) fn filed(self) -> super::super::FiledRevisionId {
        self.birth.filed()
    }
    pub(crate) fn revision(self) -> super::super::RevisionIdentity {
        self.birth.revision()
    }
    pub(crate) fn rating(self) -> super::super::rating::RatingRef {
        self.birth.rating()
    }
    pub(crate) fn product_digest(self) -> DefinitionDigest {
        self.birth.product_digest()
    }
    pub(crate) fn economics(self) -> Option<&'a PaidTermEconomicsV1> {
        self.economics
    }
}

/// Actual birth/E4 and immutable receipt identity, without live Registry or
/// issuer eligibility. The supplied term is historical and need not be current.
/// Full boot separately proves every generation and linked attempt/head.
pub(crate) fn granted<'a>(
    ctx: &'a impl CommerceRead,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    licence: &PersonalPaidLicence,
    term: &'a PaidTermRecord,
) -> Result<Granted<'a>, PublicationError> {
    let economy = ctx
        .world()
        .module_state::<EconomyStore>()
        .ok_or(PublicationError::Unavailable)?;
    if economy.is_frozen() {
        return Err(PublicationError::Unavailable);
    }
    let audit = economy.purchase_receipt_audit();
    audit.require_complete().map_err(invalid)?;
    let actual = audit.get(licence.operation()).map_err(invalid)?;
    let birth = resolve(ctx, f, licence.operation())?;
    licence.validate()?;
    if birth.licence() != licence.id()
        || birth.charter() != licence.charter()
        || birth.line() != licence.line()
    {
        return Err(PublicationError::Mismatch);
    }
    // Immutable birth is always checked against its original holder bytes.
    // Current holder authority is a distinct complete transfer-chain proof.
    let birth_pointer = PersonalPaidLicence::original(
        birth.licence(),
        birth.charter(),
        birth.line(),
        birth.holder(),
        licence.term(),
        birth.operation(),
    )?;
    let rating = birth.rating();
    let receipt = ctx
        .get(&ratings.receipts, &rating.job())
        .map_err(|_| PublicationError::Unavailable)?
        .ok_or(PublicationError::Missing)?;
    receipt.validate().map_err(invalid)?;
    if receipt.reference().map_err(invalid)? != rating
        || receipt.subject().revision() != birth.revision()
        || birth.terms().class() < receipt.minimum()
        || term.licence() != licence.id()
    {
        return Err(PublicationError::Mismatch);
    }
    let birth_key = format!("{}/{}", licence.id().key(), birth.term().key());
    let birth_term = if term.generation() == birth.term() {
        term
    } else {
        read(ctx, &f.paid_terms, birth_key.clone())?
    };
    let birth_economics = optional(ctx, &f.term_economics, birth_key)?;
    if !matches!(birth, Birth::House(_))
        && optional(ctx, &f.house_economics, birth_term.key())?.is_some()
    {
        return Err(PublicationError::Mismatch);
    }
    match birth {
        Birth::Legacy(row) => {
            if birth_economics.is_some()
                || actual.entitlement().map_err(invalid)?.is_some()
                || term.generation() != row.term
                || licence.term() != row.term
            {
                return Err(PublicationError::Mismatch);
            }
            verify_legacy_history(row, actual, receipt.compatibility().digest())?;
            super::super::paid_audit::verify_granted(row, &birth_pointer, birth_term, actual)?;
        }
        Birth::House(row) => {
            if birth_economics.is_some() || optional(ctx, &f.term_economics, term.key())?.is_some()
            {
                return Err(PublicationError::Mismatch);
            }
            let economics = read(ctx, &f.house_economics, birth_term.key())?;
            super::super::house_purchase::history::verify_grant(
                row,
                &birth_pointer,
                birth_term,
                economics,
                actual,
                receipt.compatibility().digest(),
            )?;
        }
        Birth::Entitlement(row) => verify_entitlement_grant(
            row,
            &birth_pointer,
            birth_term,
            birth_economics.ok_or(PublicationError::Missing)?,
            actual,
            receipt.compatibility().digest(),
        )?,
    }
    let (economics, terminal, accepted_holder) = if term.generation() == birth.term() {
        if term.operation() != birth.operation() {
            return Err(PublicationError::Mismatch);
        }
        (
            birth_economics,
            actual.terminal().ok_or(PublicationError::Missing)?.commit(),
            birth.holder(),
        )
    } else if let Birth::House(birth_row) = birth {
        let row = read(ctx, &f.house_renewal_operations, term.operation().to_hex())?;
        let accepted_holder = row.binding().holder();
        let accepted_pointer = row.binding().accepted_pointer(term.generation())?;
        (
            None,
            super::super::house_renewal::history::granted_generation(
                ctx,
                f,
                &accepted_pointer,
                term,
                birth_row,
                economy,
            )?,
            accepted_holder,
        )
    } else {
        let Birth::Entitlement(birth_row) = birth else {
            return Err(PublicationError::Mismatch);
        };
        let row = read(ctx, &f.renewal_operations, term.operation().to_hex())?;
        let economics = read(ctx, &f.term_economics, term.key())?;
        let view = economy
            .renewal_audit()
            .map_err(invalid)?
            .get(row.operation())
            .map_err(invalid)?;
        super::super::renewal::history::verify(row, &view)?;
        let (_, expected_term, expected_economics) = row.granted_rows()?;
        let b = row.binding();
        let accepted_pointer = row.binding().accepted_pointer(term.generation())?;
        if !b.matches_birth(&accepted_pointer)
            || term != &expected_term
            || economics != &expected_economics
            || b.product_digest() != birth_row.product_digest
            || b.grant() != birth_row.economics.grant()
            || view.offer().basis.prior_quote != birth_row.quote
            || view.offer().basis.prior_quote_digest != birth_row.quote_digest
        {
            return Err(PublicationError::Mismatch);
        }
        let prior = read(
            ctx,
            &f.paid_terms,
            format!("{}/{}", licence.id().key(), b.predecessor().key()),
        )?;
        let prior_economics = read(ctx, &f.term_economics, prior.key())?;
        b.check_predecessor(&accepted_pointer, prior, prior_economics)?;
        (
            Some(economics),
            row.state()
                .terminal_commit()
                .ok_or(PublicationError::Missing)?,
            b.holder(),
        )
    };
    Ok(Granted {
        birth,
        accepted_holder,
        term,
        economics,
        terminal,
    })
}

/// Fresh customer use/source checks current pointer only after proving its own
/// actual grant. A pending renewal leaves this predecessor usable unchanged.
pub(crate) fn current<'a>(
    ctx: &'a impl CommerceRead,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    licence: &PersonalPaidLicence,
    term: &'a PaidTermRecord,
) -> Result<Granted<'a>, PublicationError> {
    term.matches_current(licence)?;
    let grant = granted(ctx, f, ratings, licence, term)?;
    if let Some(ownership) = licence.ownership() {
        if super::super::paid_resale::history::historical(ctx, f, ratings, licence, ownership)?
            != licence.holder()
        {
            return Err(PublicationError::Mismatch);
        }
    } else if licence.holder() != grant.birth().holder() {
        return Err(PublicationError::Mismatch);
    }
    let personal_head = optional(ctx, &f.renewal_heads, licence.id().key())?;
    let house_head = optional(ctx, &f.house_renewal_heads, licence.id().key())?;
    if let Birth::House(birth) = grant.birth() {
        if personal_head.is_some() {
            return Err(PublicationError::Mismatch);
        }
        match house_head {
            None if licence.term().get() == 1 => (),
            Some(head) => {
                let row = read(
                    ctx,
                    &f.house_renewal_operations,
                    head.last_operation().to_hex(),
                )?;
                let historical_pointer = row.binding().accepted_pointer(licence.term())?;
                if head != &super::super::house_renewal::RenewalHeadV1::for_operation(row)?
                    || !row.binding().matches_birth(&historical_pointer)
                    || (row.state().pending() && !row.binding().matches_birth(licence))
                    || row.binding().issuer() != birth.issuer()
                    || head.current() != licence.term()
                {
                    return Err(PublicationError::Mismatch);
                }
                let economy = ctx
                    .world()
                    .module_state::<EconomyStore>()
                    .ok_or(PublicationError::Unavailable)?;
                let view = economy
                    .house_renewal_audit()
                    .map_err(invalid)?
                    .get(row.operation())
                    .map_err(invalid)?;
                super::super::house_renewal::history::verify(row, &view)?;
            }
            _ => return Err(PublicationError::Missing),
        }
        return Ok(grant);
    }
    if house_head.is_some() {
        return Err(PublicationError::Mismatch);
    }
    match personal_head {
        None if licence.term().get() == 1 => {}
        Some(head) => {
            let row = read(ctx, &f.renewal_operations, head.last_operation().to_hex())?;
            let historical_pointer = row.binding().accepted_pointer(licence.term())?;
            if head != &super::super::renewal::RenewalHeadV1::for_operation(row)?
                || !row.binding().matches_birth(&historical_pointer)
                || (row.state().pending() && !row.binding().matches_birth(licence))
                || head.current() != licence.term()
            {
                return Err(PublicationError::Mismatch);
            }
            let economy = ctx
                .world()
                .module_state::<EconomyStore>()
                .ok_or(PublicationError::Unavailable)?;
            let view = economy
                .renewal_audit()
                .map_err(invalid)?
                .get(row.operation())
                .map_err(invalid)?;
            super::super::renewal::history::verify(row, &view)?;
        }
        _ => return Err(PublicationError::Missing),
    }
    Ok(grant)
}
