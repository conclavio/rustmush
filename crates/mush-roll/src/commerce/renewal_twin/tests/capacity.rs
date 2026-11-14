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
fn wide_birth<I: Rig>(width: usize) -> TestResult<Birth<I>> {
    let mut b = birth::<I>()?;
    let grant = I::widen(
        I::grant_of(&b.economics),
        InstrumentId::new("x".repeat(width)).ok_or("instrument")?,
    )?;
    b.economics = I::economics_row(
        b.economics.licence(),
        b.economics.generation(),
        b.economics.operation(),
        I::product(&b.economics),
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
pub(crate) fn admitted_widest_economics_keeps_all_actual_roll_terminal_envelopes_bounded<I: Rig>()
-> TestResult {
    let (_engine, f) = families()?;
    let base = wide_birth::<I>(1)?;
    let mut max = I::economics_row(
        base.economics.licence(),
        TermGeneration::new(u64::MAX)?,
        base.economics.operation(),
        I::product(&base.economics),
        I::grant_of(&base.economics).clone(),
    )?;
    let initial = canonical_json_bytes_with_limit(&document(I::economics(&f), &max), 8192)?.len();
    let width = 8192usize
        .checked_sub(initial)
        .and_then(|n| n.checked_add(1))
        .ok_or("width")?;
    let b = wide_birth::<I>(width)?;
    max = I::economics_row(
        b.economics.licence(),
        TermGeneration::new(u64::MAX)?,
        b.economics.operation(),
        I::product(&b.economics),
        I::grant_of(&b.economics).clone(),
    )?;
    check_eq!(
        canonical_json_bytes_with_limit(&document(I::economics(&f), &max), 8192)?.len(),
        8192
    );
    I::admission_capacity(I::economics(&f), &b.economics)?;
    let opened = draft::<I>(
        &b,
        10,
        20,
        RenewalAttemptV1::BuyerRetry {
            previous_operation: token(8),
        },
    )?;
    super::super::capacity::check::<I>(&f, &opened)?;
    let funded = funded::<I>(&opened)?;
    let rows: [fn(&I::Operation, &I::Operation) -> TestResult<I::Operation>; 5] = [
        |opened, _| Ok(opened.clone()),
        |_, funded| Ok(funded.clone()),
        |_, funded| terminal::<I>(funded, true),
        |_, funded| terminal::<I>(funded, false),
        |opened, _| closed::<I>(opened),
    ];
    for make in rows {
        let row = make(&opened, &funded)?;
        let bytes = serde_json::to_vec(&row)?;
        check_eq!(I::decode(&bytes)?, row);
        check!(
            canonical_json_bytes_with_limit(
                &document(I::operations(&f), &row),
                <I::Operation as Record>::MAX_BYTES
            )
            .is_ok()
        );
        let head = RenewalHeadV1::<I>::for_operation(&row)?;
        check!(
            canonical_json_bytes_with_limit(
                &document(I::heads(&f), &head),
                RenewalHeadV1::<I>::MAX_BYTES
            )
            .is_ok()
        );
        if row.state().granted() {
            let (licence, term, economics) = row.granted_rows()?;
            check!(
                canonical_json_bytes_with_limit(
                    &document(&f.paid_licences, &licence),
                    PersonalPaidLicence::MAX_BYTES
                )
                .is_ok()
            );
            check!(
                canonical_json_bytes_with_limit(
                    &document(&f.paid_terms, &term),
                    PaidTermRecord::MAX_BYTES
                )
                .is_ok()
            );
            check!(
                canonical_json_bytes_with_limit(
                    &document(I::economics(&f), &economics),
                    <I::Economics as Record>::MAX_BYTES
                )
                .is_ok()
            );
        }
    }
    let oversized = wide_birth::<I>(width.checked_add(1).ok_or("width")?)?;
    check!(I::admission_capacity(I::economics(&f), &oversized.economics).is_err());
    Ok(())
}
