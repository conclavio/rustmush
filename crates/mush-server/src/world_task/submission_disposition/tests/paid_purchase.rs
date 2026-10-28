//! Required expensive actual-server custody gate. All business evidence is
//! created through live public commands/offers and the genuine Registry worker.
//! No direct provider preparation, manual ACK, or test certificate is used.
// 17g C1a: the player-market roster test (`player_market.rs`) boots through
// this same harness and store; only visibility is widened, no behaviour.
pub(super) mod runtime;
pub(super) mod store;

use mush_config::config::Config;
use mush_core::{CommitToken, GOD, ObjId};
use mush_economy::{Account, AccountId as WalletId, Amount, CatalogRef, InstrumentId};
use mush_market::{
    MarketId,
    mind_trade::{
        OriginalDuration, OriginalPrice, OriginalQuotePolicy,
        records::{MindListing, MindQuote, QuoteVenue},
    },
};
use mush_org::{PrincipalId, SystemPrincipalId};
use mush_roll::commerce::{
    filing::InitialFilingTerms,
    module::CommerceMode,
    rating::{
        RatingJob, RatingJobState, RatingPage, RatingReceipt, RatingRef, execution::RatingExecution,
    },
};
use mush_store::{AccountRec, AuthoritativeStore, BundleStatus, CharacterRef, ObjectStore};
use runtime::{Running, STEP, field, now, one_row, rows, wall_after};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use store::{Exact, Fault, PurchaseFaultStore};
use tacsim::{TacsimRuleset, composition::mind_commerce::MindCommerce};
use tokio::time::{Instant, timeout};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
const PRICE: i64 = 100;
const QUOTE_SECONDS: i64 = 30;
// A finite hang guard, not a pace requirement (timing is report-only under
// the owner's waiver): the live server paces a Registry lease through its
// maintenance ticks (about six a lease; `mind_pulse`), and this certificate
// takes ~1,070 of them: about three hours at production's 1,000 ms pulse,
// minutes at the stories' `MIND_PULSE_MS` default.
const REGISTRY_GUARD: Duration = Duration::from_secs(14_400);
/// The stories' maintenance pulse when `MIND_PULSE_MS` is unset: a lease
/// about a third of a second, a certificate about six minutes.
const STORY_PULSE_MS: u64 = 50;
/// Production's pulse, `MIND_PULSE_MS=1000`: the three-hour run.
const PRODUCTION_PULSE_MS: u64 = 1_000;

/// Sets the proving service's maintenance pulse
/// (`[tacsim.minds]`'s code-only `bulk_maintenance_ms`, the
/// `mind-bulk-maintenance` tick's period) for a story's server: the
/// `MIND_PULSE_MS` environment variable when it is set and not empty, else
/// [`STORY_PULSE_MS`]. Every Registry lease crosses about six pulses, so this
/// is the pace of the genuine rating and nothing else: no ledger, quota,
/// quote expiry or IC clock reads it. `MIND_PULSE_MS=1000` is production's
/// pace, the nightly schedule's in `ci.yml` (the precedent is `MINE_RATE`,
/// the bot mining rigs' world rate); an empty value is the workflow's unset
/// dispatch input, the fast path.
pub(super) fn mind_pulse(config: &mut Config) {
    let pulse = match std::env::var("MIND_PULSE_MS") {
        Ok(raw) if !raw.trim().is_empty() => raw
            .trim()
            .parse::<u64>()
            .ok()
            .filter(|ms| (10..=PRODUCTION_PULSE_MS).contains(ms))
            .unwrap_or_else(|| {
                panic!(
                    "MIND_PULSE_MS is not a maintenance pulse in milliseconds, 10 to 1000: {raw:?}"
                )
            }),
        _ => STORY_PULSE_MS,
    };
    config.tacsim.minds.bulk_maintenance_ms = pulse;
}

