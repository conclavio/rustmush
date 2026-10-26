//! Exact, bounded Conduct selectors. Selectors and decoded approvals are data;
//! only the real Roll reader supplies the opaque historical/current proof.
use super::records::{ConductSelection, Publication};
use super::{ApprovedConductV1, CoverageError, CoverageProductV1, SafeConductRole};
use mush_core::read_service::OwnerReadHandle;
use mush_core::{ActionTxn, CommitToken, DefinitionDigest, DefinitionRef, PrepareCtx};
use mush_roll::commerce::conduct::{
    ConductReadRequest, ConductReader, ConductRole, ConductTrialProof,
};
use mush_roll::commerce::coverage::CoverageOpenFacts;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
pub(super) struct Configuration {
    pub(super) manifest: DefinitionRef,
    pub(super) selections: BTreeMap<DefinitionDigest, Vec<CommitToken>>,
}
impl Configuration {
    pub(super) fn validate(&self, products: &[CoverageProductV1]) -> Result<(), CoverageError> {
        if self.selections.is_empty() || self.selections.len() > 16 {
            return Err(CoverageError::Bound);
        }
        let mut expected = BTreeSet::new();
        for p in products {
            if p.approvals().is_empty() {
                continue;
            }
            let digest = p.digest()?;
            let operations = self.selections.get(&digest).ok_or(CoverageError::Product)?;
            if operations.len() != p.approvals().len()
                || operations.len() > 16
                || operations.iter().collect::<BTreeSet<_>>().len() != operations.len()
            {
                return Err(CoverageError::Product);
            }
            expected.insert(digest);
        }
        if expected != self.selections.keys().copied().collect() {
            return Err(CoverageError::Product);
        }
        Ok(())
    }
}

pub(super) fn matches(approval: &ApprovedConductV1, proof: &ConductTrialProof) -> bool {
    approval.charter() == proof.charter()
        && approval.line() == proof.line()
        && approval.filed() == proof.filed()
        && approval.revision() == proof.revision()
        && approval.certification() == proof.rating().receipt_digest()
        && approval.role_trial() == proof.digest()
        && approval.profile() == proof.profile()
        && matches!(
            (approval.role(), proof.role()),
            (SafeConductRole::Escort, ConductRole::Escort)
                | (SafeConductRole::Transit, ConductRole::Transit)
        )
}

pub(super) fn selections(
    product: &CoverageProductV1,
    manifest: Option<&DefinitionRef>,
    proofs: &[ConductTrialProof],
) -> Result<Vec<ConductSelection>, CoverageError> {
    if proofs.len() != product.approvals().len() || proofs.len() > 16 {
        return Err(CoverageError::Product);
    }
    let mut seen = BTreeSet::new();
    product
        .approvals()
        .iter()
        .zip(proofs)
        .map(|(approval, proof)| {
            if manifest != Some(proof.manifest())
                || !matches(approval, proof)
                || !seen.insert(proof.operation())
            {
                return Err(CoverageError::Authority);
            }
            Ok(ConductSelection {
                approval: approval.clone(),
                trial: proof.operation(),
                trial_digest: proof.digest(),
            })
        })
        .collect()
}

pub(super) fn matches_retained(
    selection: &ConductSelection,
    proof: &mush_roll::commerce::coverage::CoverageConductRef,
) -> bool {
    let a = &selection.approval;
    selection.trial == proof.trial()
        && selection.trial_digest == proof.receipt()
        && a.role_trial() == proof.receipt()
        && a.charter() == proof.charter()
        && a.line() == proof.line()
        && a.filed() == proof.filed()
        && a.revision() == proof.revision()
        && a.certification() == proof.rating().receipt_digest()
        && a.profile() == proof.profile()
        && matches!(
            (a.role(), proof.role()),
            (SafeConductRole::Escort, ConductRole::Escort)
                | (SafeConductRole::Transit, ConductRole::Transit)
        )
}

pub(super) fn publication_proofs(
    ctx: &mut PrepareCtx<'_>,
    txn: &mut ActionTxn<'_>,
    product: &CoverageProductV1,
    configuration: Option<&Configuration>,
    reader: Option<&OwnerReadHandle<ConductReader>>,
) -> Result<Vec<ConductTrialProof>, CoverageError> {
    if product.approvals().is_empty() {
        return Ok(Vec::new());
    }
    let config = configuration.ok_or(CoverageError::Authority)?;
    let reader = reader.ok_or(CoverageError::Authority)?;
    let ops = config
        .selections
        .get(&product.digest()?)
        .ok_or(CoverageError::Product)?;
    let mut proofs = Vec::with_capacity(ops.len());
    for op in ops {
        proofs.push(
            ctx.read_owner_preimage(txn, reader, ConductReadRequest::metadata(*op))
                .map_err(|_| CoverageError::Authority)?,
        );
    }
    selections(product, Some(&config.manifest), &proofs)?;
    Ok(proofs)
}

// The launch qualification covers the actual single selected profile. A name
// somewhere in a multi-profile catalog is not evidence that it will execute.
fn actual_profile(departure: &crate::minds::ic_sortie::AcknowledgedDeparture, name: &str) -> bool {
    let seal = departure.seal();
    seal.governed
        && seal.engine == crate::arena::mind::SealEngine::Scripted
        && seal.identity() == seal.requested
        && seal.cards.selected.len() == 1
        && seal.cards.selected.name(0) == Some(name)
}

