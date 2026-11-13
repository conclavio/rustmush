//! Doc 46 CT.0: the purchase twins' committed snapshots
//! (`tests/ct0/purchase_personal.snap`, `tests/ct0/purchase_house.snap`).
//!
//! One walk, written once (`purchase_walk!`) and instantiated for each twin, over
//! every protocol the twin has: the registered descriptors, then the allocation
//! Funded, Granted and Refunded (the `PurchasePhase` ends), with every row the
//! Fund and the Grant write, the roots they require, and the cold read-back.
//!
//! Not reached: the paired writers themselves (`fund`, `terminal`), which need the
//! Economy owner's opaque outputs, and the roots they prepare (the reader's
//! `EntitlementSaleFacts` / `HouseSaleFacts`). The granted term's window is built
//! here as the writer's private `window` builds it (issue, issue + term,
//! + grace), so the term row's bytes are pinned but not the writer's arithmetic.
use super::ct0_rig::{self as rig, TestResult};
use crate::commerce::families::CommerceFamilies;
use crate::commerce::paid::{PaidForm, PaidTermRecord, PaidTerms, PersonalPaidLicence};
use crate::commerce::paid_holdings::PaidHolding;
use crate::commerce::purchase::PurchasePhase;
use crate::commerce::rating::{RatingClass, RatingRef};
use crate::commerce::records::{Record, RevisionPolicy};
use crate::commerce::{
    CharterId, FiledRevisionId, LicenceId, LineId, RevisionIdentity, TermGeneration, TermWindow,
    WorldIcSeconds,
};
use mush_core::{ClockRef, ClockStamp, CommitToken, DefinitionDigest, ObjId};
use mush_economy::managed_purchase::FundingReceipt;
use mush_economy::{Amount, InstrumentId};
use mush_testkit::snapshot::Snapshot;
use serde_json::json;

fn token(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}
fn hash(n: u8) -> DefinitionDigest {
    DefinitionDigest::from_bytes([n; 32])
}
fn stamp(at: i64) -> ClockStamp {
    ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at,
    }
}
fn identity() -> TestResult<RevisionIdentity> {
    Ok(serde_json::from_value(json!({"format":1,
        "revision":{"key":{"character":1,"mind":1,"number":1},"digest":hash(1)},"source":hash(2)}))?)
}
fn pin() -> TestResult<RevisionPolicy> {
    Ok(RevisionPolicy::Pin {
        filed: FiledRevisionId::new(3)?,
        revision: identity()?,
    })
}
fn rating() -> TestResult<RatingRef> {
    Ok(serde_json::from_value(
        json!({"job":token(30),"id":hash(31),"schema":1,"receipt_digest":hash(32)}),
    )?)
}
fn funding() -> TestResult<FundingReceipt> {
    Ok(serde_json::from_value(
        json!({"contract":1,"hold":2,"transaction":3,"journal_seq":4,
        "consent_hash":([7_u8;32]),"at_ic":stamp(100)}),
    )?)
}
fn policy(id: &str, version: u32) -> TestResult<mush_core::DefinitionRef> {
    use mush_core::mechanics::{
        DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
    };
    Ok(DefinitionRegistryBuilder::new().register(
        OwnerId::new("market")?,
        DefinitionDescriptor::new(
            DefinitionKind::new("entitlement_policy")?,
            DefinitionId::new(id)?,
            version,
            json!({"format":version}),
        )?,
    )?)
}

