//! Doc 46 CT.0: the personal renewal's committed snapshot
//! (`tests/ct0/renewal_personal.snap`). The walk is shared with the House twin
//! (`ct0_rig::ct0_renewal_walk`); this module supplies the personal adapters.
use super::*;
use crate::commerce::families::CommerceFamilies;
use crate::commerce::records::PublicationError;
use mush_economy::AccountId;
use mush_economy::managed_entitlement::{GrossRecipientCut, Recipient};
use mush_org::PrincipalId;

crate::commerce::tests::ct0_rig::ct0_renewal_walk! {
    test: ct0_personal_renewal_snapshot,
    title: "personal renewal",
    file: "tests/ct0/renewal_personal.snap",
    economy: managed_renewal,
    operations: renewal_operations,
    heads: renewal_heads,
    economics: term_economics,
}

/// The Economy's offer for the row's binding, every optional role filled.
fn ct0_offer(row: &RenewalOperationV1) -> Ct0Result<econ::RenewalOffer> {
    let b = row.binding();
    let template = b.grant().renewal();
    let seller = ObjId(6);
    Ok(econ::RenewalOffer {
        format: 1,
        basis: econ::RenewalBasisV1 {
            format: 1,
            licence: b.licence().get(),
            predecessor_generation: b.predecessor().get(),
            next_generation: b.next().get(),
            predecessor_operation: b.predecessor_operation(),
            prior_quote: token(3),
            prior_quote_digest: digest(4),
            product_digest: b.product_digest(),
            template_digest: b.template_digest()?,
            grant_economics_digest: b.grant().digest()?,
            royalty_chain_digest: b.grant().royalty_chain().digest(),
        },
        terms: econ::RenewalOfferTermsV1 {
            format: 1,
            lineage: econ::RenewalLineageV1::Original,
            buyer: b.holder(),
            seller,
            charter: ObjId(b.charter().get()),
            title_revision: 1,
            payer: AccountId(10),
            instrument: template.instrument().clone(),
            price: template.price(),
            registry: GrossRecipientCut {
                permille: 100,
                recipient: Some(Recipient {
                    account: AccountId(2),
                    owner: PrincipalId::Object(ObjId(3)),
                }),
            },
            venue: Some(GrossRecipientCut {
                permille: 50,
                recipient: Some(Recipient {
                    account: AccountId(4),
                    owner: PrincipalId::Object(ObjId(8)),
                }),
            }),
            parents: Vec::new(),
            remainder: Recipient {
                account: AccountId(1),
                owner: PrincipalId::Object(seller),
            },
            not_before: stamp(row.created_at_utc),
            expires_at: stamp(row.created_at_utc + 100),
        },
    })
}

fn ct0_prepare(
    action: &DefinitionRef,
    commit: CommitToken,
    publication: roots::Publication,
    before: Option<&RenewalOperationV1>,
    after: &RenewalOperationV1,
) -> Result<roots::RenewalRoots, PublicationError> {
    roots::RenewalRoots::prepare(action, commit, publication, before, after, None, None)
}

/// The birth the walk renews: its three rows, the filed royalty chain its grant
/// names, and every digest over them.
fn ct0_birth(s: &mut Snapshot, f: &CommerceFamilies, b: &Birth) -> Ct0Result {
    rig::document(s, "birth licence", &f.paid_licences, &b.licence)?;
    rig::document(s, "birth term", &f.paid_terms, &b.term)?;
    rig::document(s, "birth economics", &f.term_economics, &b.economics)?;
    s.wire("birth economics digest", &b.economics.digest()?)?;
    s.wire("birth grant digest", &b.economics.grant().digest()?)?;
    s.wire("birth royalty chain", &b.economics.grant().royalty_chain())?;
    let chain =
        FiledRoyaltyTermsV1::capture(FiledRevisionId::new(3)?, identity()?, token(2), Vec::new())?;
    rig::document(s, "filed royalty terms", &f.filed_royalties, &chain)?;
    s.wire("filed royalty digest", &chain.digest()?)?;
    check_eq!(chain.reference()?, b.economics.grant().royalty_chain());
    Ok(())
}

fn ct0_verify(
    licence: &PersonalPaidLicence,
    head: Option<&RenewalHeadV1>,
    rows: &[&RenewalOperationV1],
    terms: &BTreeMap<TermGeneration, &PaidTermRecord>,
    economics: &BTreeMap<TermGeneration, &PaidTermEconomicsV1>,
) -> Result<(), PublicationError> {
    chain::verify(licence, head, rows, terms, economics)
}
