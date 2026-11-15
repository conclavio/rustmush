#![cfg(test)]

//! Doc 46 unit CT.8: the renewal story's scenarios, one body for both issuers.
//!
//! The personal renewal story (`renewal_integration_tests.rs`) and the House one
//! (`house_renewal_integration_tests.rs`) drive the same protocol through each family's own
//! actions. The scenarios here were the personal story's; each is now written once over a
//! [`Twin`] and run for both. The personal story runs them where and as it did, every
//! preparation in the order it was. The House story runs them beside its own, under
//! `ct0b::aside`, so that CT.0b's committed record of the House twin (which pins the
//! House story as it was) does not move.
//!
//! Left to the personal story alone, each with its reason in the cut note: the grant chain's
//! payout split and its cold funded grant (the House story's own grant chain checks its own
//! split), the issuer parent's lapse (a House licence's issuer is an organisation, which has
//! no parent term; the House story's issuer loss is its suspension), and the customer's
//! sortie (the House story's `checkout`).

use super::*;
use mush_core::CommitToken;
use mush_core::authoritative::ReplayWrite;
use mush_roll::commerce::renewal::RenewalAttemptV1;
use mush_roll::commerce::{LicenceId, TermGeneration};

/// What a rig's prepare returns: the transaction and the provider's products.
pub(super) type Prepared = (
    PreparedComposedTxn,
    mush_core::action::PreparedAuthoritativeAction,
);

/// A renewal operation as the scenarios compare it, read from either family's paired rows
/// after the family's own pairing checks (the market row against the Roll's, both against
/// Economy's audit, and the receipt when the operation ended).
pub(super) struct Operation {
    pub(super) pending: bool,
    pub(super) retryable: bool,
    pub(super) attempt: RenewalAttemptV1,
    /// The generation the binding renews to.
    pub(super) next: TermGeneration,
    pub(super) contract: Value,
    pub(super) consent: Value,
    pub(super) binding: Value,
    /// When the market's offer expires, on the clock [`Twin::set_time`] sets.
    pub(super) expires_at: i64,
}

/// One issuer's renewal twin, as the scenarios drive it.
pub(super) trait Twin {
    /// The market's caller transitions.
    const DRAFT: &'static str;
    const FUND: &'static str;
    const GRANT: &'static str;
    const REFUND: &'static str;
    const CLOSE: &'static str;
    /// The twin's typed history: the market's operation and receipt, the Roll's operation
    /// and head; and the Roll's paid-term economics family.
    const MARKET_OP: &'static str;
    const RECEIPT: &'static str;
    const ROLL_OP: &'static str;
    const HEAD: &'static str;
    const ECONOMICS: &'static str;
    /// The key prefix of Economy's renewal offers.
    const ECONOMY_OFFER: &'static str;
    /// Whether the twin's seller is a character with a login account of its own, so that
    /// merging it with the renewer's is a fence (the personal issuer). A House is paid
    /// through its treasury; its own account fence is the House story's.
    const LOGIN_PAIR: bool;

