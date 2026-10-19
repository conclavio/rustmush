//! Actual contract consumers of an explicitly restricted test provider. This
//! fixture proves the Economy seam, not a production licence event or grant.
use super::*;
use mush_economy::{ExternalRef, FactPurpose, ScopedFactContext, WorldFacts};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Default)]
struct Restricted {
    holder: Mutex<Option<ObjId>>,
    enabled: AtomicBool,
    true_now: AtomicBool,
    disable_on_post: AtomicBool,
    exact_attachment: Mutex<Option<ContractId>>,
    seen: Mutex<Vec<(FactPurpose, ObjId, ContractId, [u8; 32])>>,
}
impl Restricted {
    fn reference() -> ExternalRef {
        ExternalRef {
            provider: "private_test".into(),
            kind: "term".into(),
            id: "private-proof-7".into(),
            revision: None,
        }
    }
}
impl FactProvider for Restricted {
    fn provider(&self) -> &str {
        "private_test"
    }
    fn requires_scope(&self, _: &ExternalRef) -> bool {
        true
    }
    // Deliberately unsafe legacy answer: the bridge MUST suppress it.
    fn resolve(&self, _: &World, _: &str, _: &str, _: Option<u64>) -> Option<FactState> {
        Some(FactState {
            satisfied: true,
            actor: Some(GOD),
            at: 71,
        })
    }
    fn authorize_scope(&self, ctx: &ScopedFactContext<'_>, fact: &ExternalRef) -> bool {
        let Some(holder) = *self.holder.lock().expect("holder") else {
            return false;
        };
        let s = ctx.scope();
        if s.purpose() == FactPurpose::Attach {
            assert!(
                s.attachment_actor(fact).is_none(),
                "fresh attachment never inherits old authorization"
            );
        }
        if !self.enabled.load(Ordering::SeqCst)
            || *fact != Self::reference()
            || s.issuer() != &PrincipalId::Object(holder)
            || !matches!(s.visibility(), mush_economy::EconomicVisibility::Parties)
        {
            return false;
        }
        let allowed = match s.purpose() {
            FactPurpose::Attach => {
                s.actor() == holder
                    && self
                        .exact_attachment
                        .lock()
                        .expect("exact ID")
                        .is_none_or(|id| id == s.contract())
            }
            FactPurpose::Read => s.parties().contains(&PrincipalId::Object(s.actor())),
            FactPurpose::Evaluate => true,
        };
        if allowed {
            self.seen.lock().expect("seen").push((
                s.purpose(),
                s.actor(),
                s.contract(),
                s.consent_hash(),
            ));
        }
        allowed
    }
    fn resolve_scoped(&self, _: &ScopedFactContext<'_>, _: &ExternalRef) -> Option<FactState> {
        Some(FactState {
            satisfied: self.true_now.load(Ordering::SeqCst),
            actor: None,
            at: 73,
        })
    }
}

fn rig() -> (TestWorld, Run, Arc<Restricted>) {
    let provider = Arc::new(Restricted::default());
    provider.enabled.store(true, Ordering::SeqCst);
    let mut tw = TestWorld::with_modules(scoped_modules(provider.clone(), Probe::default()));
    let r = run_rig(&mut tw, 1_000);
    *provider.holder.lock().expect("holder") = Some(r.johnson);
    (tw, r, provider)
}

#[test]
fn scoped_facts_foreign_and_missing_attachment_refuse_identically_without_a_write() {
    let (mut tw, r, _) = rig();
    ok(&mut tw, r.watcher, "@contract/draft Probe");
    let id = only_contract(&tw);
    let before = snap(&tw).contract(id).expect("draft").clone();
    let foreign = err(
        &mut tw,
        r.watcher,
        &format!("@contract/condition {id}=fact private_test term private-proof-7"),
    );
    let missing = err(
        &mut tw,
        r.watcher,
        &format!("@contract/condition {id}=fact private_test term absent"),
    );
    assert_eq!(
        foreign, missing,
        "missing and foreign references have one refusal"
    );
    assert!(
        snap(&tw).contract(id) == Some(&before),
        "refusal preserves the draft"
    );
}

