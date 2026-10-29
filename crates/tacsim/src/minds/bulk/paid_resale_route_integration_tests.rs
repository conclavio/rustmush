#![cfg(test)]

//! Continues actual acknowledged entitlement/House purchases. A third actual
//! player/account buys the existing instrument, then accepts its own renewal.
//! No fabricated grant, term, certificate, transfer or account authority.
use super::*;
use mush_core::authoritative::{AuthoritativeBundle, AuthoritativeBundleDraft, ReplayWrite};
use mush_core::{OfferExecutionContext, OfferSubmission};
use mush_roll::commerce::house_catalogue::HouseIssuer;
use mush_roll::commerce::paid_resale::PaidResaleRecord;
use mush_roll::commerce::{LicenceId, TermGeneration};

const POINTER: &str = "roll_mind_personal_paid_licence";
const TERM: &str = "roll_mind_paid_term";
const HOLDER: &str = "roll_mind_paid_holders";
const OFFER: &str = "offer_paid_mind_resale";
const ACCEPT: &str = "accept_paid_mind_resale";
const ROUTE_CONN: mush_core::ConnId = 631;

fn disk(w: &PlayerWorld) -> Disk {
    // Boot normalisation moves every `log:` chunk out of `kv` into `ledger`;
    // a snapshot without them has no Economy journal to replay.
    w.image
        .iter()
        .flat_map(|(owner, data)| {
            let kv = data
                .kv
                .iter()
                .map(move |(key, value)| ((owner.to_string(), key.to_string()), value.clone()));
            let ledger = data.ledger.iter().map(move |(key, text)| {
                (
                    (owner.to_string(), key.to_string()),
                    serde_json::from_str::<Value>(text).expect("a ledger chunk is JSON"),
                )
            });
            kv.chain(ledger)
        })
        .collect()
}
fn saved(w: &PlayerWorld) -> Durable {
    Durable::capture(&w.engine, &disk(w))
}
fn economy(w: &PlayerWorld) -> TestResult<&EconomyStore> {
    w.engine
        .world
        .module_state::<EconomyStore>()
        .ok_or_else(|| "actual Economy".into())
}
fn wallet(w: &PlayerWorld, actor: EconomicOwner) -> TestResult<WalletId> {
    let snapshot = economy(w)?.snapshot();
    let mut all = snapshot.accounts().filter(|a| {
        a.owner == actor
            && a.scope.is_none()
            && a.instrument.as_str() == "cr"
            && a.status == AccountStatus::Active
    });
    let id = all.next().ok_or("actual unique route wallet")?.id;
    check!(all.next().is_none(), "route wallet is unambiguous");
    Ok(id)
}
fn balance(w: &PlayerWorld, id: WalletId) -> TestResult<(Amount, Amount)> {
    let s = economy(w)?.snapshot();
    let a = s.account(id).ok_or("actual wallet")?;
    Ok((a.posted_balance, a.held_amount))
}
fn row(
    w: &PlayerWorld,
    owner: &str,
    family: &str,
    field: &str,
    expected: Value,
) -> TestResult<Value> {
    let mut all = rows(&disk(w), owner, family)
        .into_iter()
        .filter(|v| v.get(field) == Some(&expected));
    let value = all.next().ok_or("actual route row")?;
    check!(all.next().is_none(), "route key is unique");
    Ok(value)
}
fn pointer(w: &PlayerWorld, id: LicenceId) -> TestResult<PersonalPaidLicence> {
    Ok(serde_json::from_value(row(
        w,
        "roll",
        POINTER,
        "id",
        json!(id),
    )?)?)
}
fn term(w: &PlayerWorld, id: LicenceId, generation: TermGeneration) -> TestResult<PaidTermRecord> {
    let mut all = rows(&disk(w), "roll", TERM).into_iter().filter(|v| {
        v.get("licence") == Some(&json!(id)) && v.get("generation") == Some(&json!(generation))
    });
    let value = all.next().ok_or("actual term")?;
    check!(all.next().is_none(), "term key is unique");
    Ok(serde_json::from_value(value)?)
}
fn command(w: &mut PlayerWorld, line: &str) -> TestResult {
    let events = w.say(GOD, line)?;
    check!(
        !events
            .iter()
            .any(|(_, event)| matches!(event, OutputEvent::Sys(text)
        if text.spans().iter().any(|s| s.tag.as_deref() == Some("error")))),
        "actual route setup refused"
    );
    Ok(())
}
fn connect(w: &mut PlayerWorld, buyer: ObjId) -> TestResult {
    w.step(Inbound::ConnUp {
        conn: ROUTE_CONN,
        actor: buyer,
        caps_structured: false,
        from_linkdead: false,
    })?;
    Ok(())
}
/// The resale successor holds the licence: the current paid-licence pointer
/// names them. `SIM MIND /filed` lists original-owner holdings only (a paid
/// holder's listing is a separate, not-yet-built feature), so it is no test of
/// a buyer's holding.
fn holds(w: &PlayerWorld, actor: ObjId, id: LicenceId) -> TestResult {
    check_eq!(
        pointer(w, id)?.holder(),
        mush_economy::Holder::Character(actor),
        "actual current paid holder"
    );
    Ok(())
}
fn visible(w: &mut PlayerWorld, actor: ObjId, id: LicenceId, expected: bool) -> TestResult {
    let lobby =
        match_::centre_object(&w.engine.world, match_::LOBBY_ENTRY).ok_or("actual lobby")?;
    w.engine.world.move_to(actor, lobby)?;
    let events = w.step(Inbound::Cmd {
        conn: if actor == GOD {
            OBSERVER_CONN
        } else if actor == BUYER {
            BUYER_CONN
        } else {
            ROUTE_CONN
        },
        actor,
        line: "SIM MIND /filed".into(),
    })?;
    let text = events.iter().filter(|(a,_)| matches!(a, Audience::Actor(who) if *who == actor)
        || matches!(a, Audience::Conn(c) if *c == ROUTE_CONN && actor != GOD && actor != BUYER)
        || matches!(a, Audience::Conn(c) if *c == BUYER_CONN && actor == BUYER)
        || matches!(a, Audience::Conn(c) if *c == OBSERVER_CONN && actor == GOD))
        .map(|(_, e)| mush_testkit::text_of(e)).collect::<Vec<_>>().join("\n");
    check_eq!(
        text.contains(&format!("filed:{}:", id.get())),
        expected,
        "actual current paid-holder projection"
    );
    w.engine.world.move_to(actor, ROOM)?;
    w.commit()?;
    Ok(())
}

