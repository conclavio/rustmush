//! 17g C2 (plan §5): the player market's command story, one genuine Registry
//! rating through the server's paced maintenance ticks. It is `#[ignore]`d
//! and named verbatim in `.github/workflows/ci.yml`'s
//! `mind-commerce-acceptance` matrix ("player market commands"), so the
//! `ci_gated_test_inventory` contract covers it; no gate list runs it.
//!
//! Every step is a line command by a connected character; setup is
//! production staff commands. After each acknowledged step it checks the
//! store rows and balances that step must move, and nothing else. C2a writes
//! part 1 (the author files by command); C2c continues with part 2 (listing,
//! buying, holdings) and C2d with part 3 (resale, renewal, charter, Registry
//! operators), each where the `// C2c:` and `// C2d:` markers stand.
use super::paid_purchase::runtime::{STEP, rows};
use super::player_market::{CREDIT, acknowledged};
use super::player_market_commands::{
    TestResult, World, active, open_shells, same_refusal, saved, until_own, world_with_terms,
};
use mush_core::ObjId;
use mush_economy::{Account, Amount};
use mush_org::PrincipalId;
use mush_roll::commerce::rating::{RatingJob, RatingJobState};
use mush_store::ObjectStore;
use serde_json::Value;
use std::time::Duration;
use tokio::time::{Instant, timeout};

/// The genuine rating's finite hang guard (the server story's, `paid_purchase`).
const REGISTRY_GUARD: Duration = Duration::from_secs(14_400);
/// The launch filing fee: 100cr.
const FEE_MINOR: i64 = 10_000;
const DRIVER: &str = "That did not go through. Nothing changed.";
/// The engine's receipt acknowledgment of every command-submitted action
/// (`Engine`, E7c2 fold R11): best-effort, between the request line and the
/// driver's line, never the confirmation.
const QUEUED: &str = "Queued.";

/// The one Active, unscoped credit account `owner` holds.
async fn wallet(w: &World, owner: &PrincipalId) -> TestResult<Account> {
    let mut found = Vec::new();
    for (key, row) in w.launch.store.module_json_list("economy").await? {
        if key.starts_with("account:") {
            let account: Account = serde_json::from_value(row)?;
            if &account.owner == owner
                && account.scope.is_none()
                && account.instrument.as_str() == CREDIT
            {
                found.push(account);
            }
        }
    }
    let [account]: [Account; 1] = found.try_into().map_err(|_| "exactly one credit account")?;
    Ok(account)
}
/// The author's posted balance and the Registry's.
async fn balances(w: &World, author: ObjId) -> TestResult<(Amount, Amount)> {
    let registry = w.launch.deployment.registry.account;
    let registry = w
        .launch
        .store
        .module_json_get("economy", &format!("account:{}", registry.0))
        .await?
        .ok_or("the Registry account")?;
    let registry: Account = serde_json::from_value(registry)?;
    Ok((
        wallet(w, &PrincipalId::Object(author))
            .await?
            .posted_balance,
        registry.posted_balance,
    ))
}

/// The one filing row's `field` (a filed or licence number) as typed.
fn number(row: &Value, field: &str) -> TestResult<String> {
    row.get(field)
        .and_then(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .or_else(|| value.as_u64().map(|n| n.to_string()))
        })
        .ok_or_else(|| format!("the filing's {field}").into())
}