#[test]
fn scoped_facts_actual_parties_read_but_staff_and_unscoped_calls_do_not() {
    let (mut tw, r, provider) = rig();
    let id = accepted_run(&mut tw, &r, 300, "fact private_test term private-proof-7");
    let unscoped = WorldFacts {
        w: tw.world(),
        providers: Arc::new(vec![provider.clone()]),
    };
    assert!(
        unscoped.state(&Restricted::reference()).is_none(),
        "legacy truth cannot bypass opt-in scope"
    );
    for actor in [r.johnson, r.runner] {
        let shown = ok(&mut tw, actor, &format!("@contract/show {id}"));
        assert!(
            shown.contains("private-proof-7"),
            "an actual party reads its authorized reference"
        );
    }
    let staff = ok(&mut tw, GOD, &format!("@contract/show {id}"));
    assert!(
        !staff.contains("private-proof-7"),
        "staff has no implicit subject disclosure grant"
    );
    assert!(staff.contains("restricted external fact"));
    assert!(!err(&mut tw, r.watcher, &format!("@contract/show {id}")).contains("private-proof-7"));
}

#[test]
fn scoped_facts_current_authority_and_parties_are_rechecked_before_offer_and_edit() {
    let (mut tw, r, provider) = rig();
    ok(&mut tw, r.johnson, "@contract/draft Own");
    let id = only_contract(&tw);
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/condition {id}=fact private_test term private-proof-7"),
    );
    let before = snap(&tw).contract(id).expect("draft").clone();
    provider.enabled.store(false, Ordering::SeqCst);
    err(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.runner.0),
    );
    err(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    assert!(
        snap(&tw).contract(id) == Some(&before),
        "no scope expansion after authority loss"
    );
    provider.enabled.store(true, Ordering::SeqCst);
    err(
        &mut tw,
        r.johnson,
        &format!("@contract/visibility {id}=public"),
    );
    assert!(
        snap(&tw).contract(id) == Some(&before),
        "a restricted fact cannot become public by editing visibility"
    );
}

#[test]
fn scoped_facts_sweep_uses_exact_consent_and_rechecks_authority_before_real_settlement() {
    let (mut tw, r, provider) = rig();
    let id = accepted_run(&mut tw, &r, 300, "fact private_test term private-proof-7");
    let hash = snap(&tw)
        .contract(id)
        .expect("actual contract")
        .consent_hash;
    provider.true_now.store(true, Ordering::SeqCst);
    provider.enabled.store(false, Ordering::SeqCst);
    sweep_at(&mut tw, 100);
    assert_eq!(state(&tw, id), ContractState::Active);
    assert_eq!(bal(&tw, r.r_acct), 0);
    provider.enabled.store(true, Ordering::SeqCst);
    sweep_at(&mut tw, 200);
    assert!(
        matches!(state(&tw, id), ContractState::Settled { .. }),
        "actual sweep settles after fresh scoped authorization"
    );
    assert_eq!(bal(&tw, r.r_acct), 300);
    let seen = provider.seen.lock().expect("seen");
    assert!(seen.iter().any(
        |(purpose, _, contract, digest)| *purpose == FactPurpose::Evaluate
            && *contract == id
            && *digest == hash
    ));
    assert!(
        seen.iter()
            .filter(|(purpose, _, _, _)| *purpose == FactPurpose::Evaluate)
            .all(|(_, _, contract, digest)| *contract == id && *digest == hash)
    );
}

