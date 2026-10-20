//! Real registered complete envelopes, with structural receipt data only.
//! These tests do not claim certification, money, or business readiness.
use super::*;
use crate::commerce::families::CommerceFamilies;
use mush_core::{
    Engine, GameModule, ModError, Registrar, Registry, World, canonical_json_bytes_with_limit,
};
use std::sync::{Arc, Mutex};
struct Owner(Arc<Mutex<Option<CommerceFamilies>>>);
impl GameModule for Owner {
    fn name(&self) -> &'static str {
        "roll"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        *self.0.lock().map_err(|_| ModError::msg("family lock"))? =
            Some(CommerceFamilies::register(reg)?);
        Ok(())
    }
}
#[inline(never)]
fn families() -> TestResult<(Box<Engine>, CommerceFamilies)> {
    let shared = Arc::new(Mutex::new(None));
    let engine = Box::new(Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(Owner(shared.clone()))],
    )?);
    let families = shared
        .lock()
        .map_err(|_| "family lock")?
        .clone()
        .ok_or("families")?;
    Ok((engine, families))
}
fn wide_birth(width: usize) -> TestResult<Birth> {
    let mut b = birth()?;
    let old = b.economics.grant();
    let t = old.renewal();
    let template = RenewalTemplateV1::new(
        InstrumentId::new("x".repeat(width)).ok_or("instrument")?,
        t.price(),
        t.term_seconds(),
        t.grace_seconds(),
        t.terms(),
        t.revision(),
        t.policy().clone(),
    )?;
    let grant = GrantEconomicsV1::new(old.downstream(), template, old.royalty_chain())?;
    b.economics = PaidTermEconomicsV1::capture(
        b.economics.licence(),
        b.economics.generation(),
        b.economics.operation(),
        b.economics.product_digest(),
        grant,
    )?;
    Ok(b)
}
fn document<T: Record>(
    family: &mush_core::StateFamilyHandle<String, T>,
    row: &T,
) -> serde_json::Value {
    serde_json::json!({"family":{"owner":family.owner(),"id":family.descriptor().id()},"key":row.key(),
        "schema":{"id":family.descriptor().value_schema(),"version":family.descriptor().schema_version()},"value":row})
}
#[test]
fn admitted_widest_economics_keeps_all_actual_roll_terminal_envelopes_bounded() -> TestResult {
    let (_engine, f) = families()?;
    let base = wide_birth(1)?;
    let mut max = PaidTermEconomicsV1::capture(
        base.economics.licence(),
        TermGeneration::new(u64::MAX)?,
        base.economics.operation(),
        base.economics.product_digest(),
        base.economics.grant().clone(),
    )?;
    let initial = canonical_json_bytes_with_limit(&document(&f.term_economics, &max), 8192)?.len();
    let width = 8192usize
        .checked_sub(initial)
        .and_then(|n| n.checked_add(1))
        .ok_or("width")?;
    let b = wide_birth(width)?;
    max = PaidTermEconomicsV1::capture(
        b.economics.licence(),
        TermGeneration::new(u64::MAX)?,
        b.economics.operation(),
        b.economics.product_digest(),
        b.economics.grant().clone(),
    )?;
    assert_eq!(
        canonical_json_bytes_with_limit(&document(&f.term_economics, &max), 8192)?.len(),
        8192
    );
    crate::commerce::entitlement::admission_capacity::check(&f.term_economics, &b.economics)?;
    let opened = draft(
        &b,
        10,
        20,
        RenewalAttemptV1::BuyerRetry {
            previous_operation: token(8),
        },
    )?;
    crate::commerce::renewal::capacity::check(&f, &opened)?;
    let funded = funded(&opened)?;
    let rows: [fn(&RenewalOperationV1, &RenewalOperationV1) -> TestResult<RenewalOperationV1>; 5] = [
        |opened, _| Ok(opened.clone()),
        |_, funded| Ok(funded.clone()),
        |_, funded| terminal(funded, true),
        |_, funded| terminal(funded, false),
        |opened, _| closed(opened),
    ];
    for make in rows {
        let row = make(&opened, &funded)?;
        let bytes = serde_json::to_vec(&row)?;
        assert_eq!(RenewalOperationV1::decode(&bytes)?, row);
        assert!(
            canonical_json_bytes_with_limit(
                &document(&f.renewal_operations, &row),
                RenewalOperationV1::MAX_BYTES
            )
            .is_ok()
        );
        let head = RenewalHeadV1::for_operation(&row)?;
        assert!(
            canonical_json_bytes_with_limit(
                &document(&f.renewal_heads, &head),
                RenewalHeadV1::MAX_BYTES
            )
            .is_ok()
        );
        if row.state().granted() {
            let (licence, term, economics) = row.granted_rows()?;
            assert!(
                canonical_json_bytes_with_limit(
                    &document(&f.paid_licences, &licence),
                    PersonalPaidLicence::MAX_BYTES
                )
                .is_ok()
            );
            assert!(
                canonical_json_bytes_with_limit(
                    &document(&f.paid_terms, &term),
                    PaidTermRecord::MAX_BYTES
                )
                .is_ok()
            );
            assert!(
                canonical_json_bytes_with_limit(
                    &document(&f.term_economics, &economics),
                    PaidTermEconomicsV1::MAX_BYTES
                )
                .is_ok()
            );
        }
    }
    let oversized = wide_birth(width.checked_add(1).ok_or("width")?)?;
    assert!(
        crate::commerce::entitlement::admission_capacity::check(
            &f.term_economics,
            &oversized.economics
        )
        .is_err()
    );
    Ok(())
}
