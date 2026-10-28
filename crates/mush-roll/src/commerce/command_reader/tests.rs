#![cfg(test)]

//! 17g C2a: the line-mode command kinds over Roll's real filing fixture (the
//! registered composition, an acknowledged shell, an actual initial filing),
//! read through the registered original-holdings read (C2 plan §2.4
//! fallback 1) as TacSim's command slot reads them.
use super::*;
use crate::commerce::filing_tests::{Fixture, config, token};
use crate::commerce::holding_reader::{OriginalOwnerReader, OriginalReadRequest};
use mush_core::read_service::OwnerReadHandle;
use mush_core::{GOD, ModError, ObjId, Outbox, Registrar, World};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

type TestResult = Result<(), Box<dyn std::error::Error>>;
/// The command answer and the whole result's encoding as it leaves Roll.
type Answer = Result<(RollCommandReadResult, String), OwnerReadError>;
type Answers = Arc<Mutex<Option<Answer>>>;

struct Query {
    reader: OwnerReadHandle<OriginalOwnerReader>,
    answers: Answers,
}
impl mush_core::module::QueryHandler for Query {
    fn query(
        &self,
        _: &World,
        _: ObjId,
        _: &Value,
    ) -> Result<mush_proto::OutputEvent, mush_proto::RichText> {
        Err(mush_proto::RichText::plain("live context required"))
    }
    fn query_with_context(
        &self,
        ctx: &mush_core::module::HookReadContext<'_>,
        _: ObjId,
        args: &Value,
    ) -> Result<mush_proto::OutputEvent, mush_proto::RichText> {
        // 17g C2c: `{"holdings": <cursor|null>, "paid": <bool>}` and
        // `{"names": [<charter>...]}` select the C2c kinds.
        let request = if let Some(after) = args.get("holdings") {
            let section = if args.get("paid").and_then(Value::as_bool) == Some(true) {
                HoldingSection::Paid
            } else {
                HoldingSection::Owner
            };
            RollCommandReadRequest::own_holdings(section, after.as_u64())
        } else if let Some(licence) = args
            .get("paid")
            .and_then(Value::as_u64)
            .and_then(|n| LicenceId::new(n).ok())
        {
            // 17g C2d: `{"paid": <licence>}`.
            RollCommandReadRequest::own_paid(licence)
        } else if let Some(filed) = args
            .get("charter_offers")
            .and_then(Value::as_u64)
            .and_then(|n| FiledRevisionId::new(n).ok())
        {
            // 17g C2g: `{"charter_offers": <filed>}`.
            RollCommandReadRequest::charter_offers(filed)
        } else if let Some(names) = args.get("names").and_then(Value::as_array) {
            RollCommandReadRequest::charter_names(
                names
                    .iter()
                    .filter_map(|n| {
                        n.as_u64()
                            .and_then(|n| u32::try_from(n).ok())
                            .and_then(|n| CharterId::new(n).ok())
                    })
                    .collect(),
            )
        } else {
            match args.as_u64().and_then(|n| FiledRevisionId::new(n).ok()) {
                Some(filed) => RollCommandReadRequest::own_filed(filed),
                None => RollCommandReadRequest::open_shell(),
            }
        };
        let answer = ctx
            .read_owner(&self.reader, OriginalReadRequest::command(request))
            .and_then(|result| {
                let encoded =
                    serde_json::to_string(&result).map_err(|_| OwnerReadError::Integrity)?;
                let command = result.into_command().ok_or(OwnerReadError::Integrity)?;
                Ok((command, encoded))
            });
        if let Ok(mut answers) = self.answers.lock() {
            *answers = Some(answer);
        }
        Ok(mush_proto::OutputEvent::Sys(mush_proto::RichText::plain(
            "observed",
        )))
    }
}
/// The fixture's "tacsim" module reads through its one original-holdings
/// grant, as TacSim's command slot does.
fn tacsim_reads(
    reg: &mut Registrar<'_>,
    reader: OwnerReadHandle<OriginalOwnerReader>,
) -> Result<(), ModError> {
    let answers: Answers = Arc::new(Mutex::new(None));
    reg.state(answers.clone());
    reg.query_handler("mind.commands.test", Arc::new(Query { reader, answers }))
}
fn read(f: &mut Fixture, actor: ObjId, args: Value) -> Result<Answer, Box<dyn std::error::Error>> {
    f.engine.step(
        mush_core::Inbound::Query {
            conn: 1,
            actor,
            id: 1,
            name: "mind.commands.test".into(),
            args,
        },
        &mut Outbox::new(),
        &mut mush_core::engine::Effects::new(),
    );
    let answer = f
        .engine
        .world
        .module_state::<Answers>()
        .ok_or("answers")?
        .lock()
        .map_err(|_| "answers poisoned")?
        .take()
        .ok_or("no answer")?;
    Ok(answer)
}