#[derive(Clone, Default)]
struct Probe {
    request: Arc<Mutex<Option<ProbeRequest>>>,
    answer: Arc<Mutex<Option<Result<Option<ContractId>, ()>>>>,
}
enum ProbeRequest {
    Read {
        id: ContractId,
        spoof: ObjId,
    },
    Offer {
        spec: mush_economy::ContractSpec,
        spoof: Option<ObjId>,
    },
    Amend {
        id: ContractId,
        amend: mush_economy::ContractAmendment,
    },
}
impl GameModule for Probe {
    fn name(&self) -> &'static str {
        "scoped_probe"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::EconomyCarveOut);
        let state = self.clone();
        reg.command(
            CommandSpec {
                name: "@scopedprobe".into(),
                ..CommandSpec::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| {
                let request = state
                    .request
                    .lock()
                    .expect("request")
                    .take()
                    .expect("staged request");
                let actual = ctx.dispatch_actor();
                assert_eq!(actual, Some(ctx.actor));
                let actor = ctx.actor;
                let answer = match request {
                    ProbeRequest::Read { id, spoof } => {
                        ctx.actor = spoof;
                        assert_eq!(ctx.dispatch_actor(), actual);
                        mush_economy::contract_fact(ctx, id, &Restricted::reference())
                            .map(|_| Some(id))
                            .ok_or(())
                    }
                    ProbeRequest::Offer { spec, spoof } => {
                        if let Some(spoof) = spoof {
                            ctx.actor = spoof;
                        }
                        assert_eq!(ctx.dispatch_actor(), actual);
                        mush_economy::offer_contract(ctx, spec)
                            .map(Some)
                            .map_err(|_| ())
                    }
                    ProbeRequest::Amend { id, amend } => {
                        mush_economy::amend_contract(ctx, id, amend)
                            .map(|_| Some(id))
                            .map_err(|_| ())
                    }
                };
                ctx.actor = actor;
                *state.answer.lock().expect("answer") = Some(answer);
                Ok(())
            }),
        )
    }
}
#[derive(Debug)]
struct DisableOnPost(Arc<Restricted>);
impl EconomyObserver for DisableOnPost {
    fn on_economic_event(&self, _: &mut Ctx<'_>, event: &EconomicEvent) {
        if matches!(event, EconomicEvent::TransactionPosted { .. })
            && self.0.disable_on_post.load(Ordering::SeqCst)
        {
            self.0.enabled.store(false, Ordering::SeqCst);
        }
    }
}
fn scoped_modules(provider: Arc<Restricted>, probe: Probe) -> Vec<Box<dyn GameModule>> {
    vec![
        Box::new(
            mush_org::OrgModule::new()
                .with_capabilities(vocabulary())
                .with_scope_use(vec![Arc::new(mush_economy::EconomyScopeUse)]),
        ),
        Box::new(
            mush_economy::EconomyModule::with_custody(vec![])
                .with_facts(vec![provider.clone()])
                .with_observers(vec![Arc::new(DisableOnPost(provider))]),
        ),
        Box::new(TitleIssuingRuleset),
        Box::new(probe),
    ]
}
fn probe_rig() -> (TestWorld, Run, Arc<Restricted>, Probe) {
    let provider = Arc::new(Restricted::default());
    provider.enabled.store(true, Ordering::SeqCst);
    let probe = Probe::default();
    let mut tw = TestWorld::with_modules(scoped_modules(provider.clone(), probe.clone()));
    let r = run_rig(&mut tw, 1_000);
    *provider.holder.lock().expect("holder") = Some(r.johnson);
    (tw, r, provider, probe)
}
fn run_probe(
    tw: &mut TestWorld,
    probe: &Probe,
    actor: ObjId,
    request: ProbeRequest,
) -> Result<Option<ContractId>, ()> {
    *probe.request.lock().expect("request") = Some(request);
    let _ = tw.run(actor, "@scopedprobe");
    probe
        .answer
        .lock()
        .expect("answer")
        .take()
        .expect("actual handler entered")
}
fn spec(r: &Run) -> mush_economy::ContractSpec {
    use mush_economy::*;
    ContractSpec {
        name: "Private fact".into(),
        scope: None,
        parties: vec![PrincipalId::Object(r.runner)],
        considerations: vec![ConsiderationSpec {
            account: r.j_acct,
            amount: Amount::from_minor(300),
        }],
        outcomes: vec![OutcomeTerms {
            name: "done".into(),
            when: vec![0],
            payouts: vec![PayoutShare {
                to: r.r_acct,
                share: Share::Exact(Amount::from_minor(300)),
            }],
            expense_treatment: ExpenseTreatment::None,
            asset_consequences: vec![],
        }],
        remainder_to: r.j_acct,
        conditions: vec![TypedCondition::Fact {
            fact: Restricted::reference(),
        }],
        adjudicator: None,
        evidence: vec![],
        deadline: None,
        on_expiry: Expiry::Refund,
        visibility: EconomicVisibility::Parties,
        roles: ContractRoles::default(),
        deterministic: true,
        deliverables: vec![],
    }
}

#[test]
fn scoped_facts_immutable_dispatch_refuses_spoofed_read_and_programmatic_offer() {
    let (mut tw, r, _, probe) = probe_rig();
    let id = accepted_run(&mut tw, &r, 300, "fact private_test term private-proof-7");
    let before = snap(&tw).contracts().cloned().collect::<Vec<_>>();
    let next = store(&tw).peek_next_contract();
    assert!(
        run_probe(
            &mut tw,
            &probe,
            r.watcher,
            ProbeRequest::Read {
                id,
                spoof: r.johnson
            }
        )
        .is_err()
    );
    assert!(
        run_probe(
            &mut tw,
            &probe,
            r.watcher,
            ProbeRequest::Offer {
                spec: spec(&r),
                spoof: Some(r.johnson)
            }
        )
        .is_err()
    );
    assert!(
        snap(&tw).contracts().cloned().collect::<Vec<_>>() == before,
        "spoofed dispatch cannot read or publish a private fact"
    );
    assert_eq!(store(&tw).peek_next_contract(), next);
}

