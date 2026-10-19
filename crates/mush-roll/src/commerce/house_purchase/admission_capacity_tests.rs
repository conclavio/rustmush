//! Actual family codec/envelope boundary, not a licence or paid authority.
use super::*;
use crate::commerce::{
    FiledRevisionId, LicenceId, MAX_GRACE_SECONDS, MAX_TERM_SECONDS, RevisionIdentity,
    house_purchase::{HouseGrantEconomicsV1, HouseRenewalTemplateV1},
    paid::{PaidForm, PaidTerms},
    rating::RatingClass,
    records::RevisionPolicy,
};
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::{
    ActionStateError, CommitToken, DefinitionDigest, Engine, GameModule, ModError, Registrar,
    Registry, World,
};
use mush_economy::{Amount, InstrumentId};
use std::sync::{Arc, Mutex};
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
type Handle = StateFamilyHandle<String, HouseTermEconomicsV1>;
struct Owner(Arc<Mutex<Option<Handle>>>);
impl GameModule for Owner {
    fn name(&self) -> &'static str {
        "roll"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let families = crate::commerce::families::CommerceFamilies::register(reg)?;
        *self.0.lock().map_err(|_| ModError::msg("handle lock"))? = Some(families.house_economics);
        Ok(())
    }
}
fn row(length: usize, generation: u64) -> TestResult<HouseTermEconomicsV1> {
    let digest = DefinitionDigest::from_bytes([255; 32]);
    let revision: RevisionIdentity = serde_json::from_value(serde_json::json!({"format":1,
        "revision":{"key":{"character":u32::MAX,"mind":u64::MAX,"number":u64::MAX},"digest":digest},
        "source":digest}))?;
    let terms = PaidTerms::original_personal_form(RatingClass::III, PaidForm::Sealed {}, true)?;
    let policy = DefinitionRegistryBuilder::new().register(
        OwnerId::new("market")?,
        DefinitionDescriptor::new(
            DefinitionKind::new("entitlement_policy")?,
            DefinitionId::new("house_personal")?,
            1,
            serde_json::json!({"format":1}),
        )?,
    )?;
    let renewal = HouseRenewalTemplateV1::new(
        InstrumentId::new("x".repeat(length)).ok_or("instrument")?,
        Amount::from_minor(i64::MAX),
        MAX_TERM_SECONDS,
        MAX_GRACE_SECONDS,
        terms,
        RevisionPolicy::Pin {
            filed: FiledRevisionId::new(u64::MAX)?,
            revision,
        },
        policy,
    )?;
    Ok(HouseTermEconomicsV1::capture(
        LicenceId::new(u64::MAX)?,
        TermGeneration::new(generation)?,
        CommitToken::from_bytes([255; 16]),
        digest,
        HouseGrantEconomicsV1::new(renewal)?,
    )?)
}
fn document(family: &Handle, row: &HouseTermEconomicsV1) -> serde_json::Value {
    serde_json::json!({"family":{"owner":family.owner(),"id":family.descriptor().id()},
        "key":row.key(),"schema":{"id":family.descriptor().value_schema(),"version":1},"value":row})
}
#[test]
fn house_complete_envelope_reserves_future_generation_digits_before_money() -> TestResult {
    let shared = Arc::new(Mutex::new(None));
    let mut engine = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(Owner(shared.clone()))],
    )?;
    let handle = shared
        .lock()
        .map_err(|_| "handle lock")?
        .clone()
        .ok_or("handle")?;
    let base = row(1, 1)?;
    let base_len = canonical_json_bytes_with_limit(&document(&handle, &base), 8192)?.len();
    let width = 8192usize
        .checked_sub(base_len)
        .and_then(|n| n.checked_add(1))
        .ok_or("width")?;
    let birth = row(width, 1)?;
    // The real old body codec accepts this row. That does not prove future
    // complete-envelope capacity, and admission must reject it now.
    assert_eq!(
        HouseTermEconomicsV1::decode(&serde_json::to_vec(&birth)?)?,
        birth
    );
    assert_eq!(
        canonical_json_bytes_with_limit(&document(&handle, &birth), 8192)?.len(),
        8192
    );
    assert!(check(&handle, &birth).is_err());
    let mut txn = engine.action_transaction(&handle)?;
    txn.insert(&handle, &birth.key(), birth.clone())?;
    // Insertion uses the real registered complete-envelope encoder. Publishing
    // this isolated economics row is deliberately not paid authority.
    assert!(matches!(
        txn.prepare(),
        Err(ActionStateError::TerminalCapacity(
            "house writes require registered protocol"
        ))
    ));
    let future = row(width, u64::MAX)?;
    assert_eq!(future.key().len(), birth.key().len());
    let mut txn = engine.action_transaction(&handle)?;
    assert!(txn.insert(&handle, &future.key(), future).is_err());
    drop(txn);
    let safe_width = width.checked_sub(19).ok_or("digit reserve")?;
    let safe = row(safe_width, 1)?;
    check(&handle, &safe)?;
    for generation in [1, 9, 10, u64::MAX] {
        let future = row(safe_width, generation)?;
        assert!(canonical_json_bytes_with_limit(&document(&handle, &future), 8192)?.len() <= 8192);
        let mut independent = Engine::with_modules(
            World::new_minimal(),
            Registry::new(),
            vec![Box::new(Owner(shared.clone()))],
        )?;
        let independent_handle = shared
            .lock()
            .map_err(|_| "handle lock")?
            .clone()
            .ok_or("handle")?;
        let mut txn = independent.action_transaction(&independent_handle)?;
        txn.insert(&independent_handle, &future.key(), future)?;
        // Capacity acceptance is tested at the actual typed insertion boundary;
        // the paid protocol must still refuse an isolated protected row.
        assert!(matches!(
            txn.prepare(),
            Err(ActionStateError::TerminalCapacity(
                "house writes require registered protocol"
            ))
        ));
    }
    Ok(())
}
