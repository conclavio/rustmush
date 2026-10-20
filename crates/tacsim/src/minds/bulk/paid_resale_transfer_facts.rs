#![cfg(test)]

//! Actual transfer facts over the real signed resale and IC fixture. The probe
//! only invokes Economy's public scoped read in its actual command dispatch;
//! it cannot construct scope, consent, receipts, or a satisfied fact.
use super::*;
use mush_economy::{ContractId, ContractState, EconomicContract, ExternalRef, FactState};
const N: u128 = 64_000_000;
const THIRD_CONN: u64 = 933;
#[derive(Default)]
pub(crate) struct Selection {
    input: Option<(ContractId, ExternalRef)>,
    output: Option<Option<FactState>>,
}
pub(crate) type Calls = Arc<Mutex<Selection>>;
struct EconomyRead {
    inner: EconomyModule,
    calls: Calls,
}
pub(crate) fn economy(inner: EconomyModule, calls: Option<Calls>) -> Box<dyn GameModule> {
    match calls {
        None => Box::new(inner),
        Some(calls) => Box::new(EconomyRead { inner, calls }),
    }
}
impl GameModule for EconomyRead {
    fn name(&self) -> &'static str {
        "economy"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        self.inner.register(reg)?;
        reg.classify_commands(mush_core::CommandCompatibility::ReadOnly);
        let calls = self.calls.clone();
        reg.command(
            mush_core::CommandSpec {
                name: "@transfer-fact-fixture".into(),
                shape: mush_core::ArgShape::Raw,
                ..Default::default()
            },
            Arc::new(move |ctx: &mut mush_core::Ctx<'_>| {
                let mut calls = calls
                    .lock()
                    .map_err(|_| mush_core::CmdError::msg("probe lock"))?;
                let (id, fact) = calls
                    .input
                    .as_ref()
                    .ok_or_else(|| mush_core::CmdError::msg("probe input"))?;
                calls.output = Some(mush_economy::contract_fact(ctx, *id, fact));
                Ok(())
            }),
        )
    }
    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        self.inner.on_world_loaded(ctx, data)
    }
    fn on_all_loaded(&mut self, ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        self.inner.on_all_loaded(ctx)
    }
    fn finalize_owner_composition(&mut self, ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        self.inner.finalize_owner_composition(ctx)
    }
    fn boot_stream_requests(
        &self,
        kv: &[(smol_str::SmolStr, Value)],
    ) -> Result<Vec<(smol_str::SmolStr, u32)>, ModError> {
        self.inner.boot_stream_requests(kv)
    }
}
fn dispatch(w: &mut IcWorld, actor: ObjId, line: &str) -> TestResult<Outbox> {
    let conn = if actor == GOD {
        CONN
    } else if actor == BUYER {
        BUYER_CONN
    } else {
        THIRD_CONN
    };
    let mut out = Outbox::new();
    w.engine.step(
        Inbound::Cmd {
            conn,
            actor,
            line: line.into(),
        },
        &mut out,
        &mut mush_core::Effects::new(),
    );
    Ok(out)
}
fn say(w: &mut IcWorld, actor: ObjId, line: &str) -> TestResult<Outbox> {
    let out = dispatch(w, actor, line)?;
    w.persist_setup()?;
    Ok(out)
}
fn fact(acceptance: u128, generation: u64) -> ExternalRef {
    ExternalRef {
        provider: "minds".into(),
        kind: "licence.transferred".into(),
        id: token(acceptance).to_hex().into(),
        revision: Some(generation),
    }
}
fn read(
    w: &mut IcWorld,
    actor: ObjId,
    id: ContractId,
    fact: ExternalRef,
) -> TestResult<Option<FactState>> {
    let calls = w.transfer_facts.clone().ok_or("actual fact probe")?;
    {
        let mut value = calls.lock().map_err(module_error)?;
        value.input = Some((id, fact));
        value.output = None;
    }
    dispatch(w, actor, "@transfer-fact-fixture")?;
    let answer = calls
        .lock()
        .map_err(module_error)?
        .output
        .take()
        .ok_or("actual probe dispatch")?;
    Ok(answer)
}
fn contract(w: &IcWorld, id: ContractId) -> TestResult<EconomicContract> {
    w.engine
        .world
        .module_state::<EconomyStore>()
        .and_then(|e| e.snapshot().contract(id))
        .cloned()
        .ok_or_else(|| "contract".into())
}
fn draft(w: &mut IcWorld, actor: ObjId, name: &str) -> TestResult<ContractId> {
    say(w, actor, &format!("@contract/draft {name}"))?;
    w.engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("Economy")?
        .snapshot()
        .contracts()
        .map(|c| c.id)
        .max()
        .ok_or_else(|| "draft".into())
}
fn attach(
    w: &mut IcWorld,
    actor: ObjId,
    id: ContractId,
    acceptance: u128,
    generation: u64,
) -> TestResult<Outbox> {
    say(
        w,
        actor,
        &format!(
            "@contract/condition {id}=fact minds licence.transferred {} revision={generation}",
            token(acceptance).to_hex()
        ),
    )
}
fn branch(f: &PurchaseFixture, template: &IcWorld, saved: &Durable) -> TestResult<IcWorld> {
    let mut w = IcWorld::boot_source_facts(
        f.into(),
        saved,
        template.base,
        template.field,
        template.hulls,
        true,
        false,
        true,
        None,
        None,
        false,
        Some(Arc::new(Mutex::new(Selection::default()))),
    )?;
    w.engine.set_submission_runtime(Arc::new(Runtime))?;
    let faucet = w
        .engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("Economy")?
        .snapshot()
        .accounts()
        .find(|a| {
            a.owner
                == EconomicOwner::System(
                    SystemPrincipalId::new("purchase.fixture").expect("fixed system"),
                )
        })
        .ok_or("actual funding system")?
        .id;
    w.command(&format!("@econ/mint {faucet}=0 1000"))?;
    Ok(w)
}
fn conditional(w: &mut IcWorld, acceptance: u128) -> TestResult<ContractId> {
    let id = draft(w, BUYER, "Transfer committed condition")?;
    for line in [
        format!("@contract/party {id}=#{}", GOD.0),
        format!("@contract/pay {id}=2 1"),
        format!("@contract/split {id}=0 1"),
        format!("@contract/remainder {id}=2"),
    ] {
        say(w, BUYER, &line)?;
    }
    attach(w, BUYER, id, acceptance, 2)?;
    let c = contract(w, id)?;
    assert_eq!(c.consent.conditions.len(), 1);
    assert!(
        c.consent.fact_consent.is_some(),
        "real seller attachment marker"
    );
    for (actor, verb) in [(BUYER, "offer"), (BUYER, "accept"), (GOD, "accept")] {
        say(w, actor, &format!("@contract/{verb} {id}"))?;
    }
    assert_eq!(contract(w, id)?.state, ContractState::Active);
    Ok(id)
}
fn settle_contract(w: &mut IcWorld, id: ContractId) -> TestResult {
    for _ in 0..8 {
        if matches!(contract(w, id)?.state, ContractState::Settled { .. }) {
            return Ok(());
        }
        let next = w.engine.world.now().checked_add(1).ok_or("clock")?;
        w.engine.world.set_now(next);
        pulse(w)?;
    }
    Err("real transfer condition did not settle escrow".into())
}
struct Login<'a> {
    accounts: &'a crate::minds::bulk::registry_integration_tests::Accounts,
    actor: ObjId,
    previous: Option<AccountId>,
}
impl Drop for Login<'_> {
    fn drop(&mut self) {
        if let Ok(mut values) = self.accounts.values.lock() {
            self.accounts.epoch.invalidate();
            if let Some(previous) = self.previous {
                values.insert(self.actor, previous);
            } else {
                values.remove(&self.actor);
            }
        }
    }
}
fn third_holder<'a>(
    f: &'a PurchaseFixture,
    w: &mut IcWorld,
) -> TestResult<(ObjId, WalletId, Login<'a>)> {
    let actor =
        w.engine
            .world
            .create(ObjKind::Player, "Later unrelated holder", GOD, Some(ROOM))?;
    let account_identity = AccountId::mint("transfer-facts-later-holder", i64::from(actor.0))?;
    let login = {
        let mut values = f.accounts.values.lock().map_err(module_error)?;
        f.accounts.epoch.invalidate();
        Login {
            accounts: &f.accounts,
            actor,
            previous: values.insert(actor, account_identity),
        }
    };
    w.engine.step(
        Inbound::ConnUp {
            conn: THIRD_CONN,
            actor,
            caps_structured: false,
            from_linkdead: false,
        },
        &mut Outbox::new(),
        &mut mush_core::Effects::new(),
    );
    w.command(&format!("@account/open #{}=cr", actor.0))?;
    let account = w
        .engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("Economy")?
        .snapshot()
        .accounts()
        .find(|a| a.owner == EconomicOwner::Object(actor))
        .ok_or("actual new holder wallet")?
        .id;
    let faucet = w
        .engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("Economy")?
        .snapshot()
        .accounts()
        .find(|a| {
            a.owner
                == EconomicOwner::System(
                    SystemPrincipalId::new("purchase.fixture").expect("system"),
                )
        })
        .ok_or("funding system")?
        .id;
    w.command(&format!("@econ/mint {faucet}={account} 1000"))?;
    Ok((actor, account, login))
}