pub(super) fn current(
    ctx: &mut PrepareCtx<'_>,
    txn: &mut ActionTxn<'_>,
    reader: Option<&OwnerReadHandle<ConductReader>>,
    publication: &Publication,
    departure: &crate::minds::ic_sortie::AcknowledgedDeparture,
    facts: &CoverageOpenFacts,
) -> Result<Option<(ConductSelection, ConductTrialProof)>, CoverageError> {
    for selected in &publication.conduct {
        let a = &selected.approval;
        if a.charter() != facts.charter()
            || a.line() != facts.line()
            || a.filed() != facts.filed()
            || a.revision() != facts.revision()
            || a.certification() != facts.rating().receipt_digest()
            || !actual_profile(departure, a.profile())
        {
            continue;
        }
        let proof = ctx
            .read_owner_preimage(
                txn,
                reader.ok_or(CoverageError::Authority)?,
                ConductReadRequest::current(selected.trial),
            )
            .map_err(|_| CoverageError::Authority)?;
        if !matches(a, &proof)
            || proof.rating() != facts.rating()
            || proof.digest() != selected.trial_digest
        {
            return Err(CoverageError::Authority);
        }
        return Ok(Some((selected.clone(), proof)));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    type TestResult = Result<(), Box<dyn std::error::Error>>;
    fn token(n: u8) -> Result<CommitToken, Box<dyn std::error::Error>> {
        Ok(CommitToken::from_hex(&format!("{n:032x}"))?)
    }
    #[test]
    fn proposed_approval_requires_exact_complete_operation_selectors() -> TestResult {
        let product = super::super::product_tests::product()?;
        // Structural config uses an inert reference. Actual composition must
        // obtain the manifest and reader from the real Conduct registration.
        let manifest = product.hulls().first().ok_or("fixture hull")?.clone();
        let key = product.digest()?;
        let good = Configuration {
            manifest,
            selections: BTreeMap::from([(key, vec![token(1)?])]),
        };
        good.validate(std::slice::from_ref(&product))?;
        for operations in [vec![], vec![token(1)?, token(1)?], vec![token(1)?; 17]] {
            let mut bad = good.clone();
            bad.selections.insert(key, operations);
            check!(bad.validate(std::slice::from_ref(&product)).is_err());
        }
        let mut missing = good.clone();
        missing.selections.clear();
        check!(missing.validate(std::slice::from_ref(&product)).is_err());
        let mut extra = good;
        extra
            .selections
            .insert(DefinitionDigest::from_bytes([99; 32]), vec![token(2)?]);
        check!(extra.validate(std::slice::from_ref(&product)).is_err());
        Ok(())
    }
    #[test]
    fn decoded_approval_without_real_proof_cannot_publish_qualification() -> TestResult {
        let product = super::super::product_tests::product()?;
        let decoded = CoverageProductV1::decode(&serde_json::to_vec(&product)?)?;
        check!(selections(&decoded, decoded.hulls().first(), &[]).is_err());
        let mut raw = serde_json::to_value(&decoded)?;
        raw["approvals"] = serde_json::json!([]);
        let ordinary: CoverageProductV1 = serde_json::from_value(raw)?;
        check!(selections(&ordinary, None, &[])?.is_empty());
        Ok(())
    }
    #[test]
    fn retained_selection_checks_exact_receipt_and_full_approved_identity() -> TestResult {
        let product = super::super::product_tests::product()?;
        let approval = product.approvals().first().ok_or("approval")?.clone();
        let selection = ConductSelection {
            trial: token(1)?,
            trial_digest: approval.role_trial(),
            approval: approval.clone(),
        };
        // A decoded retention is data only. The boot path additionally proves
        // the real native/typed result and both source-root directions.
        let raw = serde_json::json!({"trial":selection.trial,"receipt":selection.trial_digest,
            "charter":approval.charter(),"line":approval.line(),"filed":approval.filed(),
            "revision":approval.revision(),"rating":{"job":token(2)?,"id":DefinitionDigest::from_bytes([8;32]),
                "schema":1,"receipt_digest":approval.certification()},
            "profile":approval.profile(),"role":"escort","manifest":product.hulls().first().ok_or("hull")?});
        let data: mush_roll::commerce::coverage::CoverageConductRef =
            serde_json::from_value(raw.clone())?;
        check!(matches_retained(&selection, &data));
        for (pointer, replacement) in [
            ("/trial", serde_json::json!(token(3)?)),
            (
                "/receipt",
                serde_json::json!(DefinitionDigest::from_bytes([6; 32])),
            ),
            ("/charter", serde_json::json!("25")),
            ("/line", serde_json::json!("2")),
            ("/filed", serde_json::json!("2")),
            ("/profile", serde_json::json!("Transit")),
            ("/role", serde_json::json!("transit")),
            (
                "/revision/source",
                serde_json::json!(DefinitionDigest::from_bytes([5; 32])),
            ),
            (
                "/rating/receipt_digest",
                serde_json::json!(DefinitionDigest::from_bytes([4; 32])),
            ),
        ] {
            let mut changed = raw.clone();
            *changed.pointer_mut(pointer).ok_or("fixture field")? = replacement;
            let changed: mush_roll::commerce::coverage::CoverageConductRef =
                serde_json::from_value(changed)?;
            check!(
                !matches_retained(&selection, &changed),
                "crossed qualification identity"
            );
        }
        Ok(())
    }
}