/// Waits for the genuine rating of `revision` as `genuine_rating` does
/// (reviewer T6): Roll's one typed job, `Queued` or `Running` waiting,
/// `Completed` of exactly this revision ending the wait, any other state an
/// immediate failure. Meanwhile it takes the author's own lines, and it ends
/// only once the author has also read `Registry work finished.` (reviewer
/// F5): the Registry service's own settle, after which `@mind/file` finds
/// the finished run (`registry_service::completed_rating`), not just Roll's
/// completed row.
async fn rated(w: &World, author: ObjId, revision: &mush_roll::Revision) -> TestResult {
    let identity = mush_roll::commerce::RevisionIdentity::capture(revision)?;
    let end = Instant::now() + REGISTRY_GUARD;
    let mut settled = false;
    let mut completed: Option<Instant> = None;
    loop {
        settled |= w
            .server
            .take_seen(author)?
            .own
            .iter()
            .any(|line| line == "Registry work finished.");
        if completed.is_none() {
            let jobs = rows(&w.launch.store, "roll", "rating_jobs").await?;
            if jobs.len() > 1 {
                return Err(format!("one Registry job expected: {jobs:?}").into());
            }
            if let [row] = jobs.as_slice() {
                let job: RatingJob = serde_json::from_value(row.clone())?;
                match job.state() {
                    RatingJobState::Completed { .. } => {
                        check_eq!(job.subject().revision(), identity, "the author's revision");
                        completed = Some(Instant::now());
                    }
                    RatingJobState::Queued {} | RatingJobState::Running { .. } => {}
                    other => return Err(format!("the rating ended {other:?}").into()),
                }
            }
        }
        match completed {
            Some(_) if settled => return Ok(()),
            Some(at) => check!(
                at.elapsed() < STEP,
                "Registry work finished. after the completion"
            ),
            None => check!(Instant::now() < end, "the genuine rating completes"),
        }
        let pause = if completed.is_some() { 50 } else { 2_000 };
        tokio::time::sleep(Duration::from_millis(pause)).await;
    }
}

/// The rows a restart must not move: the filing, the owner licence and the
/// author's open shells.
async fn durable(w: &World, author: ObjId) -> TestResult<[Vec<Value>; 3]> {
    Ok([
        rows(&w.launch.store, "roll", "roll_mind_initial_filing").await?,
        rows(&w.launch.store, "roll", "roll_mind_owner_licence").await?,
        open_shells(&w.launch, author).await?,
    ])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "17g C2 command story: one genuine Registry rating; run by the mind-commerce-acceptance dispatch"]