/// 17g C2a test 6 (C2 plan §2.4). Over the real filing fixture:
/// - `OpenShell`: GOD's own acknowledged reservation reads its request id;
///   another actor reads none; after the filing consumes it, none;
/// - `OwnFiled`: the author reads the filed revision's charter, line, owner
///   licence (the charter's initial filing operation names it), the charter's
///   public name, an original line and an Active owner phase with whole IC
///   days left;
/// - another actor naming that filed number and the author naming a number
///   that does not exist both read `Refused`, identically;
/// - the serialized answers carry no digest, account, rating or source, nor
///   any original-holding field beyond the empty ones.
///
/// The owner phase's Grace, Lapsed and rounding are proven on `phase` below.
/// The other actor is a real second player character (reviewer F4), not a
/// room. A dependent line and a transferred charter need fixtures this crate
/// does not have (a dependent filing's parent rights, an E4 charter
/// transfer); `dependent` is the birth operation's parent and the title check
/// is `charter_transfer::history::current`, the own-holding read's own. Both
/// are a residual the lead owns (the C2d story's charter step).
#[test]
fn command_reader_answers_only_for_the_actors_own_shell_and_filed_minds() -> TestResult {
    let mut f = Fixture::new_with_reads(true, tacsim_reads, true);
    let other = f.engine.world.create(
        mush_core::ObjKind::Player,
        "Other",
        GOD,
        Some(mush_core::LIMBO),
    )?;
    check!(
        f.engine
            .world
            .get(other)
            .is_some_and(|o| o.kind == mush_core::ObjKind::Player && other != GOD),
        "a second live player character"
    );
    let (shell, _) = read(&mut f, GOD, Value::Null)??;
    check_eq!(
        shell.open_shell(),
        Some(Some(RequestUid::from_bytes([3; 16]))),
        "the fixture's own acknowledged reservation"
    );
    check_eq!(
        read(&mut f, other, Value::Null)??.0.open_shell(),
        Some(None),
        "another actor's shell is invisible"
    );
    check!(matches!(
        read(&mut f, GOD, json!(1))?,
        Err(OwnerReadError::Refused)
    ));
    // File through the real registered action.
    let request = f.request();
    let (prepared, _) = f.engine.prepare_registered_composed_authoritative_action(
        &config().initial_filing_action()?,
        &request,
        token(4),
    )?;
    let (_, install) = prepared.into_parts();
    f.engine.install_composed_action(install)?;
    check_eq!(
        read(&mut f, GOD, Value::Null)??.0.open_shell(),
        Some(None),
        "filing consumed the shell"
    );
    let filed = FiledRevisionId::new(1)?;
    let (answer, encoded) = read(&mut f, GOD, json!(1))??;
    check!(
        encoded.starts_with(r#"{"holding":null,"next_after":null,"command":{"kind":"own_filed""#),
        "{encoded}"
    );
    let view = answer.into_own_filed().ok_or("an own filed view")?;
    let record = f
        .engine
        .action_state()
        .get(&f.families.filed, &filed.key())?
        .ok_or("the filed row")?;
    check_eq!(view.filed(), filed);
    check_eq!(view.charter(), record.content().position().charter);
    check_eq!(view.line(), record.content().position().line);
    check_eq!(view.charter().get(), f.shell.0, "the charter is the shell");
    check_eq!(view.owner_licence(), LicenceId::new(1)?);
    check!(!view.dependent(), "an original line");
    check_eq!(view.name(), "Original");
    check_eq!(view.owner_phase(), OwnerPhase::Active { days_left: 90 });
    check_eq!(
        serde_json::to_value(&view)?,
        json!({"filed":"1","charter":view.charter().get().to_string(),"line":"1",
            "owner_licence":"1","dependent":false,"name":"Original",
            "owner_phase":{"phase":"active","days_left":90}})
    );
    for forbidden in ["digest", "account", "payer", "rating", "source", "revision"] {
        check!(!encoded.contains(forbidden), "{forbidden} in {encoded}");
    }
    check!(
        !encoded.contains(&f.revision.reference().digest.to_string()),
        "no revision digest"
    );
    // Foreign and missing read alike.
    let foreign = read(&mut f, other, json!(1))?;
    let missing = read(&mut f, GOD, json!(99))?;
    check_eq!(foreign, Err(OwnerReadError::Refused));
    check_eq!(missing, Err(OwnerReadError::Refused));
    Ok(())
}

/// 17g C2a (C2 plan §2.4 fallback 1): the folded kinds answer only when the
/// player market opens them (`with_mind_commands`). Over the same fixture
/// with the original-holdings grant but without the market, both kinds read
/// `Unavailable`, the actor's own acknowledged shell included.
///
/// 17g C2h (a C2 coverage note): every Roll command kind since, too, so a
/// refactor that moved `read`'s early return could not open one of them
/// unnoticed: `OwnHoldings` in both sections, `CharterNames`, `OwnPaid` and
/// `CharterOffers` all read `Unavailable`. (`OwnPurchase`, `OwnOpen` and
/// `ResaleOffers` are the MARKET's command kinds, not Roll's: they exist
/// only where the market's own read is registered, which is the player
/// market's composition.)
#[test]
fn command_kinds_are_closed_without_the_player_market() -> TestResult {
    let mut f = Fixture::new_with_reads(true, tacsim_reads, false);
    for args in [
        Value::Null,
        json!(1),
        json!({"holdings": null}),
        json!({"holdings": null, "paid": true}),
        json!({"names": [1]}),
        json!({"paid": 1}),
        json!({"charter_offers": 1}),
    ] {
        check_eq!(
            read(&mut f, GOD, args.clone())?,
            Err(OwnerReadError::Unavailable),
            "{args}"
        );
    }
    Ok(())
}

/// The owner phase in whole IC days, rounded up, at the term's boundaries.
#[test]
fn owner_phase_rounds_whole_ic_days_up_and_reads_grace_and_lapse() -> TestResult {
    let day = DAY;
    let window = super::super::TermWindow::new(
        WorldIcSeconds(1_000),
        WorldIcSeconds(1_000),
        WorldIcSeconds(1_000 + 90 * day),
        WorldIcSeconds(1_000 + 93 * day),
    )?;
    let at = |offset: i64| WorldIcSeconds(1_000 + offset);
    check_eq!(phase(window, at(0)), OwnerPhase::Active { days_left: 90 });
    check_eq!(phase(window, at(1)), OwnerPhase::Active { days_left: 90 });
    check_eq!(phase(window, at(day)), OwnerPhase::Active { days_left: 89 });
    check_eq!(
        phase(window, at(90 * day - 1)),
        OwnerPhase::Active { days_left: 1 }
    );
    check_eq!(
        phase(window, at(90 * day)),
        OwnerPhase::Grace { days_left: 3 }
    );
    check_eq!(
        phase(window, at(93 * day - 1)),
        OwnerPhase::Grace { days_left: 1 }
    );
    check_eq!(phase(window, at(93 * day)), OwnerPhase::Lapsed);
    Ok(())
}

/// 17g C2c test 3 (C2 plan §2.4), over the real filing fixture: the owner
/// section of `OwnHoldings` lists the actor's own filed mind (the line's head
/// filed revision, its charter, owner licence, name and Active phase, not
/// selling, not dependent), pages with its asset cursor (after the charter's
/// own id there is nothing), and is empty for another player; the paid
/// section is `Unavailable` (the per-holder paid index C2b did not land);
/// `CharterNames` names the charter and leaves out an unknown one. No
/// answer carries a digest, account, rating, source or revision.
///
/// The C2c fix round (reviewer T-F2) adds, over the same fixture:
/// - `selling`: the fixture's charter has no pending-transfer head, so
///   `pending_sale` reads `false` through the family; the head-to-`selling`
///   mapping (`funded`: a `Funded` head is a sale, a refunded one or none is
///   not, another charter's head is `Integrity`) is proven below. Core's
///   terminal-capacity guard refuses a pending head written outside its
///   protocol, so no fixture here can hold a funded head;
/// - the line head: with the line's head moved to a filed number that has no
///   row, the charter drops out of the page (`own_filed` refuses the head),
///   so the row is the head's and not the charter's initial filed revision.
///
/// The page cuts (`OWNER_ROWS_PER_READ`, `OWN_ASSETS_PER_READ`) and their
/// cursors are `page`'s, proven below over synthetic assets.
///
/// 17g C2g (C2c test 3's owed cases, the C2e plan §4.1 item 1): the paid
/// section through the real read (`paid_section_pages_own_rows`).
///
/// Not proven here (no fixture): the dependent owner row (no dependent
/// filing in this crate's fixture, as C2a's residual says; owed to the
/// story), the House Pin row and the selling flag (both protocol-owned rows
/// mush-core refuses outside their protocol; `paid_section_pages_own_rows`'
/// doc quotes the refusals; owed to the C2h story beside the dependent row,
/// pending the lead's disposition), and another player's own
/// filing kept off the author's page (a second filing needs a second shell,
/// rating and filing, which this fixture does not build; the read walks only
/// the actor's own E4 titles, and the other direction is checked).
#[test]
fn command_reader_pages_own_holdings_of_every_kind() -> TestResult {
    let mut f = Fixture::new_with_reads(true, tacsim_reads, true);
    let other = f.engine.world.create(
        mush_core::ObjKind::Player,
        "Other",
        GOD,
        Some(mush_core::LIMBO),
    )?;
    let request = f.request();
    let (prepared, _) = f.engine.prepare_registered_composed_authoritative_action(
        &config().initial_filing_action()?,
        &request,
        token(4),
    )?;
    let (_, install) = prepared.into_parts();
    f.engine.install_composed_action(install)?;
    let charter = CharterId::new(f.shell.0)?;
    let asset = u64::from(f.shell.0);

    let (answer, encoded) = read(&mut f, GOD, json!({"holdings": null}))??;
    let page = answer.into_owner_holdings().ok_or("an owner page")?;
    check_eq!(page.next(), None, "one filing, one page");
    let [row] = page.rows() else {
        return Err(format!("one own row: {encoded}").into());
    };
    check_eq!(row.filed(), FiledRevisionId::new(1)?, "the line's head");
    check_eq!(row.charter(), charter);
    check_eq!(row.owner_licence(), LicenceId::new(1)?);
    check!(!row.dependent());
    check!(!row.selling(), "no funded pending transfer");
    check_eq!(row.name(), "Original");
    check_eq!(row.owner_phase(), OwnerPhase::Active { days_left: 90 });
    for forbidden in ["digest", "account", "payer", "rating", "source", "revision"] {
        check!(!encoded.contains(forbidden), "{forbidden} in {encoded}");
    }
    // The cursor: after an earlier asset id the charter is listed, after its
    // own id nothing is.
    let before = read(&mut f, GOD, json!({"holdings": asset - 1}))??;
    check_eq!(
        before.0.into_owner_holdings().map(|p| p.rows().len()),
        Some(1)
    );
    let after = read(&mut f, GOD, json!({"holdings": asset}))??;
    let after = after.0.into_owner_holdings().ok_or("a page")?;
    check!(after.rows().is_empty() && after.next().is_none());
    // Own-only: another player owns nothing here.
    let theirs = read(&mut f, other, json!({"holdings": null}))??;
    let theirs = theirs.0.into_owner_holdings().ok_or("a page")?;
    check!(theirs.rows().is_empty(), "another player's page is empty");
    // 17g C2g: the paid section, through the per-holder index.
    paid_section_pages_own_rows(&mut f, other)?;

    // Names only, for anyone; an unknown charter is left out.
    for actor in [GOD, other] {
        let (names, encoded) = read(&mut f, actor, json!({"names": [charter.get(), 999_999]}))??;
        let names = names.into_charter_names().ok_or("names")?;
        check_eq!(names.len(), 1);
        check_eq!(names.first().map(CharterName::charter), Some(charter));
        check_eq!(names.first().map(CharterName::name), Some("Original"));
        check_eq!(
            encoded,
            format!(
                r#"{{"holding":null,"next_after":null,"command":{{"kind":"charter_names","names":[{{"charter":"{}","name":"Original"}}]}}}}"#,
                charter.get()
            )
        );
    }
    // The row is the line head's: move the head to a filed number with no
    // row, and the charter drops out (its initial filed revision 1 still
    // exists and would have kept it listed).
    let lines = f.families.lines.clone();
    let line_key = f
        .engine
        .action_state()
        .get(&f.families.filed, &FiledRevisionId::new(1)?.key())?
        .ok_or("the filed row")?
        .content()
        .position()
        .line
        .key();
    let mut moved = f
        .engine
        .action_state()
        .get(&lines, &line_key)?
        .ok_or("the line row")?
        .clone();
    moved.head = FiledRevisionId::new(2)?;
    let mut txn = f.engine.action_transaction(&lines)?;
    txn.replace(&lines, &line_key, moved)?;
    let (_, install) = txn.prepare()?.into_parts();
    f.engine.install_prepared_action(install)?;
    let headless = read(&mut f, GOD, json!({"holdings": null}))??;
    let headless = headless.0.into_owner_holdings().ok_or("a page")?;
    check!(
        headless.rows().is_empty() && headless.next().is_none(),
        "the head is read, not the initial filed revision"
    );
    Ok(())
}

/// 17g C2c (the C2c fix round, reviewer T-F2): `selling` from the charter's
/// pending-transfer head (`funded`): a `Funded` head is a sale; no head and
/// a refunded head are not; a head naming another charter is `Integrity`.
#[test]
fn a_funded_pending_head_is_a_sale() -> TestResult {
    use crate::commerce::charter_pending::records::{PendingHead, PendingRef, PendingState};
    let charter = CharterId::new(40)?;
    let head = |charter, state| PendingHead {
        format: 1,
        charter,
        current: PendingRef {
            sequence: 1,
            operation: token(9),
            digest: mush_core::DefinitionDigest::from_bytes([9; 32]),
        },
        state,
    };
    check_eq!(funded(None, charter), Ok(false), "no head");
    check_eq!(
        funded(Some(&head(charter, PendingState::Funded {})), charter),
        Ok(true)
    );
    let refunded = head(
        charter,
        PendingState::Refunded {
            terminal: token(10),
            reason: mush_economy::managed_charter_transfer::CharterPendingDecisionKindV1::LastClaimClosed {
                next_transition: 1,
            },
        },
    );
    check_eq!(funded(Some(&refunded), charter), Ok(false), "refunded");
    check_eq!(
        funded(
            Some(&head(CharterId::new(41)?, PendingState::Funded {})),
            charter
        ),
        Err(OwnerReadError::Integrity),
        "another charter's head"
    );
    Ok(())
}

/// 17g C2c (the C2c fix round, reviewer T-F2): one `OwnHoldings` read's
/// paging over synthetic assets (`page`): the 3-row cut and its cursor, the
/// 16-asset cut and its cursor (a read whose assets hold no filed mind still
/// goes on), the ends with no cursor, the 17th asset never examined, and an
/// error ending the read.
#[test]
fn own_holdings_page_cuts_at_three_rows_and_sixteen_assets() -> TestResult {
    let ids = |n: u32| (1..=n).map(ObjId).collect::<Vec<_>>();
    let none = |_: ObjId| Ok(None::<u32>);
    check_eq!(
        page(&ids(17), none)?,
        (vec![], Some(16)),
        "no filed mind, more assets"
    );
    check_eq!(
        page(&ids(16), none)?,
        (vec![], None),
        "no filed mind, the end"
    );
    let filed = |o: ObjId| Ok((o.0 != 2).then_some(o.0));
    check_eq!(
        page(&ids(5), filed)?,
        (vec![1, 3, 4], Some(4)),
        "cut at three rows"
    );
    check_eq!(
        page(&ids(4), filed)?,
        (vec![1, 3, 4], None),
        "the third on the last asset"
    );
    let late = |o: ObjId| Ok(matches!(o.0, 14..=16).then_some(o.0));
    check_eq!(page(&ids(17), late)?, (vec![14, 15, 16], Some(16)));
    let two = |o: ObjId| Ok(matches!(o.0, 3 | 9).then_some(o.0));
    check_eq!(
        page(&ids(17), two)?,
        (vec![3, 9], Some(16)),
        "short, more assets"
    );
    check_eq!(page(&ids(9), two)?, (vec![3, 9], None), "short, the end");
    let mut examined = Vec::new();
    page(&ids(17), |o| {
        examined.push(o.0);
        Ok(None::<u32>)
    })?;
    check_eq!(
        examined,
        (1..=16).collect::<Vec<_>>(),
        "the 17th only says more follow"
    );
    check_eq!(
        page(&ids(3), |_| Err::<Option<u32>, _>(
            OwnerReadError::Integrity
        )),
        Err(OwnerReadError::Integrity)
    );
    Ok(())
}

/// The drafted and funded renewal operations of
/// `command_reader_trade_kinds_answer_only_their_own_party`, on the paid
/// licence and term that test writes, built apart from it so its stack frame
/// stays under the M7 memory-size limit (`.github/clippy-m7`).
fn renewal_rows(
    pointer: &crate::commerce::paid::PersonalPaidLicence,
    term: &crate::commerce::paid::PaidTermRecord,
    licence: LicenceId,
    one: crate::commerce::TermGeneration,
    terms: crate::commerce::paid::PaidTerms,
    policy: crate::commerce::records::RevisionPolicy,
    now: i64,
) -> Result<
    (
        crate::commerce::renewal::RenewalOperationV1,
        crate::commerce::renewal::RenewalOperationV1,
    ),
    Box<dyn std::error::Error>,
> {
    use crate::commerce::entitlement::{
        DownstreamTermsV1, FiledRoyaltyTermsV1, GrantEconomicsV1, PaidTermEconomicsV1,
        RenewalTemplateV1,
    };
    use crate::commerce::records::RevisionPolicy;
    use crate::commerce::renewal::{
        RenewalAttemptV1, RenewalBindingV1, RenewalOperationV1, RenewalStateV1,
    };
    use mush_core::mechanics::{
        DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
    };
    use mush_core::{ClockRef, ClockStamp, DefinitionDigest, DefinitionRef};
    let market = |kind: &str, id: &str| -> Result<DefinitionRef, Box<dyn std::error::Error>> {
        Ok(DefinitionRegistryBuilder::new().register(
            OwnerId::new("market")?,
            DefinitionDescriptor::new(
                DefinitionKind::new(kind)?,
                DefinitionId::new(id)?,
                1,
                json!({"format": 1}),
            )?,
        )?)
    };
    let stamp = |at: i64| ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at,
    };
    let RevisionPolicy::Pin { filed, revision } = policy else {
        return Err("a pinned revision".into());
    };
    let chain = FiledRoyaltyTermsV1::capture(filed, revision, token(30), Vec::new())?;
    let grant = GrantEconomicsV1::new(
        DownstreamTermsV1::Source {
            royalty_permille: 0,
        },
        RenewalTemplateV1::new(
            mush_economy::InstrumentId::new("cr").ok_or("instrument")?,
            mush_economy::Amount::from_minor(500),
            90 * DAY,
            3 * DAY,
            terms,
            policy,
            market("entitlement_policy", "personal")?,
        )?,
        chain.reference()?,
    )?;
    let economics = PaidTermEconomicsV1::capture(
        licence,
        one,
        token(30),
        DefinitionDigest::from_bytes([9; 32]),
        grant,
    )?;
    let drafted = RenewalOperationV1 {
        format: 1,
        operation: token(31),
        action: market("action", "draft_mind_renewal")?,
        attempt: RenewalAttemptV1::Automatic {},
        binding: Box::new(RenewalBindingV1::capture(pointer, term, &economics)?),
        offer_digest: DefinitionDigest::from_bytes([31; 32]),
        contract: serde_json::from_value(json!(31))?,
        consent_hash: [31; 32],
        created_at_ic: stamp(now),
        created_at_utc: now,
        state: RenewalStateV1::Drafted {},
    };
    drafted.validate()?;
    let funded = RenewalOperationV1 {
        state: RenewalStateV1::Funded {
            funding: serde_json::from_value(json!({
                "format": 1,
                "commit": token(32),
                "action": market("action", "fund_mind_renewal")?,
                "contract": drafted.contract,
                "hold": 32,
                "transaction": 32,
                "journal_seq": 32,
                "consent_hash": drafted.consent_hash,
                "at_ic": stamp(now + 1),
                "posted_at_utc": now + 1
            }))?,
        },
        ..drafted.clone()
    };
    funded.validate()?;
    Ok((drafted, funded))
}