fn source(w: &mut PlayerWorld, actor: ObjId, paid: &PaidTermRecord, allowed: bool) -> TestResult {
    use mush_proto::{InvokeRequest, InvokeResult, MindRequest, MindResponse};
    let conn = if actor == GOD {
        OBSERVER_CONN
    } else if actor == BUYER {
        BUYER_CONN
    } else {
        ROUTE_CONN
    };
    let mut output = Outbox::new();
    w.engine.step(
        Inbound::Invoke {
            conn,
            actor,
            id: 63_100,
            request: InvokeRequest::Mind(MindRequest::FiledSource {
                licence: paid.licence().get().to_string(),
                first: 1,
                last: 200,
            }),
        },
        &mut output,
        &mut mush_core::engine::Effects::new(),
    );
    w.commit()?;
    let mut count = 0;
    for (audience, event) in output.take() {
        if let OutputEvent::InvokeResult {
            result: InvokeResult::Mind(answer),
            ..
        } = event
        {
            count += 1;
            check_eq!(audience, Audience::Conn(conn));
            match answer {
                MindResponse::FiledSource {
                    source_sha256,
                    lines,
                    ..
                } => {
                    check!(allowed, "former/sealed holder received source");
                    let mush_roll::commerce::records::RevisionPolicy::Pin { revision, .. } =
                        paid.policy()
                    else {
                        return Err("actual Pin".into());
                    };
                    check_eq!(source_sha256, revision.source().to_hex());
                    check!(!lines.is_empty());
                }
                MindResponse::Error { .. } => {
                    check!(!allowed, "actual current Open source refused")
                }
                _ => return Err("unexpected source response".into()),
            }
        }
    }
    check_eq!(count, 1);
    Ok(())
}

