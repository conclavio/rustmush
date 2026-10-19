//! Actual registered reads exercise the shared row/call budget. Internal lens
//! refusal cases additionally prove that batching cannot replace owner checks.
use super::*;

pub(super) fn read(
    ctx: &OwnerReadContext<'_>,
    family: &StateFamilyHandle<u32, u32>,
    mode: u8,
) -> Result<Answer, OwnerReadError> {
    let initial = ctx.action_state(family, &1)?.copied();
    let value = match mode {
        20 => {
            let rows = ctx.action_states(family, &[1, 404, 1])?;
            json!(rows.into_iter().map(|row| row.copied()).collect::<Vec<_>>())
        }
        21 => {
            let mut result = Vec::new();
            // Two maximum-size real Registry certificates have 117 pages each.
            // Thirty further joins exceed the old per-key call budget while
            // remaining inside the unchanged 512-row work budget.
            for keys in [(1..=117).collect::<Vec<u32>>(), (118..=234).collect()] {
                for batch in keys.chunks(MAX_OWNER_READ_PAGE) {
                    result.extend(
                        ctx.action_states(family, batch)?
                            .into_iter()
                            .map(|row| row.copied()),
                    );
                }
            }
            for _ in 0..30 {
                assert_eq!(ctx.action_state(family, &1)?.copied(), initial);
            }
            json!(result)
        }
        22 | 23 => {
            // Even caught errors poison the result. Repeated/missing keys may
            // not be used to discount work and replenish the shared allowance.
            let key = if mode == 22 { 1 } else { 404 };
            for _ in 0..8 {
                let _ = ctx.action_states(family, &vec![key; MAX_OWNER_READ_PAGE]);
            }
            assert_eq!(ctx.action_state(family, &1), Err(OwnerReadError::Limit));
            json!("caught exhausted batch")
        }
        24 | 25 => {
            let keys = vec![
                1;
                if mode == 24 {
                    0
                } else {
                    MAX_OWNER_READ_PAGE + 1
                }
            ];
            assert_eq!(ctx.action_states(family, &keys), Err(OwnerReadError::Limit));
            assert_eq!(ctx.action_state(family, &1), Err(OwnerReadError::Limit));
            json!("caught invalid batch")
        }
        26 | 27 => {
            let remaining = if mode == 26 {
                MAX_OWNER_READS - 1
            } else {
                MAX_OWNER_READS
            };
            for _ in 0..remaining {
                let _ = ctx.action_states(family, &[1]);
            }
            json!(initial)
        }
        28 | 29 => {
            for _ in 0..7 {
                assert_eq!(ctx.action_states(family, &[1; 64])?.len(), 64);
            }
            assert_eq!(ctx.action_states(family, &[1; 63])?.len(), 63);
            if mode == 29 {
                assert_eq!(ctx.action_state(family, &1), Err(OwnerReadError::Limit));
            }
            json!(initial)
        }
        _ => return Err(OwnerReadError::Refused),
    };
    Ok(Answer {
        value,
        on_serialize: None,
    })
}

#[test]
fn exact_batch_preserves_order_missing_duplicates_and_real_two_receipt_workload() {
    for reversed in [false, true] {
        let mut f = fixture(reversed, true).unwrap();
        assert_eq!(f.read(20, false).unwrap(), json!([42, null, 42]));
        let mut txn = f.engine.action_transaction(&f.family).unwrap();
        for key in 2..=234 {
            txn.insert(&f.family, &key, 1000 + key).unwrap();
        }
        let (_, installation) = txn.prepare().unwrap().into_parts();
        f.engine.install_prepared_action(installation).unwrap();
        let expected = json!(
            (1..=234)
                .map(|key| if key == 1 { 42 } else { 1000 + key })
                .collect::<Vec<_>>()
        );
        for query in [false, true] {
            assert_eq!(f.read(21, query).unwrap(), expected);
            assert_eq!(f.engine.action_state().len(), 234);
            assert!(!f.engine.action_state().has_prepared());
        }
    }
}

#[test]
fn exact_batch_shares_unchanged_call_and_row_limits_with_sticky_exhaustion() {
    for query in [false, true] {
        for mode in [22, 23, 24, 25, 27, 29] {
            let mut f = fixture(false, true).unwrap();
            assert_eq!(
                f.read(mode, query),
                Err(OwnerReadError::Limit),
                "mode {mode}"
            );
            assert_eq!(f.engine.action_state().len(), 1);
            assert!(!f.engine.action_state().has_prepared());
        }
        for mode in [26, 28] {
            let mut f = fixture(false, true).unwrap();
            assert_eq!(f.read(mode, query).unwrap(), json!(42), "mode {mode}");
        }
    }
}

#[test]
fn exact_batch_cannot_reuse_a_foreign_handle_owner_or_detached_lens() {
    let f = fixture(false, true).unwrap();
    let other = fixture(false, true).unwrap();
    let caller = OwnerReadCaller::object("consumer", GOD).unwrap();
    for (lens, owner, family) in [
        (
            TypedStateLens::Live(f.engine.action_state()),
            "owner",
            &other.family,
        ),
        (
            TypedStateLens::Live(f.engine.action_state()),
            "consumer",
            &f.family,
        ),
        (
            TypedStateLens::Detached(crate::action_state::DetachedTypedState::CompatibilityCtx),
            "owner",
            &f.family,
        ),
    ] {
        let ctx = OwnerReadContext {
            world: &f.engine.world,
            typed: ModuleActionState::for_module(lens, Some(owner)),
            caller: &caller,
            reads: Cell::new(0),
            rows: Cell::new(0),
            exhausted: Cell::new(false),
        };
        assert_eq!(
            ctx.action_states(family, &[1, 404]),
            Err(OwnerReadError::Integrity)
        );
        assert_eq!(ctx.reads.get(), 1);
        assert_eq!(ctx.rows.get(), 2);
    }
    assert_eq!(f.engine.action_state().len(), 1);
}