/// 17g C2d test 4 (C2 plan §2.4, §2.6), over the real filing fixture with a
/// paid licence pointer and its current term written beside the filing:
/// - `OwnPaid`: the holder reads the licence's charter, public name, form,
///   transferability, Active phase in whole IC days, no revocation, no
///   resale, no renewal and not a House Pin; another player naming that
///   licence and the holder naming a licence that does not exist both read
///   `Refused`, identically; no answer carries a digest, account, rating,
///   source or revision;
/// - the renewal mark on the holder's paid view, through real rows (the C2d
///   fix round, reviewers F2 and T3): a personal renewal head whose pending
///   operation is drafted reads `Offered`, the same operation funded reads
///   `Paid`, and the foreign and missing reads stay `Refused` alike while
///   it is pending; `renewal_mark` maps any other phase to none.
///
/// Not proven here: the House Pin flag through real rows (mush-core refuses
/// a House allocation row outside a House purchase invocation; it is one
/// point read of `house_allocations`, and the House renewal head is read
/// only when that row exists; `paid_section_pages_own_rows`' doc). The
/// revoked flag is `paid_section_pages_own_rows`' (17g C2g), selling owed
/// as its doc says;
/// `CharterOffers` is `charter_offers_answer_only_the_named_buyer`'s and the
/// named buyer's resale step `resale_step_on_a_granted_transferable_licence`'s.
#[test]
fn command_reader_trade_kinds_answer_only_their_own_party() -> TestResult {
    use crate::commerce::paid::{PaidForm, PaidTermRecord, PaidTerms, PersonalPaidLicence};
    use crate::commerce::rating::RatingClass;
    use crate::commerce::records::RevisionPolicy;
    use crate::commerce::{TermGeneration, TermWindow};
    let mut f = Fixture::new_with_reads(true, tacsim_reads, true);
    let other = f.engine.world.create(
        mush_core::ObjKind::Player,
        "Other",
        GOD,
        Some(mush_core::LIMBO),
    )?;
    let request = f.request();
    let (prepared, _) = f.engine.prepare_registered_composed_authoritative_action(
        &config().initial_filing_action()?,
        &request,
        token(4),
    )?;
    let (_, install) = prepared.into_parts();
    f.engine.install_composed_action(install)?;
    let filed = FiledRevisionId::new(1)?;
    let content = f
        .engine
        .action_state()
        .get(&f.families.filed, &filed.key())?
        .ok_or("the filed row")?
        .content()
        .clone();
    let charter = content.position().charter;
    // A paid licence the author (God) holds on that charter: an open,
    // non-transferable personal term, Active for 90 IC days from now.
    let licence = LicenceId::new(41)?;
    let one = TermGeneration::new(1)?;
    let now = f.engine.world.game_now_unix();
    let window = TermWindow::new(
        WorldIcSeconds(now),
        WorldIcSeconds(now),
        WorldIcSeconds(now + 90 * DAY),
        WorldIcSeconds(now + 93 * DAY),
    )?;
    let terms = PaidTerms::original_personal_form(RatingClass::I, PaidForm::Open {}, false)?;
    let pointer = PersonalPaidLicence::original(
        licence,
        charter,
        content.position().line,
        GOD,
        one,
        token(30),
    )?;
    let term = PaidTermRecord::original(
        licence,
        one,
        window,
        terms,
        RevisionPolicy::Pin {
            filed,
            revision: content.revision(),
        },
        token(30),
    )?;
    let (licences, paid_terms) = (
        f.families.paid_licences.clone(),
        f.families.paid_terms.clone(),
    );
    let mut txn = f.engine.action_transaction(&licences)?;
    txn.insert(&licences, &licence.key(), pointer.clone())?;
    txn.insert(
        &paid_terms,
        &format!("{}/{}", licence.key(), one.key()),
        term.clone(),
    )?;
    let (_, install) = txn.prepare()?.into_parts();
    f.engine.install_prepared_action(install)?;

    // OwnPaid: the holder's own view.
    let (answer, encoded) = read(&mut f, GOD, json!({"paid": 41}))??;
    let view = answer.into_own_paid().ok_or("an own paid view")?;
    check_eq!(view.licence(), licence);
    check_eq!(view.charter(), charter);
    check!(!view.house(), "a personal licence");
    check_eq!(view.name(), "Original");
    check_eq!(view.form(), PaidForm::Open {});
    check!(!view.transferable(), "the term's own mark");
    check_eq!(view.phase(), OwnerPhase::Active { days_left: 90 });
    check!(!view.revoked() && !view.selling());
    check_eq!(view.renewal(), None);
    check_eq!(
        view.renewal_operation(),
        None,
        "17g C2h: no pending renewal"
    );
    check!(
        encoded.contains(r#""command":{"kind":"own_paid""#),
        "{encoded}"
    );
    for forbidden in ["digest", "account", "payer", "rating", "source", "revision"] {
        check!(!encoded.contains(forbidden), "{forbidden} in {encoded}");
    }
    // Foreign and missing read alike.
    let foreign = read(&mut f, other, json!({"paid": 41}))?;
    let missing = read(&mut f, GOD, json!({"paid": 999}))?;
    check_eq!(foreign, Err(OwnerReadError::Refused));
    check_eq!(missing, Err(OwnerReadError::Refused));

    // The renewal mark through real rows: a personal renewal head whose
    // pending operation is the drafted offer on this term, then funded.
    let policy = RevisionPolicy::Pin {
        filed,
        revision: content.revision(),
    };
    let (drafted, funded) = renewal_rows(&pointer, &term, licence, one, terms, policy, now)?;
    let (heads, operations) = (
        f.families.renewal_heads.clone(),
        f.families.renewal_operations.clone(),
    );
    for (row, mark, fresh) in [
        (&drafted, RenewalMark::Offered, true),
        (&funded, RenewalMark::Paid, false),
    ] {
        let head = crate::commerce::renewal::RenewalHeadV1::for_operation(row)?;
        check_eq!(head.pending(), Some(row.operation));
        let mut txn = f.engine.action_transaction(&heads)?;
        if fresh {
            txn.insert(&heads, &licence.key(), head)?;
            txn.insert(&operations, &row.operation.to_hex(), row.clone())?;
        } else {
            txn.replace(&heads, &licence.key(), head)?;
            txn.replace(&operations, &row.operation.to_hex(), row.clone())?;
        }
        let (_, install) = txn.prepare()?.into_parts();
        f.engine.install_prepared_action(install)?;
        let (answer, encoded) = read(&mut f, GOD, json!({"paid": 41}))??;
        let view = answer.into_own_paid().ok_or("an own paid view")?;
        check_eq!(view.renewal(), Some(mark), "{encoded}");
        // 17g C2h: the head's pending operation, the token `+minds/renew/pay`
        // and `/decline` bind to, as a value only (never encoded below).
        check_eq!(view.renewal_operation(), Some(row.operation));
        check!(!view.house(), "a personal licence reads the personal head");
        for forbidden in ["digest", "account", "contract", "consent", "operation"] {
            check!(!encoded.contains(forbidden), "{forbidden} in {encoded}");
        }
        check_eq!(
            read(&mut f, other, json!({"paid": 41}))?,
            Err(OwnerReadError::Refused)
        );
        check_eq!(
            read(&mut f, GOD, json!({"paid": 999}))?,
            Err(OwnerReadError::Refused)
        );
    }

    // The mark of each pending phase.
    check_eq!(renewal_mark(true, false), Some(RenewalMark::Offered));
    check_eq!(renewal_mark(false, true), Some(RenewalMark::Paid));
    check_eq!(renewal_mark(false, false), None);
    check_eq!(
        serde_json::to_value(RenewalMark::Offered)?,
        json!("offered")
    );
    Ok(())
}

/// 17g C2g (C2c test 3's paid, revoked and neighbour cases, through
/// `command_reader_pages_own_holdings_of_every_kind`): over the real filing
/// fixture, the paid licences 41 to 44 held by the author (God) and 45 by
/// another player, each a pointer, its current term and its per-holder index
/// row written beside the filing, and 42's current term revoked by the
/// Registry (its `roll_registry_revoked_terms` row):
/// - the author's paid section pages three rows at a time in licence order
///   (`PAID_ROWS_PER_READ`), the cursor the last row's licence, and the
///   second page ends with no cursor; the neighbour's row (the next key
///   after the author's prefix) never appears;
/// - each row is exactly `OwnPaid`'s view of that licence, with `revoked`
///   on 42 only and no row selling;
/// - the neighbour reads only 45, a third player nothing;
/// - no answer carries a digest, account, rating, source or revision.
///
/// Not fixtured here, although C2c test 3 names them (the C2g fix round,
/// reviewers C2G-2, F4 and T3; the brief exempted only the dependent row):
/// - `selling` through a real row: mush-core refuses a funded resale head
///   written outside the paid-resale protocol (`terminal_capacity::
///   paid_resale::transition`, "resale writes without protocol", the head
///   being one of the protocol's own families), as it refuses C2c's pending
///   head; the flag is one point read of `paid_resale_heads` (`own_paid`);
/// - the House Pin row (`house`, and the House renewal head read only
///   then): mush-core refuses any `roll_mind_house_purchase` write outside
///   an actual House purchase invocation (`terminal_capacity::house::
///   validate_changes`: "house writes require registered protocol" without
///   the protocol, "purchase write lacks actual invocation" with it), so
///   this transaction, which carries no invocation, cannot hold the
///   `house_allocations` row `own_paid` reads by the birth operation.
///
/// Both are owed to the C2h story (a funded resale's holder reading
/// `; selling` in story part 3; a House Pin holder's row reading `House
/// Pin`, which needs a House purchase in a story), pending the lead's
/// disposition.
fn paid_section_pages_own_rows(f: &mut Fixture, other: ObjId) -> TestResult {
    use crate::commerce::paid::{PaidForm, PaidTermRecord, PaidTerms, PersonalPaidLicence};
    use crate::commerce::paid_holdings::PaidHolding;
    use crate::commerce::rating::RatingClass;
    use crate::commerce::records::RevisionPolicy;
    use crate::commerce::{TermGeneration, TermWindow};
    let filed = FiledRevisionId::new(1)?;
    let content = f
        .engine
        .action_state()
        .get(&f.families.filed, &filed.key())?
        .ok_or("the filed row")?
        .content()
        .clone();
    let one = TermGeneration::new(1)?;
    let now = f.engine.world.game_now_unix();
    let window = TermWindow::new(
        WorldIcSeconds(now),
        WorldIcSeconds(now),
        WorldIcSeconds(now + 90 * DAY),
        WorldIcSeconds(now + 93 * DAY),
    )?;
    let terms = PaidTerms::original_personal_form(RatingClass::I, PaidForm::OpenResale {}, true)?;
    let families = f.families.clone();
    let mut txn = f.engine.action_transaction(&families.paid_licences)?;
    for (n, holder) in [(41, GOD), (42, GOD), (43, GOD), (44, GOD), (45, other)] {
        let licence = LicenceId::new(n)?;
        let birth = token(u8::try_from(n)?);
        let pointer = PersonalPaidLicence::original(
            licence,
            content.position().charter,
            content.position().line,
            holder,
            one,
            birth,
        )?;
        let term = PaidTermRecord::original(
            licence,
            one,
            window,
            terms,
            RevisionPolicy::Pin {
                filed,
                revision: content.revision(),
            },
            birth,
        )?;
        txn.insert(&families.paid_licences, &licence.key(), pointer)?;
        txn.insert(
            &families.paid_terms,
            &format!("{}/{}", licence.key(), one.key()),
            term,
        )?;
        let row = PaidHolding::new(holder, licence)?;
        txn.insert(&families.paid_holdings, &row.key(), row.clone())?;
    }
    let (_, install) = txn.prepare()?.into_parts();
    f.engine.install_prepared_action(install)?;
    // 42's current term revoked.
    let revoked = crate::commerce::registry_revocation::records::RevokedTerm {
        format: 1,
        licence: LicenceId::new(42)?,
        generation: one,
        operation: token(60),
        term_digest: mush_core::DefinitionDigest::from_bytes([6; 32]),
        window,
    };
    let mut txn = f.engine.action_transaction(&families.revoked_terms)?;
    txn.insert(&families.revoked_terms, &revoked.key(), revoked.clone())?;
    let (_, install) = txn.prepare()?.into_parts();
    f.engine.install_prepared_action(install)?;

    /// One paid page: its licence numbers, its cursor and its encoding.
    type Page = (Vec<u64>, Option<u64>, String);
    let page = |f: &mut Fixture, actor, after: Value| -> Result<Page, Box<dyn std::error::Error>> {
        let (answer, encoded) = read(f, actor, json!({"holdings": after, "paid": true}))??;
        let page = answer.into_paid_holdings().ok_or("a paid page")?;
        Ok((
            page.rows().iter().map(|r| r.licence().get()).collect(),
            page.next().map(LicenceId::get),
            encoded,
        ))
    };
    let (first, next, encoded) = page(f, GOD, Value::Null)?;
    check_eq!(
        (first, next),
        (vec![41, 42, 43], Some(43)),
        "three rows, then the cursor"
    );
    for forbidden in ["digest", "account", "payer", "rating", "source", "revision"] {
        check!(!encoded.contains(forbidden), "{forbidden} in {encoded}");
    }
    let (second, next, _) = page(f, GOD, json!(43))?;
    check_eq!(
        (second, next),
        (vec![44], None),
        "the neighbour's 45 never appears"
    );
    let (theirs, next, _) = page(f, other, Value::Null)?;
    check_eq!(
        (theirs, next),
        (vec![45], None),
        "the neighbour reads its own"
    );
    let third = f.engine.world.create(
        mush_core::ObjKind::Player,
        "Third",
        GOD,
        Some(mush_core::LIMBO),
    )?;
    let (none, next, _) = page(f, third, Value::Null)?;
    check!(
        none.is_empty() && next.is_none(),
        "a third player holds nothing"
    );
    // Each row is `OwnPaid`'s view; the flags are the licence's own.
    let (answer, _) = read(f, GOD, json!({"holdings": null, "paid": true}))??;
    let rows = answer.into_paid_holdings().ok_or("a paid page")?;
    for row in rows.rows() {
        let n = row.licence().get();
        let (own, _) = read(f, GOD, json!({"paid": n}))??;
        check_eq!(Some(row.clone()), own.into_own_paid(), "licence {n}");
        check_eq!(
            (row.revoked(), row.selling(), row.house(), row.renewal()),
            (n == 42, false, false, None),
            "licence {n}"
        );
        check!(row.transferable() && row.name() == "Original");
        check_eq!(row.phase(), OwnerPhase::Active { days_left: 90 });
    }
    Ok(())
}

/// 17g C2g (the C2e plan §4.1 item 4, C2d test 2's Roll twin): `CharterOffers`
/// answers only the live pending-charter offers naming the actor on the
/// filed mind's charter.
/// - Over real pending offer rows (`charter_pending`'s `offer` fixture; a
///   pending offer row cannot be written outside the composed protocol, so
///   `named_offers`, the read's own answer, is called with them): the
///   actor's two live offers are listed with their frozen prices and
///   expiries, its offer at its expiry is not, no row is the empty list, and
///   a row naming another buyer's offer, or an offer on another charter, is
///   `Integrity`;
/// - through the real read on the filing fixture: a filed number that does
///   not exist is `Refused`, the real filed mind with no row naming the
///   actor is the empty list, for the author and another player alike (the
///   command prints both as `NO_CHARTER_OFFER`), and the encoding carries no
///   seller, account or digest.
#[test]
fn charter_offers_answer_only_the_named_buyer() -> TestResult {
    use crate::commerce::charter_pending::records::{PendingBuyerOffers, PendingOffer};
    use crate::commerce::charter_pending::tests::buyer_offers::offer;
    let me = ObjId(8);
    let charter = CharterId::new(100)?;
    let now = 5_000;
    let cheap = offer(10, me, 100, now - 400)?; // expires now + 600
    let mut dear_value = serde_json::to_value(offer(11, me, 100, now)?)?;
    dear_value["price"] = json!(500);
    let dear: PendingOffer = serde_json::from_value(dear_value)?;
    let lapsed = offer(12, me, 100, now - 1_000)?; // expires now
    let theirs = offer(13, ObjId(9), 100, now)?;
    let elsewhere = offer(14, me, 101, now)?;
    let rows: std::collections::BTreeMap<String, PendingOffer> =
        [&cheap, &dear, &lapsed, &theirs, &elsewhere]
            .into_iter()
            .map(|o| (o.operation.to_hex(), o.clone()))
            .collect();
    let index = |offers: &[&PendingOffer]| PendingBuyerOffers {
        format: 1,
        buyer: me,
        charter,
        offers: offers.iter().map(|o| o.operation).collect(),
    };
    let lookup = |keys: &[String]| -> Result<Vec<Option<&PendingOffer>>, OwnerReadError> {
        Ok(keys.iter().map(|k| rows.get(k)).collect())
    };
    let mine = named_offers(
        me,
        charter,
        Some(&index(&[&cheap, &dear, &lapsed])),
        now,
        lookup,
    )?;
    check_eq!(
        mine.iter()
            .map(|v| (v.offer(), v.price_minor(), v.expires_at_utc()))
            .collect::<Vec<_>>(),
        vec![
            (cheap.operation, cheap.price.minor(), now + 600),
            (dear.operation, 500, now + 1_000),
        ],
        "the two live offers naming me; the lapsed one is left out"
    );
    check_eq!(named_offers(me, charter, None, now, lookup)?, Vec::new());
    for crossed in [index(&[&theirs]), index(&[&elsewhere])] {
        check_eq!(
            named_offers(me, charter, Some(&crossed), now, lookup),
            Err(OwnerReadError::Integrity)
        );
    }
    let serialized = serde_json::to_string(&RollCommandReadResult::CharterOffers { offers: mine })?;
    for forbidden in ["seller", "account", "digest", "buyer"] {
        check!(
            !serialized.contains(forbidden),
            "{forbidden} in {serialized}"
        );
    }

    // Through the real read.
    let mut f = Fixture::new_with_reads(true, tacsim_reads, true);
    let other = f.engine.world.create(
        mush_core::ObjKind::Player,
        "Other",
        GOD,
        Some(mush_core::LIMBO),
    )?;
    check_eq!(
        read(&mut f, GOD, json!({"charter_offers": 1}))?,
        Err(OwnerReadError::Refused),
        "no filed mind 1 yet"
    );
    let request = f.request();
    let (prepared, _) = f.engine.prepare_registered_composed_authoritative_action(
        &config().initial_filing_action()?,
        &request,
        token(4),
    )?;
    let (_, install) = prepared.into_parts();
    f.engine.install_composed_action(install)?;
    let mut answers = Vec::new();
    for actor in [GOD, other] {
        let (answer, encoded) = read(&mut f, actor, json!({"charter_offers": 1}))??;
        check_eq!(
            answer.into_charter_offers(),
            Some(Vec::new()),
            "#{}",
            actor.0
        );
        answers.push(encoded);
    }
    check_eq!(answers.first(), answers.get(1), "alike for every actor");
    check_eq!(
        read(&mut f, other, json!({"charter_offers": 99}))?,
        Err(OwnerReadError::Refused)
    );
    Ok(())
}

/// 17g C2g (the C2e plan §4.1 item 4): the named buyer's resale step
/// (`paid_resale::reader::ResaleStep`), by the provider's own claim test,
/// on a granted transferable licence from the renewal rig's `birth` (born
/// transferable and renewed once, so the term the step is asked for is a
/// granted generation the resale reader admits: `require_transferable`):
/// - no open paid claim is `Accept`, one or more is `Fund`, whatever the
///   claim census's generation; an observation that does not validate is
///   refused;
/// - `resale_step` over a world reads Roll's own live claim census, keyed
///   by the licence alone: a world with no Roll store is `Unavailable`; the
///   filing fixture's ready Roll store, whose IC claims are unwired as in
///   production, answers `Accept`; with a census holding one open paid-seat
///   claim on the granted licence (`ic_claims::tests::row`, installed by
///   `RollStore::replace_ic_claims_for_tests`), that licence is `Fund` and
///   another licence stays `Accept`, so the step is the asked licence's own.
///
/// The step reads no pointer, term or offer: whether the licence is held,
/// granted and transferable is the resale provider's own `observe` at
/// preparation, and the market read asks for the step only on a licence it
/// found an offer naming the actor on (`mush_market::mind_commands`).
#[test]
fn resale_step_on_a_granted_transferable_licence() -> TestResult {
    use crate::commerce::entitlement::writer::EntitlementPurchaseTerms;
    use crate::commerce::paid::PaidForm;
    use crate::commerce::paid_resale::ResaleClaimObservation;
    use crate::commerce::paid_resale::reader::{ResaleStep, require_transferable, resale_step};
    use crate::commerce::rating::RatingClass;
    use crate::commerce::records::PublicationError;
    use crate::commerce::renewal::RenewalAttemptV1;
    use crate::commerce::renewal::tests::{birth_with, draft, funded, granted_birth, terminal};
    let terms =
        EntitlementPurchaseTerms::personal_terms(RatingClass::I, PaidForm::OpenResale {}, true)?;
    let born = birth_with(terms)?;
    let renewed = granted_birth(&terminal(
        &funded(&draft(&born, 10, 20, RenewalAttemptV1::Automatic {})?)?,
        true,
    )?)?;
    check_eq!(renewed.term.generation().get(), 2, "a granted generation");
    require_transferable(&renewed.term)?;
    let licence = renewed.licence.id();
    let claims = |generation, next_transition, open_paid| ResaleClaimObservation {
        generation,
        next_transition,
        open_paid,
    };
    check_eq!(
        ResaleStep::of(&claims(0, 1, 0)),
        Ok(ResaleStep::Accept),
        "unwired"
    );
    check_eq!(ResaleStep::of(&claims(3, 9, 0)), Ok(ResaleStep::Accept));
    check_eq!(ResaleStep::of(&claims(3, 9, 1)), Ok(ResaleStep::Fund));
    check_eq!(ResaleStep::of(&claims(3, 9, 7)), Ok(ResaleStep::Fund));
    check_eq!(
        ResaleStep::of(&claims(0, 1, 1)),
        Err(PublicationError::Invalid),
        "no open claim without a census"
    );
    check_eq!(serde_json::to_value(ResaleStep::Fund)?, json!("fund"));
    check_eq!(
        resale_step(&World::new_minimal(), licence),
        Err(PublicationError::Unavailable),
        "no Roll store"
    );
    let mut f = Fixture::new_with_reads(true, tacsim_reads, true);
    check_eq!(
        resale_step(&f.engine.world, licence),
        Ok(ResaleStep::Accept),
        "Roll's own census, claims unwired"
    );
    {
        use crate::commerce::ic_claims::ClaimInventory;
        use crate::commerce::ic_claims::tests::{one_seat, row};
        let census = ClaimInventory::reconstruct(vec![row(1, licence.get(), one_seat()?)?])?;
        check_eq!(census.paid_open(licence), 1);
        f.engine
            .world
            .module_state_mut::<crate::RollStore>()
            .ok_or("the Roll store")?
            .replace_ic_claims_for_tests(Some(census));
    }
    check_eq!(
        resale_step(&f.engine.world, licence),
        Ok(ResaleStep::Fund),
        "an open paid claim on the granted licence"
    );
    let another = LicenceId::new(licence.get().checked_add(1).ok_or("licence")?)?;
    check_eq!(
        resale_step(&f.engine.world, another),
        Ok(ResaleStep::Accept),
        "no claim on another licence"
    );
    Ok(())
}