async fn player_market_commands_story_two_accounts_alt_stale_tiny_and_restart() -> TestResult {
    let market = tacsim::launch::MarketTerms {
        quote_seconds: 30,
        ..tacsim::launch::MarketTerms::PROVISIONAL
    };
    let mut w = world_with_terms("mush-c2-story", market).await?;
    let result = timeout(REGISTRY_GUARD, async {
        let p = w.launch.principals;
        let author = p.author;
        // Part 1 (17g C2a): the author files by command.
        // Fund the author's filing fee from the launch faucet, a staff mint.
        let faucet = wallet(
            &w,
            &PrincipalId::System(
                mush_org::SystemPrincipalId::new("launch.faucet").ok_or("faucet")?,
            ),
        )
        .await?;
        let own = wallet(&w, &PrincipalId::Object(author)).await?;
        acknowledged(
            &w.server,
            format!("@econ/mint {}={} {}", faucet.id.0, own.id.0, 2 * FEE_MINOR),
            "Minted",
        )
        .await?;
        let revision = saved(&w, author, "Sentinel").await?;
        // The buyer's own saved mind, so its probes below hold a revision.
        saved(&w, p.buyer, "Probe").await?;
        let not_rated = vec![
            "sentinel@1 has no finished Registry rating. Rate it with @mind/rate sentinel@1."
                .to_owned(),
        ];
        let answered = |l: &str| l == "Done." || l == DRIVER || l.ends_with("(already done)");
        // The step order departs from plan §5's (NOT_RATED, rate, reserve,
        // file): with no shell open, `@mind/file` reserves before it reads
        // any rating (C2 plan §6's own algorithm), so the first `@mind/file`
        // reserves and the rating refusal is read once a shell is open.
        let reserving = w
            .server
            .command(author, "@mind/file Sentinel@1".into())
            .await?;
        let reserving = until_own(&w.server, author, reserving, answered).await?;
        check_eq!(
            reserving,
            vec![
                "Reserving a charter for this filing first. Once that is confirmed, type @mind/file Sentinel@1 again.".to_owned(),
                QUEUED.to_owned(),
                "Done.".to_owned()
            ],
            "the request line, then the driver's commit-bound confirmation"
        );
        let end = Instant::now() + STEP;
        while open_shells(&w.launch, author).await?.len() != 1 {
            check!(Instant::now() < end, "the author's one open shell");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        check_eq!(
            w.server
                .command(author, "@mind/file Sentinel@1".into())
                .await?,
            not_rated
        );
        // The genuine rating, waited for as `genuine_rating` waits.
        check_eq!(
            w.server
                .command(author, "@mind/rate Sentinel@1".into())
                .await?,
            vec!["Registry preparation queued.".to_owned()]
        );
        rated(&w, author, &revision).await?;
        // The shell is open from the first `@mind/file`; this one files.
        let before = balances(&w, author).await?;
        let filing = w
            .server
            .command(author, "@mind/file Sentinel@1".into())
            .await?;
        let filing = until_own(&w.server, author, filing, answered).await?;
        check_eq!(
            filing,
            vec![
                "Filing sentinel@1 as sentinel.".to_owned(),
                QUEUED.to_owned(),
                "Done.".to_owned()
            ],
            "the request line, then the driver's commit-bound confirmation"
        );
        let end = Instant::now() + STEP;
        let filed = loop {
            let operations = rows(&w.launch.store, "roll", "roll_mind_initial_filing").await?;
            if let [operation] = operations.as_slice() {
                break operation.clone();
            }
            check!(Instant::now() < end, "one initial filing row");
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        check_eq!(
            rows(&w.launch.store, "roll", "roll_mind_owner_licence")
                .await?
                .len(),
            1,
            "one owner licence"
        );
        check_eq!(open_shells(&w.launch, author).await?.len(), 0);
        let after = balances(&w, author).await?;
        check_eq!(
            after,
            (
                Amount::from_minor(before.0.minor() - FEE_MINOR),
                Amount::from_minor(before.1.minor() + FEE_MINOR)
            ),
            "the author pays the 100cr fee to the Registry"
        );
        let filed_number = number(&filed, "filed")?;
        let licence = number(&filed, "licence")?;
        // Not due: the attestation provider refuses (the driver's line). The
        // filing's "Done." was commit-bound, so no request of the author's is
        // still pending.
        let attest = w
            .server
            .command(author, format!("@mind/attest {filed_number}"))
            .await?;
        let attest = until_own(&w.server, author, attest, |l| l == DRIVER).await?;
        check_eq!(
            attest,
            vec![
                format!("Renewing your owner's licence for filed mind {filed_number}."),
                QUEUED.to_owned(),
                DRIVER.to_owned()
            ]
        );
        // A foreign target reads like a missing one (plan §2.8; reviewer
        // F3/T2), for every author command that names one: the author's real
        // filed number and real owner licence, named by the buyer (another
        // login) and by the alt (the author's login, another character).
        const MISSING: &str = "987654";
        for (actor, name) in [(p.buyer, "buyer"), (w.alt, "alt")] {
            for (template, typed, driven) in [
                ("@mind/update {n} Probe@1", &filed_number, false),
                ("@mind/withdraw {n}", &filed_number, false),
                ("@mind/attest {n}", &filed_number, false),
                ("@mind/fork {n}=Probe Copy", &licence, true),
            ] {
                let mut replies = Vec::new();
                for typed in [typed.as_str(), MISSING] {
                    let lines = w
                        .server
                        .command(actor, template.replace("{n}", typed))
                        .await?;
                    let lines = if driven {
                        until_own(&w.server, actor, lines, |l| l == DRIVER).await?
                    } else {
                        lines
                    };
                    replies.push(lines);
                }
                let [foreign, missing] =
                    <[Vec<String>; 2]>::try_from(replies).map_err(|_| "two replies")?;
                check!(
                    same_refusal(&foreign, typed, &missing, MISSING),
                    "the {name}: {template}: {foreign:?} vs {missing:?}"
                );
            }
        }
        // Restart: the rows and balances are unchanged.
        let rows_before = durable(&w, author).await?;
        w.server.stop().await?;
        w.server = active(&w.launch, w.alt, w.bystander).await?;
        check_eq!(
            durable(&w, author).await?,
            rows_before,
            "the filing, the owner licence, no open shell"
        );
        check_eq!(balances(&w, author).await?, after);
        // C2c: part 2 continues here (publish, +minds, the alt pair, stale
        // terms, the purchase, holdings, the open listing and the paid fork,
        // and a restart).
        // C2d: part 3 continues after part 2 (resale offers, renewal, the
        // withdrawal, the charter transfer, revocation and recall, a restart).
        Ok::<_, Box<dyn std::error::Error>>(())
    })
    .await;
    w.server.stop().await?;
    let _ = std::fs::remove_dir_all(&w.data_dir);
    result?
}