fn config() -> Config {
    let mut config = Config::default();
    config.tacsim.enabled = true;
    config.store.store = "memory".into();
    config.game.ic_ratio = 24;
    mind_pulse(&mut config);
    config
}
fn issuer() -> TestResult<SystemPrincipalId> {
    SystemPrincipalId::new("roll.registry").ok_or_else(|| "Registry issuer".into())
}
fn composition(config: &Config, mode: CommerceMode, sink: WalletId) -> TestResult<MindCommerce> {
    let cfg = crate::config_ext::tacsim_config(&config.tacsim)?;
    let layer = TacsimRuleset::with_game_content(cfg.clone(), cfg.enabled);
    let commerce = MindCommerce::prepare(
        &layer,
        mode,
        InitialFilingTerms::new(90 * 86_400, 3 * 86_400)?,
        mush_economy::registry_filing::InitialFilingPolicy::new(
            issuer()?,
            sink,
            Amount::from_minor(100),
            CatalogRef {
                ruleset: "roll".into(),
                code: "mind_charter".into(),
            },
        )?,
    )?;
    if matches!(mode, CommerceMode::Initialize) {
        return Ok(commerce);
    }
    Ok(commerce.with_original_purchases(OriginalQuotePolicy::new(
        OriginalPrice::new(
            Amount::from_minor(PRICE),
            InstrumentId::new("tacsim.credit").ok_or("credit instrument")?,
            issuer()?,
            sink,
            100,
        )?,
        QuoteVenue::new(
            MarketId::new("ceres").ok_or("Ceres venue")?,
            PrincipalId::System(issuer()?),
            sink,
            0,
        )?,
        OriginalDuration::new(true, 90 * 86_400, 3 * 86_400, QUOTE_SECONDS)?,
    )?)?)
}

pub(in crate::world_task::submission_disposition::tests) async fn auth_account(
    store: &PurchaseFaultStore,
    name: &str,
    character: Option<ObjId>,
) -> TestResult<mush_core::AccountId> {
    let created = now()?;
    let id = mush_core::AccountId::mint(name, created)?;
    store
        .create_account(AccountRec {
            id: Some(id),
            name: name.into(),
            argon2_hash: "fixture-authenticated-boundary-no-password-login".into(),
            created,
            characters: character
                .into_iter()
                .map(|id| CharacterRef {
                    id,
                    name: "God".into(),
                })
                .collect(),
            ssh_keys: Vec::new(),
            discord: None,
        })
        .await?;
    Ok(id)
}
async fn wallet(store: &PurchaseFaultStore, id: WalletId) -> TestResult<Account> {
    let row = store
        .module_json_get("economy", &format!("account:{}", id.0))
        .await?
        .ok_or("actual economic wallet")?;
    Ok(serde_json::from_value(row)?)
}
pub(in crate::world_task::submission_disposition::tests) async fn open_wallet(
    server: &Running,
    store: &PurchaseFaultStore,
    owner: PrincipalId,
) -> TestResult<WalletId> {
    let command = match &owner {
        PrincipalId::Object(id) => format!("@account/open #{}=tacsim.credit", id.0),
        PrincipalId::System(id) => format!("@account/system {}=tacsim.credit", id.as_str()),
        _ => return Err("fixture requires actual personal/system wallet".into()),
    };
    let lines = server.command(GOD, command).await?;
    let ids = lines
        .iter()
        .filter_map(|line| line.split_once("Opened account #"))
        .filter_map(|(_, tail)| tail.split_whitespace().next())
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()?;
    let [id]: [u64; 1] = ids
        .try_into()
        .map_err(|_| "one acknowledged wallet creation")?;
    let row = wallet(store, WalletId(id)).await?;
    assert_eq!(row.owner, owner);
    assert_eq!(
        row.instrument,
        InstrumentId::new("tacsim.credit").ok_or("instrument")?
    );
    assert!(row.scope.is_none());
    Ok(row.id)
}