struct Prepared {
    prepared: PreparedComposedTxn,
    output: mush_core::action::PreparedAuthoritativeAction,
    invocation: ActionInvocation,
    action: DefinitionRef,
    number: u128,
}
fn prepare(
    w: &mut PlayerWorld,
    id: &str,
    actor: ObjId,
    number: u128,
    parameters: Value,
) -> TestResult<Box<Prepared>> {
    w.engine.set_offer_keys(mush_core::OfferKeyRing::new(
        mush_core::OfferSecret::new(1, [0x67; 32])?,
        [],
    )?);
    let action = w
        .engine
        .registry
        .definitions()
        .ok_or("definitions")?
        .iter()
        .map(|d| d.reference())
        .find(|r| {
            r.key().owner.as_str() == "market"
                && r.key().kind.as_str() == "action"
                && r.key().id.as_str() == id
                && r.version() == 1
        })
        .cloned()
        .ok_or("actual action")?;
    let principal = PrincipalId::Object { id: actor };
    let query = w
        .engine
        .legal_action_query(principal.clone(), actor, PerceivedRef::SelfActor)?;
    let projected = w.engine.project_legal_actions(&query)?;
    let offer = projected
        .projected_for_action(&action)
        .ok_or("actual signed route offer")?;
    let req = request(number, actor, "market", parameters.clone())?;
    let resolved = w.engine.resolve_legal_action(
        &projected,
        OfferExecutionContext {
            request: req.request(),
            principal,
            actor,
            idempotency: req.idempotency().clone(),
            viewer_scope: query.viewer_scope(),
            now: None,
        },
        OfferSubmission {
            offer: offer.offer(),
            subject: PerceivedRef::SelfActor,
            targets: vec![],
            parameters,
        },
    )?;
    let (actual, resolved, _) = resolved.into_authoritative_parts();
    check_eq!(actual, action);
    let invocation = resolved.invocation().clone();
    let (prepared, output) = w
        .engine
        .prepare_registered_composed_resolved_authoritative_action(
            &action,
            &resolved,
            token(number),
        )?;
    Ok(Box::new(Prepared {
        prepared,
        output,
        invocation,
        action,
        number,
    }))
}
fn land(w: &mut PlayerWorld, value: Box<Prepared>) -> TestResult {
    let Prepared {
        prepared,
        output,
        invocation,
        action,
        number,
    } = *value;
    let (outcome, resolution, mut definitions, mut witnesses, mut outbox, semantic_events) =
        output.into_parts();
    definitions.extend([action.clone(), outcome.schema().clone()]);
    definitions.sort();
    definitions.dedup();
    if witnesses.is_empty() {
        witnesses.push(invocation.principal().clone());
    }
    witnesses.sort();
    witnesses.dedup();
    outbox.sort_by_key(|o| o.ordinal());
    let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
        token: token(number),
        request: invocation.request(),
        idempotency: invocation.idempotency().clone(),
        ingress_hash: CanonicalHash::from_bytes([31; 32]),
        request_hash: invocation.request_hash(),
        plan_hash: CanonicalHash::from_bytes([32; 32]),
        action,
        provider: prepared.owner().clone(),
        cause: mush_core::action::PreparedCausalRef::new(invocation.request(), None, 0)?,
        definitions,
        writes: prepared.authoritative_replay_writes(),
        outcome,
        resolution: resolution.map(|r| r.bind(token(number))),
        semantic_events,
        eligible_witnesses: witnesses,
        durable_outbox: outbox,
    })?;
    let guards = w
        .engine
        .registry
        .server_guard_set_builder(mush_core::RevisionSet::default())?
        .finish()?;
    // The host's persistence pass drains ordinary world dirt (moves, logins,
    // commands) before a staged action, as the other fixtures do.
    let _ = w.engine.world.take_dirty();
    w.engine
        .stage_authoritative_action(prepared, guards, bundle)?;
    let staged = w.engine.take_staged_action()?.ok_or("actual custody")?;
    let (bundle, installation) = staged.into_parts();
    let operations = bundle
        .writes()
        .iter()
        .map(|write| match write {
            ReplayWrite::ModuleJsonSet {
                namespace,
                key,
                value,
            } => Ok(ModuleOp {
                ns: namespace.to_string().into(),
                op: ModuleWrite::JsonSet {
                    key: key.to_string().into(),
                    value: value.clone(),
                },
            }),
            ReplayWrite::ModuleDelete { namespace, key } => Ok(ModuleOp {
                ns: namespace.to_string().into(),
                op: ModuleWrite::Del {
                    key: key.to_string().into(),
                },
            }),
            ReplayWrite::LedgerChunkSet { write } => Ok(ModuleOp {
                ns: write.namespace().to_string().into(),
                op: ModuleWrite::JsonSet {
                    key: write.key().to_string().into(),
                    value: serde_json::to_value(write.chunk())?,
                },
            }),
            _ => Err("route actions must not manufacture objects".into()),
        })
        .collect::<TestResult<Vec<_>>>()?;
    mush_testkit::fold_module_ops_into(&mut w.image, operations);
    w.engine.install_composed_action(installation)?;
    w.commit()?;
    Ok(())
}
fn perform(
    w: &mut PlayerWorld,
    id: &str,
    actor: ObjId,
    number: u128,
    parameters: Value,
) -> TestResult {
    let p = prepare(w, id, actor, number, parameters)?;
    land(w, p)
}
fn paired(w: &PlayerWorld, acceptance: u128) -> TestResult<PaidResaleRecord> {
    let actual: PaidResaleRecord = serde_json::from_value(row(
        w,
        "roll",
        "roll_mind_paid_resales",
        "operation",
        json!(token(acceptance)),
    )?)?;
    let market: mush_market::mind_paid_resale::MindPaidResaleOperation =
        serde_json::from_value(row(
            w,
            "market",
            "market_mind_paid_resales",
            "operation",
            json!(token(acceptance)),
        )?)?;
    market.verify_history(&actual, economy(w)?.paid_resale_history(token(acceptance))?)?;
    let receipt: mush_market::mind_paid_resale::MindPaidResaleReceipt =
        serde_json::from_value(row(
            w,
            "market",
            "market_mind_paid_resale_receipts",
            "acceptance",
            json!(token(acceptance)),
        )?)?;
    receipt.matches_roll(&actual)?;
    Ok(actual)
}
fn renewal(w: &PlayerWorld, house: bool, operation: u128) -> TestResult<Value> {
    let (market_family, roll_family) = if house {
        (
            "market_house_mind_renewal_operation",
            "roll_house_mind_renewal_operation",
        )
    } else {
        (
            "market_mind_renewal_operation",
            "roll_mind_renewal_operation",
        )
    };
    let v = row(
        w,
        "market",
        market_family,
        "operation",
        json!(token(operation)),
    )?;
    let r = row(w, "roll", roll_family, "operation", json!(token(operation)))?;
    if house {
        let m: mush_market::mind_house_renewal::MindRenewalOperationV1 =
            serde_json::from_value(v.clone())?;
        let r: mush_roll::commerce::house_renewal::RenewalOperationV1 = serde_json::from_value(r)?;
        let audit = economy(w)?.house_renewal_audit()?;
        let proof = audit.get(token(operation))?;
        r.verify_history(&proof)?;
        m.matches_roll(&r)?;
        m.verify_history(&proof)?;
    } else {
        let m: mush_market::mind_renewal::MindRenewalOperationV1 =
            serde_json::from_value(v.clone())?;
        let r: mush_roll::commerce::renewal::RenewalOperationV1 = serde_json::from_value(r)?;
        let audit = economy(w)?.renewal_audit()?;
        let proof = audit.get(token(operation))?;
        r.verify_history(&proof)?;
        m.matches_roll(&r)?;
        m.verify_history(&proof)?;
    }
    Ok(v)
}
fn due(w: &mut PlayerWorld, at: i64) -> TestResult {
    let before = w.engine.world.world_ic_now();
    w.engine.world.set_now(
        w.engine
            .world
            .now()
            .checked_add(at.checked_sub(before.at).ok_or("IC delta")?)
            .ok_or("UTC")?,
    );
    check_eq!(w.engine.world.world_ic_now().at, at);
    check_eq!(w.engine.world.world_ic_now().clock, before.clock);
    Ok(())
}
/// 17g C1a: the House buyer admission the House resale fixture installs in
/// the paid resale writer, standing in for the launch's Coprates admission.
/// It refuses exactly the one armed `(issuer, buyer)` pair, admits every other
/// pair, and records every pair it is consulted with. The record is what binds
/// the fence: the route asserts that the fence asks about the resold licence's
/// own House issuer and the accepting buyer, and only at the buyer's steps.
#[derive(Debug, Default)]
pub(super) struct AdmissionProbe {
    armed: std::sync::Mutex<Option<(HouseIssuer, ObjId)>>,
    consulted: std::sync::Mutex<Vec<(HouseIssuer, ObjId)>>,
}
impl AdmissionProbe {
    fn arm(&self, pair: Option<(HouseIssuer, ObjId)>) -> TestResult {
        *self.armed.lock().map_err(module_error)? = pair;
        Ok(())
    }
    fn consulted(&self) -> TestResult<Vec<(HouseIssuer, ObjId)>> {
        Ok(self.consulted.lock().map_err(module_error)?.clone())
    }
}
impl mush_roll::commerce::house_purchase::HouseBuyerAdmission for AdmissionProbe {
    fn check(
        &self,
        _: &mush_core::World,
        issuer: HouseIssuer,
        buyer: ObjId,
    ) -> Result<(), mush_roll::commerce::records::PublicationError> {
        fn unavailable<T>(_: T) -> mush_roll::commerce::records::PublicationError {
            mush_roll::commerce::records::PublicationError::Unavailable
        }
        self.consulted
            .lock()
            .map_err(unavailable)?
            .push((issuer, buyer));
        if *self.armed.lock().map_err(unavailable)? == Some((issuer, buyer)) {
            Err(mush_roll::commerce::records::PublicationError::Unavailable)
        } else {
            Ok(())
        }
    }
}
/// 17g C1a: the House buyer admission on paid resale. The probe is armed for
/// the resold licence's House issuer and the offer's named buyer. That buyer's
/// Accept of the open resale offer then refuses with exactly the bytes of an
/// Accept naming an offer that does not exist: the uniform refusal, no
/// sentence of its own. Fund is not reached here; with no open paid claim it
/// refuses on that ground first.
///
/// The probe's record proves the refusal is the admission's: nothing before
/// the buyer's Accept consulted it (not the boot, the House purchase, the
/// seller's renewal attempt or the seller's Offer), the nonexistent offer
/// never reaches it, and the fenced Accept asks about exactly the armed pair.
/// A fence bound to another principal or issuer would be admitted, and fail
/// here. Disarming leaves the existing admitted path to run unchanged.
fn refused_admission(
    w: &mut PlayerWorld,
    probe: &AdmissionProbe,
    issuer: HouseIssuer,
    buyer: ObjId,
    payer: WalletId,
    start: u128,
) -> TestResult {
    check!(
        probe.consulted()?.is_empty(),
        "nothing before the buyer's Accept consults the House admission"
    );
    let missing = prepare(
        w,
        ACCEPT,
        buyer,
        start + 40,
        json!({"offer":token(start+42),"payer":payer.0}),
    );
    check!(
        probe.consulted()?.is_empty(),
        "an offer that does not exist never reaches the House admission"
    );
    probe.arm(Some((issuer, buyer)))?;
    let fenced = prepare(
        w,
        ACCEPT,
        buyer,
        start + 41,
        json!({"offer":token(start),"payer":payer.0}),
    );
    probe.arm(None)?;
    let (Err(missing), Err(fenced)) = (missing, fenced) else {
        return Err("a resold House Pin's buyer passed a refusing admission".into());
    };
    let consulted = probe.consulted()?;
    check!(
        !consulted.is_empty() && consulted.iter().all(|pair| *pair == (issuer, buyer)),
        "the fenced Accept asks about the licence's House issuer and its buyer: {consulted:?}"
    );
    check_eq!(fenced.to_string(), missing.to_string());
    check_eq!(format!("{fenced:?}"), format!("{missing:?}"));
    eprintln!("M7_STORY phase=paidresale.route event=admission_refused");
    Ok(())
}
struct Login<'a> {
    accounts: &'a Accounts,
    actor: ObjId,
    previous: Option<AccountId>,
}
impl Drop for Login<'_> {
    fn drop(&mut self) {
        if let Ok(mut values) = self.accounts.values.lock() {
            self.accounts.epoch.invalidate();
            if let Some(old) = self.previous {
                values.insert(self.actor, old);
            } else {
                values.remove(&self.actor);
            }
        }
    }
}