/// The rows a grant writes beside the allocation: the licence, its first term
/// (issued at the funding stamp, 100 IC seconds and 20 of grace, as the template
/// says) and the holder's paid-holdings row.
#[allow(clippy::too_many_arguments)]
fn granted_rows(
    s: &mut Snapshot,
    f: &CommerceFamilies,
    licence: LicenceId,
    charter: CharterId,
    line: LineId,
    holder: crate::commerce::Holder,
    terms: PaidTerms,
    operation: CommitToken,
) -> TestResult {
    let one = TermGeneration::new(1)?;
    let pointer = PersonalPaidLicence::original(licence, charter, line, holder, one, operation)?;
    let term = PaidTermRecord::original(
        licence,
        one,
        TermWindow::new(
            WorldIcSeconds::from_stamp(&stamp(100))?,
            WorldIcSeconds::from_stamp(&stamp(100))?,
            WorldIcSeconds::from_stamp(&stamp(200))?,
            WorldIcSeconds::from_stamp(&stamp(220))?,
        )?,
        terms,
        pin()?,
        operation,
    )?;
    term.matches_current(&pointer)?;
    check_eq!(term.key(), format!("{}/{}", licence.key(), one.key()));
    let bytes = rig::document(s, "granted licence", &f.paid_licences, &pointer)?;
    rig::read_back(&bytes, &pointer)?;
    let bytes = rig::document(s, "granted term", &f.paid_terms, &term)?;
    rig::read_back(&bytes, &term)?;
    let holding = PaidHolding::new(holder, licence)?;
    rig::document(s, "granted holding", &f.paid_holdings, &holding)?;
    Ok(())
}

macro_rules! purchase_walk {
    (
        test: $test:ident,
        title: $title:literal,
        file: $file:literal,
        protocols: [$($protocol:expr),+ $(,)?],
        allocations: $allocations:ident,
        economics: $economics:ident,
    ) => {
        #[test]
        fn $test() -> TestResult {
            let (_engine, f) = rig::families()?;
            let mut s = Snapshot::new();
            s.section(concat!($title, ": families"));
            rig::family(&mut s, &f.$allocations);
            rig::family(&mut s, &f.$economics);
            rig::family(&mut s, &f.paid_licences);
            rig::family(&mut s, &f.paid_terms);
            rig::family(&mut s, &f.paid_holdings);
            let manifest = rig::manifest()?;
            let participants = rig::participants()?;
            s.wire("manifest", &manifest)?;
            for protocol in [$($protocol),+] {
                let name = serde_json::to_value(protocol)?
                    .as_str()
                    .ok_or("protocol name")?
                    .to_owned();
                s.section(&format!("{}: {name}: protocol", $title));
                rig::descriptor(
                    &mut s,
                    "list",
                    "market",
                    &writer::listing_descriptor(protocol, &manifest, &participants)?,
                )?;
                for (label, d) in ["fund", "grant", "refund"]
                    .into_iter()
                    .zip(writer::purchase_descriptors(protocol, &manifest, &participants)?)
                {
                    rig::descriptor(&mut s, label, "market", &d)?;
                }

                let funded = allocation(protocol)?;
                let ends = [
                    ("funded", PurchasePhase::Funded {}, funded.operation),
                    (
                        "granted",
                        PurchasePhase::Granted { commit: token(11) },
                        token(11),
                    ),
                    (
                        "refunded",
                        PurchasePhase::Refunded { commit: token(12) },
                        token(12),
                    ),
                ];
                for (phase, value, updated_by) in ends {
                    s.section(&format!("{}: {name}: {phase}", $title));
                    let row = Allocation {
                        phase: value,
                        updated_by,
                        ..funded.clone()
                    };
                    let bytes = rig::document(&mut s, "allocation", &f.$allocations, &row)?;
                    rig::read_back(&bytes, &row)?;
                    for root in audit::required(&row)? {
                        s.line("required root", &root.anchor.record());
                        s.wire("required root identity", &root.identity)?;
                    }
                    if phase == "funded" {
                        rig::document(&mut s, "economics", &f.$economics, &row.economics)?;
                        s.wire("economics digest", &row.economics.digest()?)?;
                        s.wire("grant digest", &row.economics.grant().digest()?)?;
                    }
                    if phase == "granted" {
                        granted_rows(
                            &mut s,
                            &f,
                            row.licence,
                            row.charter,
                            row.line,
                            row.holder,
                            row.terms,
                            row.operation,
                        )?;
                    }
                    s.codec(&format!("allocation codec ({name}, {phase})"), &row, |j| {
                        j["format"] = 9.into()
                    })?;
                }
            }
            s.check(env!("CARGO_MANIFEST_DIR"), $file)?;
            Ok(())
        }
    };
}

mod personal {
    use super::*;
    use crate::commerce::entitlement::{
        DownstreamTermsV1, EntitlementAllocationV1 as Allocation, FiledRoyaltyTermsV1,
        GrantEconomicsV1, PaidTermEconomicsV1, RenewalTemplateV1, audit, writer,
    };
    use mush_economy::managed_entitlement::EntitlementProtocol;