    /// The renewed licence.
    fn licence(&self) -> LicenceId;
    /// The charter the licence is issued under: its title is fenced by a pending step.
    fn charter(&self) -> ObjId;
    /// Who renews.
    fn renewer(&self) -> ObjId;
    /// The world with the renewals composed, booted from `saved`.
    fn boot(&self, saved: &Durable) -> TestResult<PlayerWorld>;
    /// The same world without the renewals composed.
    fn boot_disabled(&self, saved: &Durable) -> TestResult<PlayerWorld>;
    /// A world on the granted purchase with its clock at the predecessor's term end.
    fn due_world(&self) -> TestResult<PlayerWorld>;
    /// Sets the clock the licence's windows are on.
    fn set_time(&self, w: &mut PlayerWorld, at: i64) -> TestResult;
    /// The predecessor's term end, on that clock.
    fn term_end(&self, w: &PlayerWorld) -> TestResult<i64>;
    fn disk(&self, w: &PlayerWorld) -> Disk;
    fn capture(&self, w: &PlayerWorld) -> Durable;
    fn balances(&self, w: &PlayerWorld) -> TestResult<Vec<(WalletId, Amount, Amount)>>;
    /// The current licence, its term and its economics, compared whole.
    fn current(&self, w: &PlayerWorld) -> TestResult<Value>;
    /// The licence's paid term at `generation`: refused unless there is exactly one.
    fn term_at(&self, w: &PlayerWorld, generation: TermGeneration) -> TestResult<Value>;
    /// The licence's paid-term economics at `generation`: refused unless there is exactly one.
    fn economics_at(&self, w: &PlayerWorld, generation: TermGeneration) -> TestResult<Value>;
    /// The licence's renewal head: its pending operation and its last one.
    fn head(&self, w: &PlayerWorld) -> TestResult<(Option<CommitToken>, CommitToken)>;
    fn operation(&self, w: &PlayerWorld, n: u128) -> TestResult<Operation>;
    /// The licence's current operation, which a renewal never changes.
    fn licence_operation(&self, w: &PlayerWorld) -> TestResult<CommitToken>;
    /// The operation that bore the licence.
    fn birth_operation(&self) -> CommitToken;
    /// The title holder of [`Twin::charter`], as Economy holds it.
    fn title_owner(&self, w: &PlayerWorld) -> TestResult<Value>;
    fn prepare(
        &self,
        w: &mut PlayerWorld,
        id: &str,
        actor: ObjId,
        n: u128,
        params: Value,
    ) -> TestResult<Prepared>;
    #[allow(clippy::too_many_arguments)]
    fn stage(
        &self,
        w: &mut PlayerWorld,
        id: &str,
        actor: ObjId,
        n: u128,
        params: Value,
        prepared: Prepared,
    ) -> TestResult;
    /// The renewer's step through the twin's public offer, installed.
    fn step(&self, w: &mut PlayerWorld, id: &str, n: u128, params: Value) -> TestResult;
    /// Merges the seller's login account with the renewer's and returns the seller's; only
    /// called when [`Twin::LOGIN_PAIR`].
    fn merge_login_accounts(&self) -> TestResult<mush_core::AccountId>;
    fn restore_login_account(&self, seller: mush_core::AccountId) -> TestResult;
}

fn licence_params<T: Twin>(t: &T) -> Value {
    json!({"licence":t.licence().get()})
}
fn operation_params(n: u128) -> Value {
    json!({"operation":token(n)})
}
fn step_for<T: Twin>(t: &T, w: &mut PlayerWorld, id: &str, n: u128, operation: u128) -> TestResult {
    t.step(w, id, n, operation_params(operation))
}
fn same_disk<T: Twin>(t: &T, w: &PlayerWorld, before: &Disk) -> TestResult {
    check!(
        t.disk(w) == *before,
        "pre-ACK or refused action changed retained images"
    );
    Ok(())
}

