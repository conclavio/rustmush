//! **Doc 41 cut RS.5** — the retrieval contract (Q-41-8) and the one-term debt (Q-41-13), driven
//! through a stand-in ruleset's commands over the real dispatch.
//!
//! - A retrieval is posted with its fee escrowed from the poster, joined by one acceptor (never
//!   the poster, never a second), and settles to the acceptor when its delivery fact holds; a
//!   cancelled one refunds the poster.
//! - A debt is due now; unpaid at its first sweep it is `Delinquent` and out of the due pool;
//!   `reinstate_debt` makes it due again (a reinstated one-shot is not due "never"), and
//!   `pay_debt` settles it in one post, refusing an account that cannot cover it.
use std::sync::atomic::{AtomicBool, Ordering};

use super::*;

/// Whether the delivery fact holds (one test at a time touches it: the file's own tests run
/// under nextest, one process each).
static DELIVERED: AtomicBool = AtomicBool::new(false);

/// The host's delivery fact, toggled by the test.
#[derive(Debug)]
struct Delivery;

impl FactProvider for Delivery {
    fn provider(&self) -> &str {
        "rs5"
    }
    fn resolve(
        &self,
        _w: &World,
        kind: &str,
        _id: &str,
        _revision: Option<u64>,
    ) -> Option<FactState> {
        (kind == "rs5.delivered" && DELIVERED.load(Ordering::SeqCst)).then_some(FactState {
            satisfied: true,
            actor: None,
            at: 0,
        })
    }
}

/// The ruleset standing in for tacsim: each command one seam.
struct RetrievalRuleset;

/// A dbref argument.
fn obj(word: &str) -> Result<ObjId, CmdError> {
    word.trim()
        .trim_start_matches('#')
        .parse()
        .map(ObjId)
        .map_err(|_| CmdError::msg("bad dbref"))
}

/// A number argument.
fn num(word: Option<&str>) -> Result<u64, CmdError> {
    word.unwrap_or("")
        .trim()
        .parse()
        .map_err(|_| CmdError::msg("bad number"))
}

impl GameModule for RetrievalRuleset {
    fn name(&self) -> &'static str {
        "rs5testruleset"
    }
    #[allow(clippy::too_many_lines)] // one command per seam
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::EconomyCarveOut);
        // `@rpost <hull>=<account> <fee>`: the actor posts.
        reg.command(
            CommandSpec {
                name: "@rpost".into(),
                shape: ArgShape::EqPair,
                help: "test".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let hull = obj(ctx.args.lhs())?;
                let rhs = ctx.args.rhs().unwrap_or_default().to_owned();
                let mut words = rhs.split_whitespace();
                let account = AccountId(num(words.next())?);
                let fee = i64::try_from(num(words.next())?).unwrap_or(i64::MAX);
                let deadline_ic = ctx.w.world_ic_now().at + 56 * 86_400;
                let id = mush_economy::post_retrieval(
                    ctx,
                    mush_economy::RetrievalSpec {
                        name: "Retrieval".into(),
                        poster: PrincipalId::Object(ctx.actor),
                        title_holder: None,
                        poster_account: account,
                        hull,
                        fee: mush_economy::Amount::from_minor(fee),
                        delivered: mush_economy::ExternalRef {
                            provider: "rs5".into(),
                            kind: "rs5.delivered".into(),
                            id: hull.0.to_string().into(),
                            revision: Some(1),
                        },
                        deadline_ic,
                    },
                )
                .map_err(|e| CmdError::msg(e.to_string()))?;
                ctx.out
                    .sys_to_actor(ctx.actor, "test", format!("posted {}", id.0));
                Ok(())
            }),
        )?;
        // `@raccept <contract>=<account>`: the actor accepts.
        reg.command(
            CommandSpec {
                name: "@raccept".into(),
                shape: ArgShape::EqPair,
                help: "test".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let id = ContractId(num(Some(ctx.args.lhs()))?);
                let account = AccountId(num(ctx.args.rhs())?);
                mush_economy::accept_retrieval(ctx, id, &PrincipalId::Object(ctx.actor), account)
                    .map_err(|e| CmdError::msg(e.to_string()))
            }),
        )?;
        // `@rcancel <contract>`.
        reg.command(
            CommandSpec {
                name: "@rcancel".into(),
                shape: ArgShape::Raw,
                help: "test".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let id = ContractId(num(Some(ctx.args.raw()))?);
                mush_economy::cancel_retrieval(ctx, id)
                    .map(|_| ())
                    .map_err(|e| CmdError::msg(e.to_string()))
            }),
        )?;
        // `@debt <debtor account>=<creditor account> <amount>`.
        reg.command(
            CommandSpec {
                name: "@debt".into(),
                shape: ArgShape::EqPair,
                help: "test".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let debit = AccountId(num(Some(ctx.args.lhs()))?);
                let rhs = ctx.args.rhs().unwrap_or_default().to_owned();
                let mut words = rhs.split_whitespace();
                let credit = AccountId(num(words.next())?);
                let amount = i64::try_from(num(words.next())?).unwrap_or(i64::MAX);
                let id = mush_economy::create_debt(
                    ctx,
                    debit,
                    credit,
                    mush_economy::Amount::from_minor(amount),
                    "rs5.debt",
                )
                .map_err(|e| CmdError::msg(e.to_string()))?;
                ctx.out
                    .sys_to_actor(ctx.actor, "test", format!("debt {}", id.0));
                Ok(())
            }),
        )?;
        // `@debtpay <id>` and `@debtback <id>`.
        reg.command(
            CommandSpec {
                name: "@debtpay".into(),
                shape: ArgShape::Raw,
                help: "test".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let id = mush_economy::ObligationId(num(Some(ctx.args.raw()))?);
                mush_economy::pay_debt(ctx, id, "rs5.debt")
                    .map(|_| ())
                    .map_err(|e| CmdError::msg(e.to_string()))
            }),
        )?;
        reg.command(
            CommandSpec {
                name: "@debtback".into(),
                shape: ArgShape::Raw,
                help: "test".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let id = mush_economy::ObligationId(num(Some(ctx.args.raw()))?);
                mush_economy::reinstate_debt(ctx, id, "rs5.debt")
                    .map(|_| ())
                    .map_err(|e| CmdError::msg(e.to_string()))
            }),
        )?;
        Ok(())
    }
}

