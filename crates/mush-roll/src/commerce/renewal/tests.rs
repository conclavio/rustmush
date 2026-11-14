//! Structural history tests. These do not fabricate an accepted E4 grant;
//! actual owner/output/audit integration must also pass at the paired boundary.
//!
//! Doc 46 CT.5: the scenarios are the twin's (`renewal_twin::tests`), run here for the
//! personal issuer under their old names; this module holds the personal rig, the one
//! scenario only the personal issuer has, and the CT.0 walk.
use super::super::renewal_twin::tests::{self as twin, Rig};
use super::super::{
    FiledRevisionId, Holder, LicenceId, RevisionIdentity, TermGeneration,
    entitlement::{
        DownstreamTermsV1, FiledRoyaltyTermsV1, GrantEconomicsV1, PaidTermEconomicsV1,
        RenewalTemplateV1,
    },
    paid::{PaidForm, PaidTermRecord, PaidTerms, PersonalPaidLicence},
    rating::RatingClass,
    records::{PublicationError, Record, RevisionPolicy},
    renewal_twin::Personal,
};
use super::records::*;
use super::*;
use mush_core::{ClockRef, CommitToken, DefinitionDigest, DefinitionRef, ObjId};
use mush_economy::{Amount, InstrumentId};
#[allow(unused_imports)]
use std::collections::BTreeMap;
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
use mush_testkit::fixtures::digest;
#[allow(unused_imports)]
use mush_testkit::fixtures::stamp;
use mush_testkit::fixtures::token;

impl Rig for Personal {
    fn birth_terms() -> TestResult<PaidTerms> {
        Ok(PaidTerms::original_personal_form(
            RatingClass::I,
            PaidForm::OpenResale {},
            false,
        )?)
    }
    fn grant(terms: PaidTerms, policy: RevisionPolicy) -> TestResult<GrantEconomicsV1> {
        let chain = FiledRoyaltyTermsV1::capture(
            FiledRevisionId::new(3)?,
            twin::identity()?,
            token(2),
            Vec::new(),
        )?;
        Ok(GrantEconomicsV1::new(
            DownstreamTermsV1::Source {
                royalty_permille: 73,
            },
            RenewalTemplateV1::new(
                InstrumentId::new("cr").ok_or("instrument")?,
                Amount::from_minor(500),
                100,
                20,
                terms,
                policy,
                twin::definition("entitlement_policy", "personal")?,
            )?,
            chain.reference()?,
        )?)
    }
    fn widen(old: &GrantEconomicsV1, instrument: InstrumentId) -> TestResult<GrantEconomicsV1> {
        let t = old.renewal();
        let template = RenewalTemplateV1::new(
            instrument,
            t.price(),
            t.term_seconds(),
            t.grace_seconds(),
            t.terms(),
            t.revision(),
            t.policy().clone(),
        )?;
        Ok(GrantEconomicsV1::new(
            old.downstream(),
            template,
            old.royalty_chain(),
        )?)
    }
    fn capture(
        licence: &PersonalPaidLicence,
        term: &PaidTermRecord,
        economics: &PaidTermEconomicsV1,
    ) -> Result<RenewalBindingV1, PublicationError> {
        RenewalBindingV1::capture(licence, term, economics)
    }
    fn birth_line() -> TestResult<()> {
        Ok(())
    }
    fn other_line() -> TestResult<Option<()>> {
        Ok(None)
    }
    fn economics_row(
        licence: LicenceId,
        generation: TermGeneration,
        operation: CommitToken,
        product: DefinitionDigest,
        grant: GrantEconomicsV1,
    ) -> Result<PaidTermEconomicsV1, PublicationError> {
        PaidTermEconomicsV1::capture(licence, generation, operation, product, grant)
    }
    fn product(economics: &PaidTermEconomicsV1) -> DefinitionDigest {
        economics.product_digest()
    }
    fn grant_of(economics: &PaidTermEconomicsV1) -> &GrantEconomicsV1 {
        economics.grant()
    }
    fn with_holder(row: &RenewalOperationV1, holder: Holder) -> RenewalOperationV1 {
        let mut row = row.clone();
        row.binding.holder = holder;
        row
    }
    fn with_attempt(row: &RenewalOperationV1, attempt: RenewalAttemptV1) -> RenewalOperationV1 {
        let mut row = row.clone();
        row.attempt = attempt;
        row
    }
    fn with_offer_digest(row: &RenewalOperationV1, digest: DefinitionDigest) -> RenewalOperationV1 {
        let mut row = row.clone();
        row.offer_digest = digest;
        row
    }
    fn with_holder_basis(
        row: &RenewalOperationV1,
        holder: Holder,
        basis: mush_economy::managed_paid_resale::RenewalHolderBasisV1,
    ) -> RenewalOperationV1 {
        let mut row = row.clone();
        row.binding.format = 2;
        row.binding.holder = holder;
        row.binding.holder_basis = Some(basis);
        row
    }
    fn decode(bytes: &[u8]) -> Result<RenewalOperationV1, PublicationError> {
        RenewalOperationV1::decode(bytes)
    }
}