/// Each way an attempt closes without a grant (declined, expired, refunded), then the
/// buyer's linked retry with fresh consent to the next generation.
pub(super) fn retries<T: Twin>(t: &T, start: u128) -> TestResult {
    for (index, ending) in ["declined", "expired", "refund"].into_iter().enumerate() {
        let number = start + 100 + u128::try_from(index)? * 10;
        let mut world = t.due_world()?;
        let current = t.current(&world)?;
        let initial = t.balances(&world)?;
        t.step(&mut world, T::DRAFT, number, licence_params(t))?;
        if ending == "refund" {
            step_for(t, &mut world, T::FUND, number + 1, number)?;
            step_for(t, &mut world, T::REFUND, number + 2, number)?;
        } else {
            if ending == "expired" {
                let expires = t.operation(&world, number)?.expires_at;
                check!(
                    t.prepare(
                        &mut world,
                        T::CLOSE,
                        t.renewer(),
                        number + 2,
                        json!({"operation":token(number),"reason":"expired"})
                    )
                    .is_err()
                );
                t.set_time(&mut world, expires)?;
            }
            t.step(
                &mut world,
                T::CLOSE,
                number + 2,
                json!({"operation":token(number),"reason":ending}),
            )?;
        }
        let closed = t.operation(&world, number)?;
        check!(closed.retryable);
        check_eq!(t.balances(&world)?, initial);
        check_eq!(t.current(&world)?, current);
        check!(crate::test_checks::refused(t.term_at(&world, closed.next))?);
        check!(crate::test_checks::refused(
            t.economics_at(&world, closed.next)
        )?);
        check_eq!(t.head(&world)?.0, None);
        // Terminal history is durable before any retry and grants no extension.
        let closed_snapshot = t.capture(&world);
        let mut world = t.boot(&closed_snapshot)?;
        t.operation(&world, number)?;
        let before = t.disk(&world);
        check!(
            t.prepare(
                &mut world,
                T::DRAFT,
                t.renewer(),
                number + 3,
                licence_params(t)
            )
            .is_err(),
            "same predecessor requires explicit new consent/retry link"
        );
        check!(
            t.prepare(
                &mut world,
                T::DRAFT,
                t.renewer(),
                number + 3,
                json!({"previous_operation":token(number+99)})
            )
            .is_err()
        );
        same_disk(t, &world, &before)?;
        t.step(
            &mut world,
            T::DRAFT,
            number + 3,
            json!({"previous_operation":token(number)}),
        )?;
        let retry = t.operation(&world, number + 3)?;
        check_eq!(
            retry.attempt,
            RenewalAttemptV1::BuyerRetry {
                previous_operation: token(number)
            }
        );
        check_ne!(retry.contract, closed.contract);
        check_eq!(retry.binding, closed.binding);
        check_ne!(retry.consent, closed.consent);
        step_for(t, &mut world, T::FUND, number + 4, number + 3)?;
        step_for(t, &mut world, T::GRANT, number + 5, number + 3)?;
        t.operation(&world, number)?;
        t.operation(&world, number + 3)?;
        check_eq!(
            t.term_at(&world, retry.next)?.get("operation"),
            Some(&json!(token(number + 3)))
        );
        check_eq!(
            t.economics_at(&world, retry.next)?.get("operation"),
            Some(&json!(token(number + 3)))
        );
        check_eq!(t.licence_operation(&world)?, t.birth_operation());
        // Old Closed/Refunded rows coexist lawfully with the linked retry's
        // successful next generation. No row is attributed to the old attempt.
        let complete = t.capture(&world);
        let cold = t.boot(&complete)?;
        t.operation(&cold, number)?;
        t.operation(&cold, number + 3)?;
        check_eq!(t.balances(&cold)?, t.balances(&world)?);
        check_eq!(t.head(&cold)?.1, token(number + 3));
    }
    Ok(())
}

/// The charter's title cannot move under a prepared or a staged renewal step, and the step
/// writes nothing while it is refused.
pub(super) fn title_fences<T: Twin>(t: &T, start: u128) -> TestResult {
    for (offset, id) in [(0, T::DRAFT), (10, T::FUND), (20, T::GRANT)] {
        for after_stage in [false, true] {
            let number = start + 200 + offset;
            let mut world = t.due_world()?;
            if id != T::DRAFT {
                t.step(&mut world, T::DRAFT, number, licence_params(t))?;
            }
            if id == T::GRANT {
                step_for(t, &mut world, T::FUND, number + 1, number)?;
            }
            let params = if id == T::DRAFT {
                licence_params(t)
            } else {
                operation_params(number)
            };
            let number_for_action = number + 2;
            let before = t.balances(&world)?;
            let old = t.current(&world)?;
            let held = t.title_owner(&world)?;
            let prepared =
                t.prepare(&mut world, id, t.renewer(), number_for_action, params.clone())?;
            // Core admits one pending commit: the issuer's title cannot move
            // under a prepared (or staged) renewal step, so the transfer is
            // refused and the renewal writes nothing.
            if after_stage {
                t.stage(
                    &mut world,
                    id,
                    t.renewer(),
                    number_for_action,
                    params.clone(),
                    prepared,
                )?;
                check!(
                    world
                        .transfer_charter(t.charter(), GOD, 49_000_130)
                        .is_err(),
                    "title moved under a staged renewal step"
                );
            } else {
                check!(
                    world
                        .transfer_charter(t.charter(), GOD, 49_000_140)
                        .is_err(),
                    "title moved under a prepared renewal step"
                );
                drop((params, prepared));
            }
            check_eq!(t.title_owner(&world)?, held);
            check_eq!(t.current(&world)?, old);
            check_eq!(t.balances(&world)?, before);
            check!(rows(&t.disk(&world), "market", T::RECEIPT).is_empty());
            if id == T::DRAFT {
                check!(rows(&t.disk(&world), "roll", T::ROLL_OP).is_empty());
            } else {
                check!(t.operation(&world, number)?.pending);
            }
        }
    }
    Ok(())
}