/// The world: the economy with the delivery provider and a title issuer, and the ruleset.
fn boot() -> TestWorld {
    TestWorld::with_modules(vec![
        Box::new(
            mush_org::OrgModule::new()
                .with_capabilities(vocabulary())
                .with_scope_use(vec![Arc::new(mush_economy::EconomyScopeUse)]),
        ),
        Box::new(
            mush_economy::EconomyModule::with_custody(vec![])
                .with_facts(vec![Arc::new(Delivery)])
                .with_title_issuers(vec![SystemPrincipalId::new("wod.housing").expect("valid")]),
        ),
        Box::new(TitleIssuingRuleset),
        Box::new(RetrievalRuleset),
    ])
}

/// The number at the end of a reply (`posted 4`).
fn tail_number(reply: &str) -> u64 {
    reply
        .split_whitespace()
        .last()
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("no number in {reply:?}"))
}

#[test]
fn rs5_a_retrieval_escrows_at_posting_takes_one_acceptor_and_pays_on_delivery() {
    DELIVERED.store(false, Ordering::SeqCst);
    let mut tw = boot();
    let r = run_rig(&mut tw, 10_000);
    let hull = thing(&mut tw, "Prospector");
    ok(
        &mut tw,
        GOD,
        &format!("@titleissue #{}=wod.housing #{}", hull.0, r.johnson.0),
    );
    // The fee escrowed at posting, from the poster's own account.
    let id = ContractId(tail_number(&ok(
        &mut tw,
        r.johnson,
        &format!("@rpost #{}={} 5250", hull.0, r.j_acct.0),
    )));
    let c = snap(&tw).contract(id).expect("posted").clone();
    assert_eq!(mush_economy::retrieval_contract_hull(&c), Some(hull));
    assert!(mush_economy::retrieval_acceptor(&c).is_none());
    assert_eq!(c.state, ContractState::Active, "in force at once");
    assert_eq!(
        snap(&tw)
            .account(r.j_acct)
            .expect("acct")
            .held_amount
            .minor(),
        5_250,
        "the fee is held"
    );
    // A stranger who does not own the account named, and the poster themself, are refused.
    err(
        &mut tw,
        r.runner,
        &format!("@raccept {}={}", id.0, r.j_acct.0),
    );
    err(
        &mut tw,
        r.johnson,
        &format!("@raccept {}={}", id.0, r.j_acct.0),
    );
    // The runner accepts; a second acceptor is refused.
    ok(
        &mut tw,
        r.runner,
        &format!("@raccept {}={}", id.0, r.r_acct.0),
    );
    let watcher_acct = account_of(&tw, r.watcher);
    err(
        &mut tw,
        r.watcher,
        &format!("@raccept {}={}", id.0, watcher_acct.0),
    );
    let c = snap(&tw).contract(id).expect("accepted").clone();
    assert_eq!(
        mush_economy::retrieval_acceptor(&c),
        Some(&PrincipalId::Object(r.runner))
    );
    assert!(c.fully_accepted(), "both bound to the joined consent");
    // No delivery, no pay.
    let now = tw.world().now();
    sweep_at(&mut tw, now + 120);
    assert_eq!(state(&tw, id), ContractState::Active);
    assert_eq!(bal(&tw, r.r_acct), 0);
    // Delivered: the sweep settles to the acceptor.
    DELIVERED.store(true, Ordering::SeqCst);
    sweep_at(&mut tw, now + 240);
    assert!(
        matches!(state(&tw, id), ContractState::Settled { .. }),
        "{:?}",
        state(&tw, id)
    );
    assert_eq!(bal(&tw, r.r_acct), 5_250, "the acceptor is paid the fee");
    assert_eq!(bal(&tw, r.j_acct), 10_000 - 5_250);
    DELIVERED.store(false, Ordering::SeqCst);
}