async fn saved_revision(
    server: &Running,
    store: &PurchaseFaultStore,
) -> TestResult<mush_roll::Revision> {
    for command in [
        "@mind/new ServerCustody",
        "@mind/line ServerCustody 2=fn think(v,c){stock::rules(); speed(0); this.landed=t_ms();}",
        "@mind/delete ServerCustody 3-3",
        "@mind/save ServerCustody=actual server custody gate",
    ] {
        server.command(GOD, command.into()).await?;
    }
    let end = Instant::now() + STEP;
    loop {
        let candidates = store
            .module_json_list("roll")
            .await?
            .into_iter()
            .filter(|(key, _)| key.starts_with(&format!("revision:{}:", GOD.0)))
            .map(|(_, row)| serde_json::from_value::<mush_roll::Revision>(row))
            .collect::<Result<Vec<_>, _>>()?;
        if let [revision] = candidates.as_slice() {
            return Ok(revision.clone());
        }
        if !candidates.is_empty() {
            return Err("unexpected multiple saved source revisions".into());
        }
        if Instant::now() >= end {
            return Err("public workshop save did not publish revision".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn genuine_rating(
    server: &Running,
    store: &PurchaseFaultStore,
    revision: &mush_roll::Revision,
) -> TestResult<RatingRef> {
    let reference = revision.reference();
    server
        .command(
            GOD,
            format!(
                "@registry/rate {}/{}",
                reference.key.mind, reference.key.number
            ),
        )
        .await?;
    let end = Instant::now() + REGISTRY_GUARD;
    let mut report = Instant::now();
    loop {
        let jobs = rows(store, "roll", "rating_jobs").await?;
        if let [row] = jobs.as_slice() {
            let job: RatingJob = serde_json::from_value(row.clone())?;
            match job.state() {
                RatingJobState::Completed { receipt: expected } => {
                    let receipt: RatingReceipt =
                        serde_json::from_value(one_row(store, "roll", "rating_receipts").await?)?;
                    let mut pages = rows(store, "roll", "rating_pages")
                        .await?
                        .into_iter()
                        .map(serde_json::from_value::<RatingPage>)
                        .collect::<Result<Vec<_>, _>>()?;
                    pages.sort_by_key(RatingPage::key);
                    receipt.verify_pages(&pages)?;
                    let actual = receipt.reference()?;
                    assert_eq!(&actual, expected);
                    assert_eq!(
                        receipt.subject().revision(),
                        mush_roll::commerce::RevisionIdentity::capture(revision)?
                    );
                    eprintln!(
                        "M7_SERVER_REGISTRY_COMPLETE job={} pages={}",
                        actual.job(),
                        pages.len()
                    );
                    return Ok(actual);
                }
                RatingJobState::Queued {} | RatingJobState::Running { .. } => {}
                other => return Err(format!("actual Registry terminal refusal: {other:?}").into()),
            }
        } else if jobs.len() > 1 {
            return Err("one public Registry job expected".into());
        }
        if Instant::now() >= end {
            return Err("genuine server Registry finite hang guard".into());
        }
        if report.elapsed() >= Duration::from_secs(30) {
            // Bounded stage metadata only. No source, seals, card or receipt bodies.
            eprintln!(
                "M7_SERVER_REGISTRY_WAIT jobs={} seconds_remaining={}",
                jobs.len(),
                end.saturating_duration_since(Instant::now()).as_secs()
            );
            // Fixed enum/count metadata only: distinguish work progressing from
            // a live timer waiting on an unacknowledged owner transition.
            for row in &jobs {
                let job: RatingJob = serde_json::from_value(row.clone())?;
                let phase = match job.state() {
                    RatingJobState::Queued {} => "queued",
                    RatingJobState::Running { .. } => "running",
                    RatingJobState::Completed { .. } => "completed",
                    RatingJobState::Nonconforming { .. } => "nonconforming",
                    RatingJobState::Failed { .. } => "failed",
                    RatingJobState::Interrupted {} => "interrupted",
                    RatingJobState::Cancelled {} => "cancelled",
                };
                eprintln!("M7_SERVER_REGISTRY_JOB phase={phase}");
            }
            let executions = rows(store, "roll", "rating_executions").await?;
            eprintln!("M7_SERVER_REGISTRY_EXECUTIONS count={}", executions.len());
            for row in executions.into_iter().take(2) {
                let execution: RatingExecution = serde_json::from_value(row)?;
                let observed = execution.observation();
                eprintln!(
                    "M7_SERVER_REGISTRY_PROGRESS phase={:?} next={} case={} simulated_ms={} charged_unit_ms={} flight={} result={}",
                    observed.phase,
                    observed.next_sequence,
                    observed.cursor.index,
                    observed.cursor.simulated_ms,
                    observed.charged_unit_ms,
                    observed.flight.is_some(),
                    observed.result.is_some()
                );
            }
            report = Instant::now();
        }
        server.drain_output()?;
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

async fn file_and_list(
    server: &Running,
    store: &PurchaseFaultStore,
    payer: WalletId,
    revision: &mush_roll::Revision,
    rating: &RatingRef,
) -> TestResult<CommitToken> {
    server
        .offer(GOD, "roll-reserve-mind-charter", json!({}))
        .await?;
    let shell = one_row(store, "roll", "initial_filing_shells").await?;
    let request = field(field(&shell, "key")?, "request")?;
    let reference = revision.reference();
    server.offer(GOD,"roll-file-original-mind",json!({
        "shell_request":request, "mind":reference.key.mind, "revision":reference.key.number,
        "revision_digest":reference.digest, "rating_job":rating.job(), "rating_id":rating.id(),
        "rating_digest":rating.receipt_digest(), "payer":payer.0, "charter_name":"Server custody charter"
    })).await?;
    let owner = one_row(store, "roll", "roll_mind_owner_licence").await?;
    server
        .offer(
            GOD,
            "market-original-mind-list",
            json!({"owner_licence":field(&owner,"id")?}),
        )
        .await?;
    let listing: MindListing = MindListing::decode(&serde_json::to_vec(
        &one_row(store, "market", "market_mind_original_listing").await?,
    )?)?;
    Ok(listing.publication())
}

async fn purchase(store: &PurchaseFaultStore, operation: CommitToken) -> TestResult<Value> {
    let matches = rows(store, "market", "market_mind_purchase")
        .await?
        .into_iter()
        .filter(|row| row.get("operation") == Some(&json!(operation)))
        .collect::<Vec<_>>();
    let [row]: [Value; 1] = matches
        .try_into()
        .map_err(|_| "one exact paired purchase")?;
    Ok(row)
}
fn phase(row: &Value) -> TestResult<&str> {
    field(field(row, "phase")?, "kind")?
        .as_str()
        .ok_or_else(|| "purchase phase kind".into())
}
async fn operation_quote(store: &PurchaseFaultStore, quote: CommitToken) -> TestResult<Value> {
    let matches = rows(store, "market", "market_mind_purchase")
        .await?
        .into_iter()
        .filter(|row| row.get("quote").and_then(|q| q.get("publication")) == Some(&json!(quote)))
        .collect::<Vec<_>>();
    let [row]: [Value; 1] = matches
        .try_into()
        .map_err(|_| "one exact quote consumption")?;
    Ok(row)
}
async fn wait_selected(store: &PurchaseFaultStore, owned: bool) -> TestResult<Exact> {
    let end = Instant::now() + STEP;
    loop {
        if let Some(exact) = store.selected()?
            && (!owned
                || matches!(
                    store.authoritative_status(exact.token, exact.hash).await?,
                    BundleStatus::PendingExact
                ))
        {
            return Ok(exact);
        }
        if Instant::now() >= end {
            return Err("actual paid service did not submit Grant".into());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
async fn roots(store: &PurchaseFaultStore) -> TestResult<Vec<(String, Value)>> {
    let mut rows = store
        .module_json_list("roll")
        .await?
        .into_iter()
        .filter(|(key, _)| key.starts_with("reference:"))
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(rows)
}
async fn assert_pending(
    store: &PurchaseFaultStore,
    exact: &Exact,
    before: &Value,
    wallet_before: &Account,
    retained_roots: &[(String, Value)],
    owned: bool,
) -> TestResult {
    assert!(
        purchase(store, exact.operation).await? == *before,
        "pending purchase bytes changed"
    );
    assert!(
        wallet(store, wallet_before.id).await? == *wallet_before,
        "pending economic balance/hold changed"
    );
    assert!(
        roots(store).await? == retained_roots,
        "pending retained source roots changed"
    );
    assert_eq!(phase(before)?, "funded");
    let status = store.authoritative_status(exact.token, exact.hash).await?;
    assert!(
        if owned {
            matches!(status, BundleStatus::PendingExact)
        } else {
            matches!(status, BundleStatus::Absent)
        },
        "actual exact ownership status differs"
    );
    let events = store.events()?;
    let selected_event = events
        .iter()
        .find(|event| event.exact == *exact)
        .ok_or("actual selected writer observation")?;
    assert_eq!(selected_event.reserved, owned);
    assert!(!selected_event.finalized);
    if owned {
        assert_eq!(
            events
                .iter()
                .filter(|e| e.exact.operation == exact.operation
                    && e.exact.action.key().id.as_str() == "settle_original_mind")
                .count(),
            1,
            "owned Unknown must retain its exact original Grant attempt"
        );
    }
    assert_eq!(
        events
            .iter()
            .filter(|e| e.exact.operation == exact.operation
                && e.exact.action.key().id.as_str() == "purchase_original_mind"
                && e.finalized)
            .count(),
        1
    );
    assert!(!events.iter().any(|e| e.exact.operation == exact.operation
        && e.exact.action.key().id.as_str() == "refund_original_mind"));
    Ok(())
}
async fn wait_terminal(
    store: &PurchaseFaultStore,
    operation: CommitToken,
    granted: bool,
    payer_before_fund: &Account,
    paid_before: usize,
    retained_roots: &[(String, Value)],
) -> TestResult {
    let end = Instant::now() + STEP;
    loop {
        let row = purchase(store, operation).await?;
        match phase(&row)? {
            "funded" => {}
            actual if actual == if granted { "granted" } else { "refunded" } => break,
            _ => return Err("unexpected terminal purchase phase".into()),
        }
        if Instant::now() >= end {
            return Err("actual paid driver terminal hang guard".into());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let after = wallet(store, payer_before_fund.id).await?;
    assert_eq!(after.held_amount, Amount::ZERO);
    assert_eq!(
        after.posted_balance.minor(),
        payer_before_fund.posted_balance.minor() - if granted { PRICE } else { 0 }
    );
    let receipts = rows(store, "market", "market_mind_purchase_receipt").await?;
    assert_eq!(
        receipts
            .iter()
            .filter(|r| r.get("operation") == Some(&json!(operation)))
            .count(),
        1
    );
    assert_eq!(
        rows(store, "roll", "roll_mind_personal_paid_licence")
            .await?
            .len(),
        paid_before + usize::from(granted)
    );
    assert_eq!(
        rows(store, "roll", "roll_mind_paid_term").await?.len(),
        paid_before + usize::from(granted)
    );
    assert!(
        roots(store).await? == retained_roots,
        "terminal preserves exact pre-funded roots"
    );
    let events = store.events()?;
    let for_op = events
        .iter()
        .filter(|e| e.exact.operation == operation && e.finalized)
        .collect::<Vec<_>>();
    assert_eq!(
        for_op
            .iter()
            .filter(|e| e.exact.action.key().id.as_str() == "purchase_original_mind")
            .count(),
        1
    );
    assert_eq!(
        for_op
            .iter()
            .filter(|e| e.exact.action.key().id.as_str()
                == if granted {
                    "settle_original_mind"
                } else {
                    "refund_original_mind"
                })
            .count(),
        1
    );
    assert_eq!(
        for_op.len(),
        2,
        "one funding and exactly one actual terminal writer receipt"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "required expensive real server Registry, writer Unknown, expiry and cold recovery gate"]
async fn genuine_registry_purchase_writer_unknown_expiry_and_recovery() -> TestResult {
    timeout(Duration::from_secs(18_000), story()).await?
}

/// A generic committed receipt is real durable success even when the provider
/// deliberately has no optional result projector. Exercise the exact public
/// server path before paying the expensive certificate gate's runtime cost.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn public_commerce_setup_accepts_generic_committed_receipts() -> TestResult {
    timeout(STEP, async {
        let config = config();
        let store = Arc::new(PurchaseFaultStore::new());
        auth_account(&store, "generic-receipt-owner", Some(GOD)).await?;
        let mut server = Running::boot(&config, store.clone(), None).await?;
        server.connect(GOD).await?;
        let sink = open_wallet(&server, &store, PrincipalId::System(issuer()?)).await?;
        server.stop().await?;
        let initialize = composition(&config, CommerceMode::Initialize, sink)?;
        server = Running::boot(&config, store.clone(), Some(&initialize)).await?;
        server.connect(GOD).await?;
        let initialized = server
            .offer(GOD, "roll-initialize-commerce", json!({}))
            .await?;
        check!(
            initialized.result().is_none(),
            "actual provider uses generic receipt"
        );
        initialized_rows(&store, &initialized).await?;
        server.stop().await?;
        // Real cold owner audits accept the acknowledged markers. No test
        // state setter, direct action prepare, or artificial ACK participates.
        let active = composition(&config, CommerceMode::Active, sink)?;
        server = Running::boot(&config, store.clone(), Some(&active)).await?;
        server.connect(GOD).await?;
        let reserved = server
            .offer(GOD, "roll-reserve-mind-charter", json!({}))
            .await?;
        check!(
            reserved.result().is_none(),
            "actual shell provider uses generic receipt"
        );
        let shell = one_row(&store, "roll", "initial_filing_shells").await?;
        let commit = CommitToken::from_hex(reserved.commit())?;
        check!(
            field(&shell, "receipt")? == &json!(commit),
            "actual shell receipt commit"
        );
        check!(rows(&store, "roll", "rating_jobs").await?.is_empty());
        check!(
            rows(&store, "market", "market_mind_purchase")
                .await?
                .is_empty()
        );
        server.stop().await?;
        Ok(())
    })
    .await?
}

async fn initialized_rows(
    store: &PurchaseFaultStore,
    receipt: &mush_proto::ProjectedActionReceipt,
) -> TestResult {
    let commit = CommitToken::from_hex(receipt.commit())?;
    for family in ["roll_mind_commerce", "initial_filing_shell_deployment"] {
        let row = one_row(store, "roll", family).await?;
        assert!(
            field(&row, "initialized_by")? == &json!(commit),
            "actual setup marker must share the finalized receipt commit"
        );
    }
    Ok(())
}

async fn story() -> TestResult {
    let config = config();
    let store = Arc::new(PurchaseFaultStore::new());
    auth_account(&store, "server-seller", Some(GOD)).await?;
    let buyer_account = auth_account(&store, "server-buyer", None).await?;
    let mut server = Running::boot(&config, store.clone(), None).await?;
    server.connect(GOD).await?;
    let buyer = timeout(
        STEP,
        server
            .handle()?
            .create_player("ServerBuyer", Some(buyer_account)),
    )
    .await??;
    store
        .link_character("server-buyer", buyer, "ServerBuyer")
        .await?;
    server.connect(buyer).await?;
    let sink = open_wallet(&server, &store, PrincipalId::System(issuer()?)).await?;
    let seller_wallet = open_wallet(&server, &store, PrincipalId::Object(GOD)).await?;
    let buyer_wallet = open_wallet(&server, &store, PrincipalId::Object(buyer)).await?;
    for wallet_id in [seller_wallet, buyer_wallet] {
        server
            .command(GOD, format!("@econ/mint {}={} 100000", sink.0, wallet_id.0))
            .await?;
        assert_eq!(
            wallet(&store, wallet_id).await?.posted_balance,
            Amount::from_minor(100000)
        );
    }
    for actor in [GOD, buyer] {
        server
            .command(
                GOD,
                format!("@teleport #{}=#{}", actor.0, server.exchange.0),
            )
            .await?;
    }
    server.stop().await?;
    let initialize = composition(&config, CommerceMode::Initialize, sink)?;
    server = Running::boot(&config, store.clone(), Some(&initialize)).await?;
    server.connect(GOD).await?;
    let initialized = server
        .offer(GOD, "roll-initialize-commerce", json!({}))
        .await?;
    initialized_rows(&store, &initialized).await?;
    server.stop().await?;
    let active = composition(&config, CommerceMode::Active, sink)?;
    server = Running::boot(&config, store.clone(), Some(&active)).await?;
    server.connect(GOD).await?;
    server.connect(buyer).await?;
    let revision = saved_revision(&server, &store).await?;
    let rating = genuine_rating(&server, &store, &revision).await?;
    let listing = file_and_list(&server, &store, seller_wallet, &revision, &rating).await?;
    // No snapshot-copy shortcut: the same store and actual certificate survive
    // each acknowledged operation and actual cold boot.
    for case in 0..3 {
        let old_quotes = rows(&store, "market", "market_mind_original_quote").await?;
        server
            .offer(
                buyer,
                "market-original-mind-quote",
                json!({"listing":listing}),
            )
            .await?;
        let fresh = rows(&store, "market", "market_mind_original_quote")
            .await?
            .into_iter()
            .filter(|row| {
                !old_quotes
                    .iter()
                    .any(|old| old.get("publication") == row.get("publication"))
            })
            .collect::<Vec<_>>();
        let [quote_row]: [Value; 1] = fresh.try_into().map_err(|_| "one actual new quote")?;
        let quote = MindQuote::decode(&serde_json::to_vec(&quote_row)?)?;
        let expiry = field(&quote_row, "expires_at_utc")?
            .as_i64()
            .ok_or("actual quote expiry")?;
        let payer_before = wallet(&store, buyer_wallet).await?;
        let paid_before = rows(&store, "roll", "roll_mind_personal_paid_licence")
            .await?
            .len();
        store.arm(
            quote.publication(),
            if case == 2 {
                Fault::AbsentBeforeIntent
            } else {
                Fault::ReconcileUnknown
            },
        )?;
        server
            .offer(
                buyer,
                "market-original-mind-fund",
                json!({"quote":quote.publication()}),
            )
            .await?;
        let funded = operation_quote(&store, quote.publication()).await?;
        let operation: CommitToken = serde_json::from_value(field(&funded, "operation")?.clone())?;
        let exact = wait_selected(&store, case != 2).await?;
        assert_eq!(exact.operation, operation);
        assert_eq!(exact.quote, quote.publication());
        let held = wallet(&store, buyer_wallet).await?;
        assert_eq!(held.posted_balance, payer_before.posted_balance);
        assert_eq!(held.held_amount, Amount::from_minor(PRICE));
        let retained_roots = roots(&store).await?;
        if retained_roots.is_empty() {
            return Err("actual funded source roots missing".into());
        }
        // This is a wall-clock crossing. CommitPending deliberately keeps the
        // actual logical World/IC clock at the owned preparation boundary.
        wall_after(expiry + 1).await?;
        assert_pending(&store, &exact, &funded, &held, &retained_roots, case != 2).await?;
        eprintln!(
            "M7_SERVER_PURCHASE_PENDING case={case} operation={operation} token={} hash={}",
            exact.token, exact.hash
        );
        if case == 1 {
            server.crash().await?;
            assert_pending(&store, &exact, &funded, &held, &retained_roots, true).await?;
            store.heal();
            // Actual K3 reconciliation runs before typed hydration/final audit.
            // A restart composes its owners afresh, as a new process does: the
            // writers are installed once per composition.
            let active = composition(&config, CommerceMode::Active, sink)?;
            server = Running::boot(&config, store.clone(), Some(&active)).await?;
            server.connect(GOD).await?;
            server.connect(buyer).await?;
        } else {
            store.heal();
        }
        wait_terminal(
            &store,
            operation,
            case != 2,
            &payer_before,
            paid_before,
            &retained_roots,
        )
        .await?;
        server.stop().await?;
        let active = composition(&config, CommerceMode::Active, sink)?;
        server = Running::boot(&config, store.clone(), Some(&active)).await?;
        server.connect(GOD).await?;
        server.connect(buyer).await?;
        wait_terminal(
            &store,
            operation,
            case != 2,
            &payer_before,
            paid_before,
            &retained_roots,
        )
        .await?;
        eprintln!(
            "M7_SERVER_PURCHASE_TERMINAL case={case} operation={operation} granted={}",
            case != 2
        );
    }
    assert_eq!(
        rows(&store, "market", "market_mind_purchase").await?.len(),
        3
    );
    assert_eq!(
        rows(&store, "market", "market_mind_purchase_receipt")
            .await?
            .len(),
        3
    );
    assert_eq!(
        rows(&store, "roll", "roll_mind_personal_paid_licence")
            .await?
            .len(),
        2
    );
    server.stop().await?;
    Ok(())
}