#[test]
fn scoped_facts_programmatic_offer_binds_actual_id_and_survives_real_cold_boot() {
    let (mut tw, r, provider, probe) = probe_rig();
    assert_eq!(store(&tw).peek_next_contract(), 0);
    *provider.exact_attachment.lock().expect("exact ID") = Some(ContractId(1));
    let before_contracts = snap(&tw).contracts().count();
    assert!(
        run_probe(
            &mut tw,
            &probe,
            r.johnson,
            ProbeRequest::Offer {
                spec: spec(&r),
                spoof: None
            },
        )
        .is_err()
    );
    assert_eq!(
        store(&tw).peek_next_contract(),
        0,
        "refusal cannot consume an ID"
    );
    assert_eq!(snap(&tw).contracts().count(), before_contracts);
    *provider.exact_attachment.lock().expect("exact ID") = Some(ContractId(0));
    let id = run_probe(
        &mut tw,
        &probe,
        r.johnson,
        ProbeRequest::Offer {
            spec: spec(&r),
            spoof: None,
        },
    )
    .expect("offer")
    .expect("id");
    let c = snap(&tw).contract(id).expect("actual offer");
    assert!(c.consent_intact());
    let marker = serde_json::to_value(c.consent.fact_consent.as_ref().expect("marker"))
        .expect("marker codec");
    assert_eq!(
        marker.get("contract"),
        Some(&serde_json::to_value(id).expect("id codec"))
    );
    assert_eq!(id, ContractId(0), "first actual allocated contract is zero");
    let attachments = provider
        .seen
        .lock()
        .expect("seen")
        .iter()
        .filter(|(purpose, _, _, _)| *purpose == FactPurpose::Attach)
        .map(|(_, _, contract, _)| *contract)
        .collect::<Vec<_>>();
    assert_eq!(
        attachments,
        vec![id],
        "one exact preallocation authorization"
    );
    let before = c.clone();
    let boot = mush_testkit::fold_module_ops(tw.take_module_ops());
    let mut tw = tw
        .reboot_into(scoped_modules(provider, probe), boot)
        .expect("actual cold boot");
    assert!(
        snap(&tw).contract(id) == Some(&before),
        "cold marker and consent are exact"
    );
    tw.connect(r.runner);
    let shown = ok(&mut tw, r.runner, &format!("@contract/show {id}"));
    assert!(shown.contains("private-proof-7"));
}

#[test]
fn scoped_facts_provider_added_after_contract_cannot_invent_attachment_consent() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 1_000);
    let id = accepted_run(&mut tw, &r, 300, "fact private_test term private-proof-7");
    assert!(
        snap(&tw)
            .contract(id)
            .expect("legacy contract")
            .consent
            .fact_consent
            .is_none()
    );
    let provider = Arc::new(Restricted::default());
    *provider.holder.lock().expect("holder") = Some(r.johnson);
    provider.enabled.store(true, Ordering::SeqCst);
    provider.true_now.store(true, Ordering::SeqCst);
    let boot = mush_testkit::fold_module_ops(tw.take_module_ops());
    let mut tw = tw
        .reboot_into(scoped_modules(provider, Probe::default()), boot)
        .expect("old contract still loads");
    tw.connect(r.runner);
    let shown = ok(&mut tw, r.runner, &format!("@contract/show {id}"));
    assert!(!shown.contains("private-proof-7"));
    assert!(shown.contains("restricted external fact"));
    sweep_at(&mut tw, 200);
    assert_eq!(state(&tw, id), ContractState::Active);
    assert_eq!(bal(&tw, r.r_acct), 0);
}