#[test]
fn rs5_a_cancelled_retrieval_refunds_the_poster_and_a_poor_poster_posts_nothing() {
    DELIVERED.store(false, Ordering::SeqCst);
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let hull = thing(&mut tw, "Prospector");
    ok(
        &mut tw,
        GOD,
        &format!("@titleissue #{}=wod.housing #{}", hull.0, r.johnson.0),
    );
    // Short of the fee: nothing stands.
    err(
        &mut tw,
        r.johnson,
        &format!("@rpost #{}={} 5250", hull.0, r.j_acct.0),
    );
    assert!(
        snap(&tw).contracts().all(|c| c.state.is_terminal()),
        "no live retrieval"
    );
    assert_eq!(
        snap(&tw).account(r.j_acct).expect("a").held_amount.minor(),
        0
    );
    // Not the title holder: refused.
    err(
        &mut tw,
        r.runner,
        &format!("@rpost #{}={} 100", hull.0, r.r_acct.0),
    );
    let id = ContractId(tail_number(&ok(
        &mut tw,
        r.johnson,
        &format!("@rpost #{}={} 2000", hull.0, r.j_acct.0),
    )));
    ok(
        &mut tw,
        r.runner,
        &format!("@raccept {}={}", id.0, r.r_acct.0),
    );
    ok(&mut tw, GOD, &format!("@rcancel {}", id.0));
    assert_eq!(state(&tw, id), ContractState::Cancelled);
    let now = tw.world().now();
    sweep_at(&mut tw, now + 120);
    assert_eq!(
        snap(&tw).account(r.j_acct).expect("a").held_amount.minor(),
        0
    );
    assert_eq!(bal(&tw, r.j_acct), 3_000, "the escrow came back");
    assert_eq!(bal(&tw, r.r_acct), 0);
}

#[test]
fn rs5_a_debt_goes_delinquent_unpaid_is_reinstated_due_now_and_paid_in_one_post() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 0);
    // The runner owes Johnson 700, opened by the runner's own act (their own debit authority).
    let id = tail_number(&ok(
        &mut tw,
        r.runner,
        &format!("@debt {}={} 700", r.r_acct.0, r.j_acct.0),
    ));
    let debtor = PrincipalId::Object(r.runner);
    assert_eq!(
        mush_economy::debts_of(tw.world(), &debtor, &["rs5.debt"]).len(),
        1
    );
    // A stranger cannot open a debt on another's wallet.
    err(
        &mut tw,
        r.watcher,
        &format!("@debt {}={} 700", r.r_acct.0, r.j_acct.0),
    );
    // The first sweep finds nothing to take: a one-shot goes straight to Delinquent.
    let now = tw.world().now();
    sweep_at(&mut tw, now + 120);
    let o = snap(&tw)
        .obligation(mush_economy::ObligationId(id))
        .expect("the debt")
        .clone();
    assert_eq!(o.status, mush_economy::ObligationStatus::Delinquent);
    // Paying while short refuses with nothing written.
    err(&mut tw, r.runner, &format!("@debtpay {id}"));
    // Reinstated, it is due again now (not "never"), and the next sweep tries it once more.
    ok(&mut tw, r.runner, &format!("@debtback {id}"));
    let o = snap(&tw)
        .obligation(mush_economy::ObligationId(id))
        .expect("the debt")
        .clone();
    assert_eq!(o.status, mush_economy::ObligationStatus::Active);
    assert!(o.schedule.next_due <= tw.world().world_ic_now().at);
    // Funded, it is paid in one post.
    let faucet = snap(&tw)
        .accounts()
        .find(|x| matches!(x.owner, PrincipalId::System(_)))
        .expect("faucet")
        .id;
    ok(
        &mut tw,
        GOD,
        &format!("@econ/mint {faucet}={} 1000", r.r_acct.0),
    );
    ok(&mut tw, r.runner, &format!("@debtpay {id}"));
    let o = snap(&tw)
        .obligation(mush_economy::ObligationId(id))
        .expect("the debt")
        .clone();
    assert_eq!(o.status, mush_economy::ObligationStatus::Satisfied);
    assert_eq!(bal(&tw, r.r_acct), 300);
    assert_eq!(bal(&tw, r.j_acct), 700);
    assert!(mush_economy::debts_of(tw.world(), &debtor, &["rs5.debt"]).is_empty());
    // Paid once: a second pay is refused.
    err(&mut tw, r.runner, &format!("@debtpay {id}"));
}