/// A staged draft cannot publish before the predecessor's term end; a staged step is
/// refused when the seller's and the renewer's login accounts have become one (the personal
/// issuer); a staged fund or grant is refused at the offer's exact expiry.
pub(super) fn accounts_and_expiry<T: Twin>(t: &T, start: u128) -> TestResult {
    {
        let mut world = t.due_world()?;
        let due = t.term_end(&world)?;
        let before = t.disk(&world);
        let money = t.balances(&world)?;
        let params = licence_params(t);
        let prepared = t.prepare(&mut world, T::DRAFT, t.renewer(), start + 249, params.clone())?;
        t.stage(
            &mut world,
            T::DRAFT,
            t.renewer(),
            start + 249,
            params,
            prepared,
        )?;
        t.set_time(&mut world, due.checked_sub(1).ok_or("due boundary")?)?;
        check!(
            world.engine.take_staged_action().is_err(),
            "captured due eligibility cannot publish before predecessor term_end"
        );
        same_disk(t, &world, &before)?;
        check_eq!(t.balances(&world)?, money);
    }
    if T::LOGIN_PAIR {
        for (offset, id) in [(0, T::DRAFT), (10, T::FUND), (20, T::GRANT)] {
            let number = start + 250 + offset;
            let mut world = t.due_world()?;
            if id != T::DRAFT {
                t.step(&mut world, T::DRAFT, number, licence_params(t))?;
            }
            if id == T::GRANT {
                step_for(t, &mut world, T::FUND, number + 1, number)?;
            }
            let params = if id == T::DRAFT {
                licence_params(t)
            } else {
                operation_params(number)
            };
            let before = t.disk(&world);
            let money = t.balances(&world)?;
            let prepared = t.prepare(&mut world, id, t.renewer(), number + 2, params.clone())?;
            let seller = t.merge_login_accounts()?;
            let refusal = t
                .stage(&mut world, id, t.renewer(), number + 2, params, prepared)
                .is_err();
            t.restore_login_account(seller)?;
            check!(
                refusal,
                "retained actual login account pair must be rechecked"
            );
            same_disk(t, &world, &before)?;
            check_eq!(t.balances(&world)?, money);
        }
    }
    for (offset, id) in [(0, T::FUND), (10, T::GRANT)] {
        let number = start + 300 + offset;
        let mut world = t.due_world()?;
        t.step(&mut world, T::DRAFT, number, licence_params(t))?;
        if id == T::GRANT {
            step_for(t, &mut world, T::FUND, number + 1, number)?;
        }
        let expires = t.operation(&world, number)?.expires_at;
        let params = operation_params(number);
        let before = t.disk(&world);
        let money = t.balances(&world)?;
        let prepared = t.prepare(&mut world, id, t.renewer(), number + 2, params.clone())?;
        t.stage(&mut world, id, t.renewer(), number + 2, params, prepared)?;
        t.set_time(&mut world, expires)?;
        check!(
            world.engine.take_staged_action().is_err(),
            "exact offer expiry at custody refuses fresh money/grant"
        );
        same_disk(t, &world, &before)?;
        check_eq!(t.balances(&world)?, money);
    }
    Ok(())
}