#[test]
fn scoped_facts_copied_marker_does_not_authorize_other_contract_or_changed_surface() {
    for crossed_id in [false, true] {
        let (mut tw, r, provider, probe) = probe_rig();
        let first = run_probe(
            &mut tw,
            &probe,
            r.johnson,
            ProbeRequest::Offer {
                spec: spec(&r),
                spoof: None,
            },
        )
        .expect("offer")
        .expect("id");
        let first_marker = snap(&tw)
            .contract(first)
            .expect("first offer")
            .consent
            .fact_consent
            .clone();
        let target = run_probe(
            &mut tw,
            &probe,
            r.johnson,
            ProbeRequest::Offer {
                spec: spec(&r),
                spoof: None,
            },
        )
        .expect("offer")
        .expect("id");
        let mut changed = snap(&tw).contract(target).expect("second offer").clone();
        if crossed_id {
            changed.consent.fact_consent = first_marker;
        } else {
            changed.consent.evidence = vec![mush_economy::EvidenceKind::Fact];
        }
        // Repair ordinary hashes. Provider subject/parties/visibility still
        // qualify, so the retained marker's exact affinity is the failing gate.
        changed.consent_hash = mush_economy::consent_hash(&changed.issuer, &changed.consent);
        let mut boot = mush_testkit::fold_module_ops(tw.take_module_ops());
        let data = boot.get_mut("economy").expect("real Economy documents");
        let key = format!("contract:{}", target.0);
        let row = data
            .kv
            .iter_mut()
            .find(|(k, _)| k.as_str() == key)
            .expect("actual contract document");
        row.1 = serde_json::to_value(changed).expect("contract codec");
        let mut tw = tw
            .reboot_into(scoped_modules(provider, Probe::default()), boot)
            .expect("document-only contract loads");
        tw.connect(r.runner);
        let shown = ok(&mut tw, r.runner, &format!("@contract/show {target}"));
        assert!(
            !shown.contains("private-proof-7"),
            "copied marker has no disclosure authority"
        );
        assert!(shown.contains("restricted external fact"));
    }
}

#[test]
fn scoped_facts_programmatic_amend_authorizes_before_any_refund() {
    let (mut tw, r, provider, probe) = probe_rig();
    // This test provider explicitly chooses GOD as its subject holder. Actual
    // staff account visibility permits a legitimate two-payer offer; this does
    // not bypass the separate subject authorization tested with Johnson above.
    *provider.holder.lock().expect("holder") = Some(GOD);
    // Both parties owe money; runner accepts first while issuer has not.
    // This is an offered contract with real counterparty escrow to refund.
    let faucet = snap(&tw)
        .accounts()
        .find(|a| matches!(a.owner, PrincipalId::System(_)))
        .expect("faucet")
        .id;
    ok(
        &mut tw,
        GOD,
        &format!("@econ/mint {faucet}={} 100", r.r_acct),
    );
    let mut terms = spec(&r);
    terms.parties.push(PrincipalId::Object(r.johnson));
    terms.considerations.push(mush_economy::ConsiderationSpec {
        account: r.r_acct,
        amount: mush_economy::Amount::from_minor(100),
    });
    terms
        .outcomes
        .first_mut()
        .expect("outcome")
        .payouts
        .first_mut()
        .expect("payout")
        .share = mush_economy::Share::Exact(mush_economy::Amount::from_minor(400));
    let id = run_probe(
        &mut tw,
        &probe,
        GOD,
        ProbeRequest::Offer {
            spec: terms,
            spoof: None,
        },
    )
    .expect("offer")
    .expect("id");
    ok(&mut tw, r.runner, &format!("@contract/accept {id}"));
    let before = snap(&tw).contract(id).expect("offered").clone();
    assert!(!before.escrow.is_empty(), "real runner escrow exists");
    let journal = store(&tw).peek_next_transaction();
    provider.enabled.store(false, Ordering::SeqCst);
    assert!(
        run_probe(
            &mut tw,
            &probe,
            GOD,
            ProbeRequest::Amend {
                id,
                amend: mush_economy::ContractAmendment {
                    add_parties: vec![PrincipalId::Object(r.watcher)],
                    reoffer: true,
                    ..Default::default()
                }
            }
        )
        .is_err()
    );
    assert!(
        snap(&tw).contract(id) == Some(&before),
        "failed authorization preserves exact contract"
    );
    assert_eq!(store(&tw).peek_next_transaction(), journal);
    provider.enabled.store(true, Ordering::SeqCst);
    // A real refund observer revokes eligibility at the landed boundary. The
    // exact preflight must publish without calling the provider again afterward.
    provider.disable_on_post.store(true, Ordering::SeqCst);
    assert!(
        run_probe(
            &mut tw,
            &probe,
            GOD,
            ProbeRequest::Amend {
                id,
                amend: mush_economy::ContractAmendment {
                    add_parties: vec![PrincipalId::Object(r.watcher)],
                    reoffer: true,
                    ..Default::default()
                }
            }
        )
        .is_ok()
    );
    let after = snap(&tw).contract(id).expect("amended");
    assert!(after.consent_intact());
    assert_eq!(after.state, ContractState::Offered);
    assert_eq!(after.amendments, before.amendments + 1);
    assert!(store(&tw).peek_next_transaction() > journal);
    assert!(
        !provider.enabled.load(Ordering::SeqCst),
        "actual post observer ran"
    );
    assert!(after.consent.fact_consent.is_some());
    assert!(
        after
            .consent
            .parties
            .contains(&PrincipalId::Object(r.watcher))
    );
}