    fn allocation(protocol: EntitlementProtocol) -> TestResult<Allocation> {
        let licence = LicenceId::new(7)?;
        let one = TermGeneration::new(1)?;
        let terms =
            PaidTerms::original_personal_form(RatingClass::I, PaidForm::OpenResale {}, true)?;
        let chain = FiledRoyaltyTermsV1::capture(
            FiledRevisionId::new(3)?,
            identity()?,
            token(2),
            Vec::new(),
        )?;
        let grant = GrantEconomicsV1::new(
            DownstreamTermsV1::Source {
                royalty_permille: 73,
            },
            RenewalTemplateV1::new(
                InstrumentId::new("cr").ok_or("instrument")?,
                Amount::from_minor(500),
                100,
                20,
                terms,
                pin()?,
                policy("personal", 1)?,
            )?,
            chain.reference()?,
        )?;
        let row = Allocation {
            format: 1,
            protocol,
            operation: token(10),
            participants: rig::participants()?,
            licence,
            term: one,
            quote: token(9),
            quote_digest: hash(8),
            product_digest: hash(9),
            holder: ObjId(5).into(),
            charter: CharterId::new(9)?,
            line: LineId::new(2)?,
            filed: FiledRevisionId::new(3)?,
            revision: identity()?,
            rating: rating()?,
            terms,
            economics: PaidTermEconomicsV1::capture(licence, one, token(10), hash(9), grant)?,
            funding: funding()?,
            phase: PurchasePhase::Funded {},
            updated_by: token(10),
        };
        row.validate()?;
        Ok(row)
    }

    purchase_walk! {
        test: ct0_personal_purchase_snapshot,
        title: "personal purchase",
        file: "tests/ct0/purchase_personal.snap",
        protocols: [EntitlementProtocol::OriginalV3, EntitlementProtocol::DerivedV1],
        allocations: entitlement_allocations,
        economics: term_economics,
    }
}

mod house {
    use super::*;
    use crate::commerce::house_catalogue::HouseIssuer;
    use crate::commerce::house_purchase::{
        HouseAllocationV1 as Allocation, HouseGrantEconomicsV1, HouseProtocol,
        HouseRenewalTemplateV1, HouseTermEconomicsV1, audit, writer,
    };

    fn allocation(protocol: HouseProtocol) -> TestResult<Allocation> {
        let licence = LicenceId::new(7)?;
        let one = TermGeneration::new(1)?;
        let terms = writer::HousePurchaseTerms::personal_terms_for_protocol(
            protocol,
            RatingClass::I,
            false,
        )?;
        let grant = HouseGrantEconomicsV1::new(HouseRenewalTemplateV1::for_protocol(
            protocol,
            InstrumentId::new("cr").ok_or("instrument")?,
            Amount::from_minor(500),
            100,
            20,
            terms,
            pin()?,
            policy("house_personal", protocol.version())?,
        )?)?;
        let row = Allocation {
            format: protocol.format(),
            protocol,
            operation: token(10),
            participants: rig::participants()?,
            licence,
            term: one,
            quote: token(9),
            quote_digest: hash(8),
            product_digest: hash(9),
            holder: ObjId(5).into(),
            issuer: HouseIssuer::new(mush_org::OrgId(4), mush_org::OrgScopeId(6))?,
            charter: CharterId::new(9)?,
            line: LineId::new(2)?,
            filed: FiledRevisionId::new(3)?,
            revision: identity()?,
            rating: rating()?,
            terms,
            economics: HouseTermEconomicsV1::capture(licence, one, token(10), hash(9), grant)?,
            funding: funding()?,
            phase: PurchasePhase::Funded {},
            updated_by: token(10),
        };
        row.validate()?;
        Ok(row)
    }

    purchase_walk! {
        test: ct0_house_purchase_snapshot,
        title: "house purchase",
        file: "tests/ct0/purchase_house.snap",
        protocols: [HouseProtocol::HouseV1, HouseProtocol::HouseV2],
        allocations: house_allocations,
        economics: house_economics,
    }
}