#[inline(never)]
#[allow(clippy::too_many_arguments)]
pub(super) fn exercise(
    accounts: &Accounts,
    snapshot: &Durable,
    id: LicenceId,
    seller: ObjId,
    house: bool,
    funding_system: &str,
    start: u128,
    admission: Option<(&AdmissionProbe, HouseIssuer)>,
    boot: impl Fn(&Durable) -> TestResult<PlayerWorld>,
) -> TestResult {
    eprintln!("M7_STORY phase=paidresale.route event=begin house={house}");
    let mut world = snapshot.world()?;
    let buyer = world.create(ObjKind::Player, "Actual resale successor", GOD, Some(ROOM))?;
    let previous = {
        let mut values = accounts.values.lock().map_err(module_error)?;
        accounts.epoch.invalidate();
        values.insert(
            buyer,
            AccountId::mint("actual-resale-successor", i64::from(buyer.0))?,
        )
    };
    let _login = Login {
        accounts,
        actor: buyer,
        previous,
    };
    let mut input = snapshot.clone();
    input.objects = world.iter().cloned().collect();
    input.next_id = world.next_id();
    let mut w = boot(&input)?;
    connect(&mut w, buyer)?;
    for who in [seller, buyer] {
        w.engine.world.move_to(who, ROOM)?;
    }
    w.commit()?;
    command(&mut w, &format!("@account/open #{}=cr", buyer.0))?;
    let payer = wallet(&w, EconomicOwner::Object(buyer))?;
    let seller_account = wallet(&w, EconomicOwner::Object(seller))?;
    let faucet = wallet(
        &w,
        EconomicOwner::System(SystemPrincipalId::new(funding_system).ok_or("funding system")?),
    )?;
    command(&mut w, &format!("@econ/mint {faucet}={payer} 2000"))?;
    let old = pointer(&w, id)?;
    let predecessor = term(&w, id, old.term())?;
    check_eq!(old.holder(), mush_economy::Holder::Character(seller));
    check_eq!(predecessor.terms().seats().get(), 1);
    let birth_family = if house {
        "roll_mind_house_purchase"
    } else {
        "roll_mind_entitlement_purchase"
    };
    let economics_family = if house {
        "roll_mind_house_term_economics"
    } else {
        "roll_mind_paid_term_economics"
    };
    let birth = rows(&disk(&w), "roll", birth_family);
    let old_economics = rows(&disk(&w), "roll", economics_family);
    let payer_before = balance(&w, payer)?;
    let (draft, close, fund, grant) = if house {
        (
            "draft_house_mind_renewal",
            "close_house_mind_renewal",
            "fund_house_mind_renewal",
            "settle_house_mind_renewal",
        )
    } else {
        (
            "draft_mind_renewal",
            "close_mind_renewal",
            "fund_mind_renewal",
            "settle_mind_renewal",
        )
    };
    // A closed seller attempt is immutable history, not the buyer's retry root.
    // Transfer occurs in Grace with exactly the disclosed original term.
    due(&mut w, predecessor.window().term_end().get())?;
    perform(
        &mut w,
        draft,
        seller,
        start + 30,
        json!({"licence":id.get()}),
    )?;
    let seller_offer = renewal(&w, house, start + 30)?;
    check_eq!(
        seller_offer.pointer("/attempt/kind"),
        Some(&json!("automatic"))
    );
    perform(
        &mut w,
        close,
        seller,
        start + 31,
        json!({"operation":token(start+30),"reason":"declined"}),
    )?;
    check_eq!(pointer(&w, id)?, old);
    perform(
        &mut w,
        OFFER,
        seller,
        start,
        json!({"licence":id.get(),"buyer":buyer.0,"price":100,"seller_account":seller_account.0}),
    )?;
    check_eq!(pointer(&w, id)?, old);
    if let Some((probe, issuer)) = admission {
        refused_admission(&mut w, probe, issuer, buyer, payer, start)?;
        check_eq!(pointer(&w, id)?, old);
        check_eq!(balance(&w, payer)?, payer_before);
    }
    let accepted = start + 1;
    perform(
        &mut w,
        ACCEPT,
        buyer,
        accepted,
        json!({"offer":token(start),"payer":payer.0}),
    )?;
    if let Some((probe, issuer)) = admission {
        // The admitted Accept asked the same question, and was answered yes.
        let consulted = probe.consulted()?;
        check!(
            consulted.len() >= 2 && consulted.iter().all(|pair| *pair == (issuer, buyer)),
            "the admitted Accept asks about the same issuer and buyer: {consulted:?}"
        );
    }
    let actual = paired(&w, accepted)?;
    check_eq!(actual.binding().term(), &predecessor);
    let transferred = pointer(&w, id)?;
    check_eq!(transferred.holder(), mush_economy::Holder::Character(buyer));
    check_eq!(transferred.term(), old.term());
    check_eq!(transferred.operation(), old.operation());
    check_eq!(
        transferred.ownership().ok_or("holder capture")?.generation,
        2
    );
    check_eq!(term(&w, id, old.term())?, predecessor);
    check!(
        rows(&disk(&w), "roll", birth_family) == birth,
        "resale changed immutable birth"
    );
    check!(
        rows(&disk(&w), "roll", economics_family) == old_economics,
        "resale changed immutable economics"
    );
    check_eq!(
        balance(&w, payer)?,
        (
            payer_before
                .0
                .checked_sub(Amount::from_minor(100))
                .ok_or("paid price")?,
            payer_before.1
        )
    );
    visible(&mut w, seller, id, false)?;
    holds(&w, buyer, id)?;
    let open = predecessor.terms().form() != mush_roll::commerce::paid::PaidForm::Sealed {};
    source(&mut w, seller, &predecessor, false)?;
    source(&mut w, buyer, &predecessor, open)?;
    let transferred_image = saved(&w);
    drop(w);
    let mut w = boot(&transferred_image)?;
    connect(&mut w, buyer)?;
    paired(&w, accepted)?;
    renewal(&w, house, start + 30)?;
    check_eq!(pointer(&w, id)?, transferred);
    holds(&w, buyer, id)?;
    source(&mut w, buyer, &predecessor, open)?;
    let mut missing = transferred_image.clone();
    check!(remove_family(&mut missing, "roll", HOLDER) > 0);
    check!(
        crate::test_checks::refused(boot(&missing))?,
        "transferred current holder lost its actual cause"
    );
    due(&mut w, predecessor.window().term_end().get())?;
    check!(
        crate::test_checks::refused(prepare(
            &mut w,
            draft,
            seller,
            start + 10,
            json!({"licence":id.get()})
        ))?,
        "former holder cannot renew"
    );
    check!(
        crate::test_checks::refused(prepare(
            &mut w,
            draft,
            buyer,
            start + 10,
            json!({"previous_operation":token(start+30)})
        ))?,
        "new holder cannot relabel the seller's closed attempt as its retry"
    );
    let automatic = start + 11;
    perform(&mut w, draft, buyer, automatic, json!({"licence":id.get()}))?;
    let offered = renewal(&w, house, automatic)?;
    check_eq!(offered.pointer("/binding/holder"), Some(&json!(buyer)));
    check_eq!(offered.pointer("/binding/format"), Some(&json!(2)));
    check_eq!(
        offered.pointer("/binding/holder_basis/acceptance"),
        Some(&json!(token(accepted)))
    );
    check_eq!(
        offered.pointer("/attempt/kind"),
        Some(&json!("holder_automatic"))
    );
    check_eq!(
        pointer(&w, id)?,
        transferred,
        "Draft did not publish a new term"
    );
    perform(
        &mut w,
        close,
        buyer,
        start + 12,
        json!({"operation":token(automatic),"reason":"declined"}),
    )?;
    check!(
        crate::test_checks::refused(prepare(
            &mut w,
            draft,
            buyer,
            start + 13,
            json!({"licence":id.get()})
        ))?,
        "one automatic offer per holder/term"
    );
    let retry = start + 14;
    perform(
        &mut w,
        draft,
        buyer,
        retry,
        json!({"previous_operation":token(automatic)}),
    )?;
    let retried = renewal(&w, house, retry)?;
    check_eq!(
        retried.pointer("/attempt/kind"),
        Some(&json!("holder_retry"))
    );
    check_eq!(
        retried.pointer("/binding/holder_basis"),
        offered.pointer("/binding/holder_basis")
    );
    check_ne!(retried.get("contract"), offered.get("contract"));
    perform(
        &mut w,
        fund,
        buyer,
        start + 15,
        json!({"operation":token(retry)}),
    )?;
    check_eq!(
        pointer(&w, id)?,
        transferred,
        "Fund did not publish a new term"
    );
    let funded = saved(&w);
    drop(w);
    let mut w = boot(&funded)?;
    connect(&mut w, buyer)?;
    renewal(&w, house, retry)?;
    perform(
        &mut w,
        grant,
        buyer,
        start + 16,
        json!({"operation":token(retry)}),
    )?;
    renewal(&w, house, retry)?;
    let renewed = pointer(&w, id)?;
    check_eq!(renewed.holder(), mush_economy::Holder::Character(buyer));
    check_eq!(renewed.ownership(), transferred.ownership());
    check_eq!(renewed.term().get(), old.term().get() + 1);
    check_eq!(renewed.operation(), old.operation());
    check_eq!(term(&w, id, old.term())?, predecessor);
    check!(
        rows(&disk(&w), "roll", birth_family) == birth,
        "renewal changed immutable birth"
    );
    let current = term(&w, id, renewed.term())?;
    check_eq!(current.operation(), token(retry));
    check_eq!(current.terms(), predecessor.terms());
    visible(&mut w, seller, id, false)?;
    holds(&w, buyer, id)?;
    let complete = saved(&w);
    drop(w);
    let mut cold = boot(&complete)?;
    connect(&mut cold, buyer)?;
    paired(&cold, accepted)?;
    renewal(&cold, house, start + 30)?;
    renewal(&cold, house, automatic)?;
    renewal(&cold, house, retry)?;
    check_eq!(pointer(&cold, id)?, renewed);
    holds(&cold, buyer, id)?;
    source(&mut cold, buyer, &current, open)?;
    eprintln!("M7_STORY phase=paidresale.route event=complete house={house}");
    Ok(())
}