/// 17g C2g: `pub(crate)` with the helpers below, for Roll's command-read
/// tests (the named buyer's resale step on a granted licence).
pub(crate) type Birth = twin::Birth<Personal>;
#[allow(dead_code)]
fn definition(kind: &str, id: &str) -> TestResult<DefinitionRef> {
    twin::definition(kind, id)
}
#[allow(dead_code)]
fn identity() -> TestResult<RevisionIdentity> {
    twin::identity()
}
fn birth() -> TestResult<Birth> {
    twin::birth::<Personal>()
}
#[allow(dead_code)]
pub(crate) fn birth_with(terms: PaidTerms) -> TestResult<Birth> {
    twin::birth_with::<Personal>(terms)
}
pub(crate) fn draft(
    b: &Birth,
    n: u8,
    at: i64,
    attempt: RenewalAttemptV1,
) -> TestResult<RenewalOperationV1> {
    twin::draft::<Personal>(b, n, at, attempt)
}
#[allow(dead_code)]
pub(crate) fn funded(row: &RenewalOperationV1) -> TestResult<RenewalOperationV1> {
    twin::funded::<Personal>(row)
}
#[allow(dead_code)]
pub(crate) fn terminal(row: &RenewalOperationV1, grant: bool) -> TestResult<RenewalOperationV1> {
    twin::terminal::<Personal>(row, grant)
}
pub(crate) fn granted_birth(row: &RenewalOperationV1) -> TestResult<Birth> {
    twin::granted_birth::<Personal>(row)
}

super::super::renewal_twin::tests::instantiate!(crate::commerce::renewal_twin::Personal);

/// 17g C2b (D-UM-40, C2 plan §2.5; test 12): a licence granted from a listing
/// the author marked `transferable: false` carries non-transferable terms
/// (the entitlement writer's own `personal_terms`). Both of the paid-resale
/// reader's gates refuse it: `observe`'s own check (`require_transferable` on
/// the current term) and the resale binding it then builds
/// (`PaidResaleBinding::validate`), so dropping either one still leaves this
/// test red for the other. A renewal carries the predecessor's terms, so the
/// renewed generation is refused the same way; a renewal binding that widens
/// them refuses. The unmarked control passes both gates before and after its
/// renewal, so the refusals are the mark's and nothing else's. The marked
/// listing's own path into these terms (List → product → quote → grant terms)
/// is mush-market's `the_transfer_mark_narrows_and_never_widens`; a full
/// `observe` over a granted licence needs the real-engine resale harness.
#[test]
fn the_marked_licence_cannot_be_offered_for_resale() -> TestResult {
    use crate::commerce::entitlement::writer::EntitlementPurchaseTerms;
    use crate::commerce::paid_resale::PaidResaleBinding;
    use crate::commerce::paid_resale::reader::require_transferable;
    use crate::commerce::records::PublicationError;
    use mush_economy::managed_paid_resale::{PaidBirthRefV1, PaidBirthRouteV1, PaidHolderRefV1};
    // The binding `observe` builds over the current term, at its first holder.
    let binding = |b: &Birth, term: &PaidTermRecord| PaidResaleBinding {
        licence: term.licence(),
        charter: b.licence.charter(),
        line: b.licence.line(),
        birth: PaidBirthRefV1 {
            route: PaidBirthRouteV1::Entitlement,
            operation: token(1),
            digest: digest(9),
        },
        ownership: PaidHolderRefV1 {
            generation: 1,
            operation: token(1),
            digest: digest(9),
        },
        term: term.clone(),
        term_clock: ClockRef::WorldIc { generation: 1 },
        economics_digest: digest(9),
        royalty_chain_digest: digest(8),
    };
    for mark in [false, true] {
        let terms = EntitlementPurchaseTerms::personal_terms(
            RatingClass::I,
            PaidForm::OpenResale {},
            mark,
        )?;
        check_eq!(terms.transferable(), mark);
        let b = birth_with(terms)?;
        check_eq!(b.economics.grant().renewal().terms(), terms);
        let renewed = granted_birth(&terminal(
            &funded(&draft(&b, 10, 20, RenewalAttemptV1::Automatic {})?)?,
            true,
        )?)?;
        check_eq!(renewed.term.generation().get(), 2);
        check_eq!(renewed.term.terms(), terms);
        for term in [&b.term, &renewed.term] {
            if mark {
                require_transferable(term)?;
                binding(&b, term).validate()?;
            } else {
                check!(matches!(
                    require_transferable(term),
                    Err(PublicationError::Unavailable)
                ));
                check!(matches!(
                    binding(&b, term).validate(),
                    Err(PublicationError::Mismatch)
                ));
            }
        }
        if !mark {
            // The renewal binding cannot widen the frozen terms: a predecessor
            // whose terms were flipped no longer matches its own economics.
            let widened = PaidTermRecord::original(
                b.term.licence(),
                b.term.generation(),
                b.term.window(),
                PaidTerms::original_personal_form(RatingClass::I, PaidForm::OpenResale {}, true)?,
                b.term.policy(),
                b.term.operation(),
            )?;
            check!(RenewalBindingV1::capture(&b.licence, &widened, &b.economics).is_err());
        }
    }
    Ok(())
}

mod ct0_snapshot;