pub(super) fn exercise(f: &PurchaseFixture, template: &IcWorld, saved: &Durable) -> TestResult {
    eprintln!("M7_STORY phase=transfer_facts event=begin");
    let mut w = branch(f, template, saved)?;
    let original = pointer(&w)?;
    let hull = w.hulls[0];
    let ticket = w.ticket(BUYER, hull, original.id().get())?;
    w.land(&ticket, N, hull, false)?;
    offer(&mut w, N + 1, BUYER, GOD, 2)?;
    buy(&mut w, N + 2, N + 1, FUND, GOD, 0)?;
    let id = conditional(&mut w, N + 2)?;
    assert_eq!(
        read(&mut w, BUYER, id, fact(N + 2, 2))?,
        None,
        "pending is not completion"
    );
    let funded = w.snapshot();
    // A real refusal/refund never satisfies the same authorized condition.
    let mut refund = w.reboot(f, &funded)?;
    refund.engine.set_submission_runtime(Arc::new(Runtime))?;
    refund.command("@account/approval 0=1 threshold:1")?;
    let (request, _) = next(&mut refund, REFUND)?;
    let prepared = terminal(&mut refund, &request, N + 3)?;
    land(&mut refund, prepared)?;
    assert_eq!(
        history(&refund, N + 2)?.outcome(),
        mush_economy::managed_paid_resale::PaidResaleOutcome::Refunded
    );
    assert_eq!(read(&mut refund, BUYER, id, fact(N + 2, 2))?, None);
    assert_eq!(contract(&refund, id)?.state, ContractState::Active);
    let mut refund = refund.reboot(f, &refund.snapshot())?;
    assert_eq!(read(&mut refund, BUYER, id, fact(N + 2, 2))?, None);
    drop(refund);

    let event = ic_sortie::fixture_close(&w.engine.world, BUYER, token(N), w.base)?;
    let close = w.host.enqueue_close(&event)?;
    w.land(&close, N + 4, hull, true)?;
    let (request, _) = next(&mut w, SETTLE)?;
    let prepared = terminal(&mut w, &request, N + 5)?;
    assert_eq!(
        read(&mut w, BUYER, id, fact(N + 2, 2))?,
        None,
        "preparation is not completion"
    );
    land(&mut w, prepared)?;
    let h = history(&w, N + 2)?;
    let decision = h
        .terminal()
        .ok_or("actual settled native receipt")?
        .decision();
    let expected = FactState {
        satisfied: true,
        actor: Some(decision.actor),
        at: decision.posted_at_utc,
    };
    assert_eq!(
        read(&mut w, BUYER, id, fact(N + 2, 2))?,
        Some(expected.clone())
    );
    assert_eq!(
        read(&mut w, GOD, id, fact(N + 2, 2))?,
        Some(expected.clone())
    );
    assert_eq!(
        read(&mut w, BUYER, id, fact(N + 2, 3))?,
        None,
        "wrong exact holder generation"
    );
    settle_contract(&mut w, id)?;
    assert_eq!(pointer(&w)?.holder(), GOD);
    let mut w = w.reboot(f, &w.snapshot())?;
    assert_eq!(
        read(&mut w, BUYER, id, fact(N + 2, 2))?,
        Some(expected.clone())
    );

    // The same real licence can change holder again; the old transaction still
    // belongs to its original parties, not to the new unrelated holder.
    let (later, payer, _login) = third_holder(f, &mut w)?;
    offer(&mut w, N + 10, GOD, later, 0)?;
    buy(&mut w, N + 11, N + 10, ACCEPT, later, payer.0)?;
    assert_eq!(pointer(&w)?.ownership().ok_or("ownership")?.generation, 3);
    assert_eq!(pointer(&w)?.holder(), later);
    assert_eq!(pointer(&w)?.term(), original.term());
    assert_eq!(
        read(&mut w, BUYER, id, fact(N + 2, 2))?,
        Some(expected.clone())
    );
    assert_eq!(read(&mut w, later, id, fact(N + 2, 2))?, None);
    let foreign = draft(&mut w, later, "Not my earlier transfer")?;
    say(
        &mut w,
        later,
        &format!("@contract/party {foreign}=#{}", GOD.0),
    )?;
    let before = contract(&w, foreign)?;
    attach(&mut w, later, foreign, N + 2, 2)?;
    assert_eq!(
        contract(&w, foreign)?,
        before,
        "current ownership never authorizes somebody else's historical transaction"
    );
    assert_eq!(
        read(&mut w, later, foreign, fact(N + 2, 2))?,
        None,
        "missing attachment marker"
    );

    // Lapse scope follows the real current ownership proof while the accepted
    // term's holder remains immutable. No event is fabricated by attachment.
    let lapse = draft(&mut w, later, "Current holder lapse scope")?;
    say(
        &mut w,
        later,
        &format!(
            "@contract/condition {lapse}=fact minds licence.lapsed {} revision={}",
            original.id().key(),
            original.term().get()
        ),
    )?;
    assert_eq!(contract(&w, lapse)?.consent.conditions.len(), 1);
    let former = draft(&mut w, BUYER, "Former holder lapse refusal")?;
    let old = contract(&w, former)?;
    say(
        &mut w,
        BUYER,
        &format!(
            "@contract/condition {former}=fact minds licence.lapsed {} revision={}",
            original.id().key(),
            original.term().get()
        ),
    )?;
    assert_eq!(contract(&w, former)?, old);

    // Actual immediate settlement uses acceptance UTC, not its term clock.
    attach(&mut w, later, foreign, N + 11, 3)?;
    assert_eq!(contract(&w, foreign)?.consent.conditions.len(), 1);
    for line in [
        format!("@contract/pay {foreign}={payer} 1"),
        format!("@contract/split {foreign}=0 1"),
        format!("@contract/remainder {foreign}={payer}"),
        format!("@contract/offer {foreign}"),
    ] {
        say(&mut w, later, &line)?;
    }
    assert_eq!(contract(&w, foreign)?.state, ContractState::Offered);
    let immediate = history(&w, N + 11)?;
    assert!(immediate.terminal().is_none());
    let now = FactState {
        satisfied: true,
        actor: Some(later),
        at: immediate.consent().accepted_at_utc,
    };
    assert_eq!(read(&mut w, later, foreign, fact(N + 11, 3))?, Some(now));
    let kept = w.snapshot();
    let mut cold = w.reboot(f, &kept)?;
    assert_eq!(read(&mut cold, BUYER, id, fact(N + 2, 2))?, Some(expected));
    cold.engine.registry.freeze_mechanics_owner(
        &OwnerId::new("market")?,
        mush_core::mechanics::IncidentId(64_001),
        mush_core::mechanics::RecoveryReason::IntegrityViolation {
            code: CanonicalId::new("transfer_fact_fixture")?,
        },
    )?;
    assert_eq!(
        read(&mut cold, BUYER, id, fact(N + 2, 2))?,
        None,
        "unavailable actual owner refuses"
    );
    // Paired boot still refuses a missing actual holder receipt.
    let mut missing = kept.clone();
    let before = missing.disk.len();
    missing.disk.retain(|(owner, _), v| {
        !(owner == "roll"
            && v.pointer("/family/id") == Some(&json!(HOLDER))
            && v.pointer("/value/generation") == Some(&json!(2)))
    });
    assert_eq!(missing.disk.len() + 1, before);
    assert!(w.reboot(f, &missing).is_err());
    eprintln!("M7_STORY phase=transfer_facts event=complete");
    Ok(())
}