/// A renewed store that lost part of its history, or whose licence was crossed with another
/// generation or operation, refuses to boot. `renewed_by` is the operation that renewed the
/// licence to generation 2 in `snapshot`.
pub(super) fn corruptions<T: Twin>(t: &T, snapshot: &Durable, renewed_by: u128) -> TestResult {
    for (owner, family) in [
        ("market", T::MARKET_OP),
        ("market", T::RECEIPT),
        ("roll", T::ROLL_OP),
        ("roll", T::HEAD),
        ("roll", T::ECONOMICS),
    ] {
        let mut lost = snapshot.clone();
        let count = lost.disk.len();
        lost.disk
            .retain(|(ns, _), v| !(ns == owner && v.pointer("/family/id") == Some(&json!(family))));
        check!(lost.disk.len() < count, "removed actual persisted family");
        check!(
            crate::test_checks::refused(t.boot(&lost))?,
            "missing or all-missing {family} must refuse"
        );
    }
    // One side absent while E4/roots/head survive cannot be mistaken for zero jobs.
    let mut lost = snapshot.clone();
    let count = lost.disk.len();
    lost.disk.retain(|_, v| {
        ![T::ROLL_OP, T::HEAD, T::MARKET_OP, T::RECEIPT]
            .iter()
            .any(|name| v.pointer("/family/id") == Some(&json!(name)))
    });
    check!(lost.disk.len() < count);
    check!(crate::test_checks::refused(t.boot(&lost))?);
    let mut missing_economy = snapshot.clone();
    let before = missing_economy.disk.len();
    missing_economy
        .disk
        .retain(|(ns, key), _| !(ns == "economy" && key.starts_with(T::ECONOMY_OFFER)));
    check!(
        missing_economy.disk.len() < before,
        "removed the actual E4 offer history"
    );
    check!(
        crate::test_checks::refused(t.boot(&missing_economy))?,
        "receipt/contract alone cannot replace missing E4 origins"
    );
    let mut missing_current_term = snapshot.clone();
    let before = missing_current_term.disk.len();
    missing_current_term.disk.retain(|(ns, _), value| {
        !(ns == "roll"
            && value.pointer("/family/id") == Some(&json!("roll_mind_paid_term"))
            && value.pointer("/value/licence") == Some(&json!(t.licence()))
            && value.pointer("/value/generation") == Some(&json!("2")))
    });
    check!(missing_current_term.disk.len() < before);
    check!(
        crate::test_checks::refused(t.boot(&missing_current_term))?,
        "current pointer cannot substitute for its term"
    );
    // Exact current pointer and retained immutable birth are independently joined.
    for field in ["term", "operation"] {
        let mut crossed = snapshot.clone();
        let mut changed = false;
        for ((ns, _), v) in &mut crossed.disk {
            if ns == "roll"
                && v.pointer("/family/id") == Some(&json!("roll_mind_personal_paid_licence"))
                && v.pointer("/value/id") == Some(&json!(t.licence()))
            {
                let value = v
                    .get_mut("value")
                    .and_then(Value::as_object_mut)
                    .ok_or("licence document")?;
                let slot = value.get_mut(field).ok_or("licence field")?;
                *slot = if field == "term" {
                    json!("1")
                } else {
                    json!(token(renewed_by))
                };
                changed = true;
            }
        }
        check!(changed);
        check!(
            crate::test_checks::refused(t.boot(&crossed))?,
            "crossed {field}"
        );
    }
    Ok(())
}

/// A store whose Economy holds a renewal's history (a draft's offered contract and its
/// offer) while the Roll and the market hold none of it boots only with the renewals
/// composed: a market that composes none refuses Economy's history on its own. (CT.6's
/// F22; the Roll's half of the check, CT.5's R19, sees no Roll history in this store.)
pub(super) fn economy_history_alone<T: Twin>(t: &T, start: u128) -> TestResult {
    let mut world = t.due_world()?;
    let due = t.capture(&world);
    check!(
        t.boot_disabled(&due).is_ok(),
        "the due store boots without the renewals"
    );
    let (prepared, _) = t.prepare(
        &mut world,
        T::DRAFT,
        t.renewer(),
        start + 400,
        licence_params(t),
    )?;
    let mut economy_only = due;
    let mut added = 0_usize;
    for write in prepared.authoritative_replay_writes() {
        if let ReplayWrite::ModuleJsonSet {
            namespace,
            key,
            value,
        } = write
            && namespace.as_str() == "economy"
        {
            economy_only
                .disk
                .insert((namespace.to_string(), key.to_string()), value);
            added += 1;
        }
    }
    check!(added > 0, "the draft writes Economy's renewal history");
    check!(
        crate::test_checks::refused(t.boot_disabled(&economy_only))?,
        "a market that composes no renewals refuses Economy's renewal history"
    );
    Ok(())
}
