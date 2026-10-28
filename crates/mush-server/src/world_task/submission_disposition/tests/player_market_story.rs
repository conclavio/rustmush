//! 17g C2 (plan §5): the player market's command story, one genuine Registry
//! rating through the server's paced maintenance ticks. It is `#[ignore]`d
//! and named verbatim in `.github/workflows/ci.yml`'s
//! `mind-commerce-acceptance` matrix ("player market commands"), so the
//! `ci_gated_test_inventory` contract covers it; no gate list runs it.
//!
//! Every step is a line command by a connected character; setup is
//! production staff commands. After each acknowledged step it checks the
//! store rows and balances that step must move, and nothing else. C2a writes
//! part 1 (the author files by command); part 2 (listing, buying, holdings)
//! and part 3 (resale, renewal, charter, Registry operators) follow at the
//! `// C2c:` and `// C2d:` markers, written by 17g C2h once the indexes (C2f),
//! the buy steps and named offers (C2g) and the renewal front (C2h) were
//! live (`part_two`, `part_three`).
use super::paid_purchase::runtime::{STEP, rows};
use super::paid_purchase::{auth_account, open_wallet};
use super::player_market::{CREDIT, acknowledged, character};
use super::player_market_commands::{
    TestResult, World, active, assert_private, await_own, known_accounts, open_shells,
    same_refusal, saved, until_own, world_with_terms,
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
/// 17g C2h: the whole story's guard, the rating's and an hour and a half for
/// parts 2 and 3 (part 3 runs the IC clock fast for about 36 real minutes
/// and waits about 10 more at the launch's rate), inside the dispatch job's
/// 360 minutes.
const STORY_GUARD: Duration = Duration::from_secs(14_400 + 5_400);
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

// ---- 17g C2h: parts 2 and 3 (the C2e plan §5.2, the C2h brief item 11) ----
//
// Both parts are written to the C2c and C2d briefs as the C2e plan and the
// lead's rulings after C2g amend them, and to the command words as built
// (17g C2c, C2d, C2g and C2h). Every command-submitted reply is its request
// line (and a seller's tell), the engine's best-effort `Queued.`, then
// `Done.` (no commerce provider projects a receipt summary, so the driver's
// confirmation is `Done.`) or the driver's line (the C2a rule).
// `+minds/renew` prints nothing itself: the renewal service's `front`
// continuation prints its words after the step, on the world loop's next
// timer wake, so its replies are awaited (`later`).

/// The third account's character of part 3.
const CY: &str = "Cy";
/// What a step's last line may be.
fn answered(line: &str) -> bool {
    line == "Done." || line == DRIVER || line.ends_with("(already done)")
}
/// One command-submitted step: its own reply until `Done.` or the driver's.
async fn submitted(w: &World, actor: ObjId, line: String) -> TestResult<Vec<String>> {
    let lines = w.server.command(actor, line).await?;
    until_own(&w.server, actor, lines, answered).await
}
/// A command whose reply arrives later (the renewal service's front): its
/// own lines until one satisfies `done`.
async fn later(
    w: &World,
    actor: ObjId,
    line: String,
    done: impl Fn(&str) -> bool,
) -> TestResult<Vec<String>> {
    let lines = w.server.command(actor, line).await?;
    until_own(&w.server, actor, lines, done).await
}
/// Waits (within `STEP`) until `actor` has had no line for a whole second,
/// dropping what arrived (17g C2h, review F3). The renewal service drafts,
/// grants and closes as the holder (its own continuation, D-UM-39's
/// automatic halves: it never pays), so those submissions' `Queued.` and
/// confirmations reach the holder on the service's schedule; a step pinned
/// exactly after one starts from quiet.
async fn settle(w: &World, actor: ObjId) -> TestResult {
    let end = Instant::now() + STEP;
    let mut quiet = Instant::now();
    loop {
        if !w.server.take_seen(actor)?.own.is_empty() {
            quiet = Instant::now();
        }
        if quiet.elapsed() >= Duration::from_secs(1) {
            return Ok(());
        }
        check!(Instant::now() < end, "#{} falls quiet", actor.0);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
/// `later` under the observer rule (17g C2h, review T5): the renewal front's
/// words are printed by a market continuation, a new output path, so every
/// line of the step must reach `actor` alone (the author, the buyer, the alt
/// and the bystander hear nothing of it) and carry no account id or
/// digest-shaped run (`assert_private`).
async fn private_later(
    w: &World,
    actor: ObjId,
    line: String,
    done: impl Fn(&str) -> bool,
) -> TestResult<Vec<String>> {
    let p = w.launch.principals;
    let mut seen = w.server.command_seen(actor, line.clone()).await?;
    if !seen.own.iter().any(|l| done(l)) {
        seen.extend(await_own(&w.server, actor, &done).await?);
    }
    for observer in [p.author, p.buyer, w.alt, w.bystander] {
        if observer != actor {
            let reached = seen.reaching(observer);
            check!(
                reached.is_empty(),
                "{line} reached #{}: {reached:?}",
                observer.0
            );
        }
    }
    assert_private(&seen.own, &known_accounts(w).await?)?;
    Ok(seen.own)
}
/// A credit rendering (`400cr`, `12.50cr`) in minor credits.
fn minor_of(price: &str) -> TestResult<i64> {
    let amount = price
        .strip_suffix("cr")
        .ok_or_else(|| format!("a credit amount: {price}"))?;
    let (whole, cents) = amount.split_once('.').unwrap_or((amount, "00"));
    check_eq!(cents.len(), 2, "{price}");
    Ok(whole
        .parse::<i64>()?
        .checked_mul(100)
        .and_then(|w| w.checked_add(cents.parse::<i64>().ok()?))
        .ok_or_else(|| format!("a credit amount: {price}"))?)
}
/// The frozen price, in minor credits, of the market's newest renewal
/// operation on `licence` (the row the front prints `RENEW_OFFER` from).
async fn frozen_price(w: &World, licence: &str) -> TestResult<i64> {
    let mut newest: Option<(i64, i64)> = None;
    for row in rows(&w.launch.store, "market", "market_mind_renewal_operation").await? {
        if row.pointer("/binding/licence").and_then(Value::as_str) == Some(licence) {
            let at = row
                .pointer("/created_at_utc")
                .and_then(Value::as_i64)
                .ok_or("a renewal time")?;
            let price = row
                .pointer("/offer/terms/price")
                .and_then(Value::as_i64)
                .ok_or("a frozen price")?;
            if newest.is_none_or(|(t, _)| at >= t) {
                newest = Some((at, price));
            }
        }
    }
    Ok(newest.ok_or("a market renewal operation")?.1)
}
/// The posted balance of `owner`'s one credit account, in minor credits.
async fn minor(w: &World, owner: ObjId) -> TestResult<i64> {
    Ok(wallet(w, &PrincipalId::Object(owner))
        .await?
        .posted_balance
        .minor())
}
/// The Registry's posted balance, in minor credits.
async fn registry_minor(w: &World, author: ObjId) -> TestResult<i64> {
    Ok(balances(w, author).await?.1.minor())
}
/// The sum of every credit account's posted balance (escrow included): a
/// money step moves money, it never makes or destroys it.
async fn total(w: &World) -> TestResult<i64> {
    let mut sum = 0_i64;
    for (key, row) in w.launch.store.module_json_list("economy").await? {
        if key.starts_with("account:") {
            let account: Account = serde_json::from_value(row)?;
            if account.instrument.as_str() == CREDIT {
                sum = sum
                    .checked_add(account.posted_balance.minor())
                    .ok_or("balance sum")?;
            }
        }
    }
    Ok(sum)
}
/// The licence numbers of the paid licences `holder` holds now, ascending.
async fn paid_licences(w: &World, holder: ObjId) -> TestResult<Vec<u64>> {
    let holder = serde_json::to_value(holder)?;
    let mut held = Vec::new();
    for row in rows(&w.launch.store, "roll", "roll_mind_personal_paid_licence").await? {
        if row.get("holder") == Some(&holder) {
            held.push(number(&row, "id")?.parse::<u64>()?);
        }
    }
    held.sort_unstable();
    Ok(held)
}
/// Waits (within `STEP`) until `holder` holds exactly `count` paid licences.
async fn holds(w: &World, holder: ObjId, count: usize) -> TestResult<Vec<u64>> {
    let end = Instant::now() + STEP;
    loop {
        let held = paid_licences(w, holder).await?;
        if held.len() == count {
            return Ok(held);
        }
        check!(
            Instant::now() < end,
            "#{} holds {count}: {held:?}",
            holder.0
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
/// Roll's renewal operation rows on `licence`, as `(state kind, created
/// world-IC generation)`, oldest first by creation.
async fn renewals(w: &World, licence: &str) -> TestResult<Vec<(String, u64)>> {
    let mut found: Vec<(i64, String, u64)> = Vec::new();
    for row in rows(&w.launch.store, "roll", "roll_mind_renewal_operation").await? {
        if row.pointer("/binding/licence").and_then(Value::as_str) == Some(licence) {
            let kind = row
                .pointer("/state/kind")
                .and_then(Value::as_str)
                .ok_or("a renewal state")?
                .to_owned();
            let generation = row
                .pointer("/created_at_ic/clock/generation")
                .and_then(Value::as_u64)
                .ok_or("a renewal clock")?;
            let at = row
                .pointer("/created_at_utc")
                .and_then(Value::as_i64)
                .ok_or("a renewal time")?;
            found.push((at, kind, generation));
        }
    }
    found.sort();
    Ok(found.into_iter().map(|(_, kind, g)| (kind, g)).collect())
}
/// Waits (within `STEP`) until the newest renewal operation on `licence` is
/// in state `kind`.
async fn renewal_reaches(w: &World, licence: &str, kind: &str) -> TestResult {
    let end = Instant::now() + STEP;
    loop {
        let all = renewals(w, licence).await?;
        if all.last().is_some_and(|(k, _)| k == kind) {
            return Ok(());
        }
        check!(
            Instant::now() < end,
            "licence {licence} renewal {kind}: {all:?}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}
/// Waits for the newest renewal operation on `licence` to be drafted, and
/// checks that it was drafted under the current world-IC generation (an
/// offer drafted under an earlier one is the service's to close).
async fn drafted_now(w: &World, licence: &str) -> TestResult {
    renewal_reaches(w, licence, "drafted").await?;
    let (_, generation) = ic_now(w).await?;
    check_eq!(
        renewals(w, licence).await?.last().map(|(_, g)| *g),
        Some(generation),
        "drafted under the current generation"
    );
    Ok(())
}
/// The current world-IC second and generation, from the persisted timeline
/// (`rustmush_kernel_time`'s newest epoch) and the wall clock, as the world
/// reads `world_ic_now`.
async fn ic_now(w: &World) -> TestResult<(i64, u64)> {
    let mut newest: Option<(u64, i64, i64, i64)> = None;
    for (_, row) in w
        .launch
        .store
        .module_json_list("rustmush_kernel_time")
        .await?
    {
        let (Some(generation), Some(real), Some(ic), Some(rate)) = (
            row.get("generation").and_then(Value::as_u64),
            row.get("real_anchor").and_then(Value::as_i64),
            row.get("ic_anchor").and_then(Value::as_i64),
            row.get("rate").and_then(Value::as_i64),
        ) else {
            continue;
        };
        if newest.is_none_or(|(g, ..)| generation > g) {
            newest = Some((generation, real, ic, rate));
        }
    }
    let (generation, real, ic, rate) = newest.ok_or("a world timeline epoch")?;
    let unix: i64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs()
        .try_into()?;
    Ok((ic + (unix - real) * rate, generation))
}
/// Staff open a world-IC rate generation (`@tacsim/time rate=<n>`, God);
/// the acknowledgement is held behind the durable write, so it is awaited.
async fn rate(w: &World, rate: u32) -> TestResult {
    let opens = format!("opens at {rate} IC seconds per real second");
    let lines = later(
        w,
        mush_core::GOD,
        format!("@tacsim/time rate={rate}"),
        |l| l.contains(&opens),
    )
    .await?;
    check!(lines.iter().any(|l| l.contains(&opens)), "{lines:?}");
    Ok(())
}
/// `licence`'s current term's end, in world-IC seconds.
async fn term_end(w: &World, licence: &str, generation: &str) -> TestResult<i64> {
    for row in rows(&w.launch.store, "roll", "roll_mind_paid_term").await? {
        if row.get("licence").and_then(Value::as_str) == Some(licence)
            && row.get("generation").and_then(Value::as_str) == Some(generation)
        {
            let term: mush_roll::commerce::paid::PaidTermRecord = serde_json::from_value(row)?;
            return Ok(term.window().term_end().get());
        }
    }
    Err(format!("licence {licence}'s term {generation}").into())
}
/// The phase words at the end of a holdings row that starts with `prefix`
/// (the phase moves with the IC clock, so a row is pinned up to it).
fn phase_of<'a>(row: &'a str, prefix: &str) -> TestResult<&'a str> {
    row.strip_prefix(prefix)
        .ok_or_else(|| format!("{row:?} starts {prefix:?}").into())
}
/// `HOLD_PAID`'s columns up to the phase.
fn paid_prefix(licence: &str, form: &str) -> String {
    format!(
        "  licence {licence:<8}  {:<9}  {:<20}  {form:<11}  ",
        "paid", "sentinel"
    )
}
/// `HOLD_OWNER`'s columns up to the phase.
fn owner_prefix(filed: &str) -> String {
    format!("  filed {filed:<8}  {:<9}  {:<20}  ", "owner", "sentinel")
}
/// A named offer's line (`OFFER`, `CHARTER_OFFER`): `head`, a whole number of
/// minutes left from `lowest` to `highest`, then `tail`.
fn offered(line: &str, head: &str, tail: &str, lowest: i64, highest: i64) -> bool {
    line.strip_prefix(head)
        .and_then(|rest| rest.strip_suffix(tail))
        .and_then(|minutes| minutes.parse::<i64>().ok())
        .is_some_and(|m| (lowest..=highest).contains(&m))
}
/// What part 2 hands part 3.
struct Sold {
    /// The filed number (a decimal string).
    filed: String,
    /// The sealed listing's code and the buyer's sealed licence.
    sealed_code: String,
    sealed: String,
    /// The open, `transfer=no` licence.
    open: String,
    /// The third account's character.
    cy: ObjId,
}
/// Every holdings view the restarts compare: the author's, the buyer's and,
/// in part 3, Cy's.
async fn views(w: &World, actors: &[ObjId]) -> TestResult<Vec<Vec<String>>> {
    let mut all = Vec::new();
    for actor in actors {
        let _ = w.server.take_seen(*actor)?;
        all.push(w.server.command(*actor, "+minds/holdings".into()).await?);
    }
    Ok(all)
}
/// Stop, then boot Active again with Cy connected too.
async fn restart(w: &mut World, cy: ObjId) -> TestResult {
    w.server.stop().await?;
    w.server = active(&w.launch, w.alt, w.bystander).await?;
    w.server.connect(cy).await?;
    Ok(())
}

/// Part 2 (17g C2c's brief, the lead's ruling 1 after C2g, built by 17g
/// C2h): publish, a foreign filed number refused like a missing one
/// (`NO_FILED` under `same_refusal`), read the listing alike for three
/// principals, the alt pair
/// refused by the Quote provider (the lobby's login identities), stale terms
/// (an expired quote is `NO_QUOTE` on `/buy/pay` and `QUOTE_EXPIRED` on
/// `+minds <code>`, then a fresh quote), the purchase with every account's
/// sum unchanged, both holdings, the open `transfer=no` listing bought and
/// forked, Cy's account made for part 3, and a restart.
async fn part_two(w: &mut World, filed: &str) -> TestResult<Sold> {
    let p = w.launch.principals;
    let (author, buyer) = (p.author, p.buyer);
    // The third account's character for part 3, made now so the restart
    // below hydrates its login identity (the account oracle is built at
    // boot); it waits at the counter with a funded personal account. The
    // alt gets a personal account too, so its purchase is refused for the
    // alt pair alone.
    let cy_login = auth_account(&w.launch.store, "launch-cy", None).await?;
    let cy = character(&w.server, &w.launch.store, ("launch-cy", cy_login), CY).await?;
    let room = w.launch.deployment.venue.room;
    w.server
        .command(mush_core::GOD, format!("@teleport #{}=#{}", cy.0, room.0))
        .await?;
    let cy_wallet = open_wallet(&w.server, &w.launch.store, PrincipalId::Object(cy)).await?;
    let alt_wallet = open_wallet(&w.server, &w.launch.store, PrincipalId::Object(w.alt)).await?;
    let faucet = wallet(
        w,
        &PrincipalId::System(mush_org::SystemPrincipalId::new("launch.faucet").ok_or("faucet")?),
    )
    .await?;
    for (account, amount) in [(cy_wallet, 100_000), (alt_wallet, 100_000)] {
        acknowledged(
            &w.server,
            format!("@econ/mint {}={} {amount}", faucet.id.0, account.0),
            "Minted",
        )
        .await?;
    }
    acknowledged(
        &w.server,
        format!(
            "@econ/mint {}={} 200000",
            faucet.id.0,
            wallet(w, &PrincipalId::Object(buyer)).await?.id.0
        ),
        "Minted",
    )
    .await?;

    // Publish the sealed listing at the counter.
    check_eq!(
        submitted(w, author, format!("@mind/publish {filed} form=sealed")).await?,
        vec![
            format!("Listing filed mind {filed} here as sealed."),
            QUEUED.to_owned(),
            "Done.".to_owned()
        ]
    );
    // A foreign filed number reads like a missing one (the C2c brief's test
    // 8, whose real foreign half is this story's; the C2e plan §5.2): the
    // buyer (another login) and the alt (the author's login, another
    // character), both at the counter, naming the author's real filed number
    // and a number nobody filed read `NO_FILED` alike, and no listing is
    // filed.
    const MISSING: &str = "987654";
    let listings = rows(&w.launch.store, "market", "market_mind_entitlement_listing")
        .await?
        .len();
    for (actor, name) in [(buyer, "buyer"), (w.alt, "alt")] {
        let foreign = w
            .server
            .command(actor, format!("@mind/publish {filed} form=sealed"))
            .await?;
        let missing = w
            .server
            .command(actor, format!("@mind/publish {MISSING} form=sealed"))
            .await?;
        check_eq!(
            foreign,
            vec![format!("You own no filed mind {filed}.")],
            "the {name}"
        );
        check!(
            same_refusal(&foreign, filed, &missing, MISSING),
            "the {name}: {foreign:?} vs {missing:?}"
        );
    }
    check_eq!(
        rows(&w.launch.store, "market", "market_mind_entitlement_listing")
            .await?
            .len(),
        listings,
        "no listing is filed for a filed mind the actor does not own"
    );
    // One row, byte-identical for the buyer, the alt and the bystander.
    let mut seen = Vec::new();
    for actor in [buyer, w.alt, w.bystander] {
        seen.push(w.server.command(actor, "+minds".into()).await?);
    }
    let listing = seen.first().cloned().ok_or("the buyer's +minds")?;
    for other in &seen {
        check_eq!(other, &listing, "+minds reads alike for every principal");
    }
    let [head, row, foot]: [String; 3] = listing
        .clone()
        .try_into()
        .map_err(|_| format!("one listing row: {listing:?}"))?;
    check_eq!(head, "Minds for sale here (term and Grace in IC days):");
    check_eq!(
        foot,
        "Use +minds <code> for the full terms and +minds/buy <code> to ask for a quote."
    );
    let code = row
        .split_whitespace()
        .next()
        .ok_or("the row's code")?
        .to_owned();
    check_eq!(code.len(), 8, "{row}");
    let class = row
        .split_once("class ")
        .and_then(|(_, rest)| rest.split_whitespace().next())
        .ok_or("the row's class")?
        .to_owned();
    check!(
        row.starts_with(&format!(
            "  {code}  {:<24}  {:<11}  class ",
            "sentinel", "sealed"
        )) && row.contains("400cr")
            && row.ends_with("90+3 days"),
        "{row}"
    );
    let detail = vec![
        format!("Listing {code}: sentinel, sealed, class {class} and above."),
        "One seat for 90 IC days plus 3 days of Grace; the licence can be resold.".to_owned(),
        "Price 400cr: Registry 5%, charter owner the rest.".to_owned(),
    ];
    let with = |last: String| -> Vec<String> {
        let mut lines = detail.clone();
        lines.push(last);
        lines
    };
    check_eq!(
        w.server.command(buyer, format!("+minds {code}")).await?,
        with(format!("Ask for a quote with +minds/buy {code}."))
    );
    let quoting = format!(
        "Asking for a quote on listing {code}. +minds {code} shows it once it is confirmed."
    );

    // The alt pair: the alt's purchase is refused by the Quote provider (the
    // alt and the author share one login); no quote, hold or licence.
    check_eq!(
        submitted(w, w.alt, format!("+minds/buy {code}")).await?,
        vec![quoting.clone(), QUEUED.to_owned(), DRIVER.to_owned()]
    );
    check!(
        rows(&w.launch.store, "market", "market_mind_entitlement_quote")
            .await?
            .is_empty()
    );
    check!(paid_licences(w, w.alt).await?.is_empty());

    // Stale terms (the lead's ruling 1 after C2g): a quote, left to expire
    // (the story's 30-second quote), is NO_QUOTE on `/buy/pay` and
    // QUOTE_EXPIRED on `+minds <code>`; nothing is held.
    check_eq!(
        submitted(w, buyer, format!("+minds/buy {code}")).await?,
        vec![quoting.clone(), QUEUED.to_owned(), "Done.".to_owned()]
    );
    let quoted = w.server.command(buyer, format!("+minds {code}")).await?;
    check_eq!(quoted.get(..3), Some(detail.as_slice()), "{quoted:?}");
    check!(
        quoted.len() == 4
            && quoted
                .last()
                .is_some_and(|l| l.starts_with("Your quote: 400cr, good for ")
                    && l.ends_with(&format!(
                        " more seconds. Pay it with +minds/buy/pay {code}."
                    ))),
        "{quoted:?}"
    );
    tokio::time::sleep(Duration::from_secs(31)).await;
    check_eq!(
        w.server
            .command(buyer, format!("+minds/buy/pay {code}"))
            .await?,
        vec![format!(
            "You have no quote on listing {code}. Ask for one with +minds/buy {code}."
        )]
    );
    check_eq!(
        w.server.command(buyer, format!("+minds {code}")).await?,
        with(format!(
            "Your quote has expired. Ask for a new one with +minds/buy {code}."
        ))
    );
    check!(
        rows(
            &w.launch.store,
            "market",
            "market_mind_entitlement_purchase"
        )
        .await?
        .is_empty()
    );

    // A fresh quote, paid into escrow and completed: one paid licence, the
    // Registry +20cr, the author +380cr, the buyer -400cr, every account's
    // sum unchanged.
    let before = (
        minor(w, buyer).await?,
        balances(w, author).await?,
        total(w).await?,
    );
    check_eq!(
        submitted(w, buyer, format!("+minds/buy {code}")).await?,
        vec![quoting.clone(), QUEUED.to_owned(), "Done.".to_owned()]
    );
    check_eq!(
        submitted(w, buyer, format!("+minds/buy/pay {code}")).await?,
        vec![
            format!("Paying your quote on listing {code} into escrow."),
            QUEUED.to_owned(),
            "Done.".to_owned()
        ]
    );
    check_eq!(
        w.server.command(buyer, format!("+minds {code}")).await?,
        with(format!(
            "Your payment is held in escrow. Take the licence with +minds/buy/complete {code}."
        ))
    );
    check_eq!(
        total(w).await?,
        before.2,
        "escrow moves money, it makes none"
    );
    let waiting = w.server.command(buyer, "+minds/holdings".into()).await?;
    check_eq!(
        waiting,
        vec![
            "You hold no mind licences.".to_owned(),
            "Purchases waiting:".to_owned(),
            format!(
                "  listing {code}  paid into escrow: +minds/buy/complete {code} or +minds/buy/refund {code}"
            ),
        ]
    );
    check_eq!(
        submitted(w, buyer, format!("+minds/buy/complete {code}")).await?,
        vec![
            format!("Taking the licence from listing {code}."),
            QUEUED.to_owned(),
            "Done.".to_owned()
        ]
    );
    let [sealed]: [u64; 1] = holds(w, buyer, 1)
        .await?
        .try_into()
        .map_err(|_| "one sealed licence")?;
    let sealed = sealed.to_string();
    let after = (
        minor(w, buyer).await?,
        balances(w, author).await?,
        total(w).await?,
    );
    check_eq!(after.0, before.0 - 40_000, "the buyer pays 400cr");
    check_eq!(
        after.1,
        (
            Amount::from_minor(before.1.0.minor() + 38_000),
            Amount::from_minor(before.1.1.minor() + 2_000)
        ),
        "the author +380cr, the Registry +20cr"
    );
    check_eq!(after.2, before.2, "every account's sum is unchanged");

    // Both holdings: the buyer's paid row, the author's owner row.
    let held = w.server.command(buyer, "+minds/holdings".into()).await?;
    check_eq!(held.len(), 2, "{held:?}");
    check_eq!(
        held.first().map(String::as_str),
        Some("Your mind licences:")
    );
    let phase = phase_of(
        held.get(1).ok_or("the paid row")?,
        &paid_prefix(&sealed, "sealed"),
    )?;
    check!(
        phase.starts_with("Active, ") && phase.ends_with(" days left"),
        "{phase}"
    );
    let owned = w.server.command(author, "+minds/holdings".into()).await?;
    check_eq!(owned.len(), 2, "{owned:?}");
    let phase = phase_of(owned.get(1).ok_or("the owner row")?, &owner_prefix(filed))?;
    check!(
        phase.starts_with("Active, ") && phase.ends_with(" days left"),
        "{phase}"
    );

    // The open, transfer=no listing; the buyer buys it and forks it.
    check_eq!(
        submitted(
            w,
            author,
            format!("@mind/publish {filed} form=open transfer=no")
        )
        .await?,
        vec![
            format!("Listing filed mind {filed} here as open; no resale."),
            QUEUED.to_owned(),
            "Done.".to_owned()
        ]
    );
    let listing = w.server.command(buyer, "+minds".into()).await?;
    let open_code = listing
        .iter()
        .filter(|line| line.starts_with("  ") && line.contains("no resale"))
        .filter_map(|line| line.split_whitespace().next())
        .find(|found| *found != code)
        .ok_or_else(|| format!("the open listing: {listing:?}"))?
        .to_owned();
    for (line, request) in [
        (
            format!("+minds/buy {open_code}"),
            format!(
                "Asking for a quote on listing {open_code}. +minds {open_code} shows it once it is confirmed."
            ),
        ),
        (
            format!("+minds/buy/pay {open_code}"),
            format!("Paying your quote on listing {open_code} into escrow."),
        ),
        (
            format!("+minds/buy/complete {open_code}"),
            format!("Taking the licence from listing {open_code}."),
        ),
    ] {
        check_eq!(
            submitted(w, buyer, line.clone()).await?,
            vec![request, QUEUED.to_owned(), "Done.".to_owned()],
            "{line}"
        );
    }
    let open = holds(w, buyer, 2)
        .await?
        .into_iter()
        .map(|n| n.to_string())
        .find(|n| *n != sealed)
        .ok_or("the open licence")?;
    let fork = submitted(w, buyer, format!("@mind/fork {open}=Sentinel Fork")).await?;
    check!(
        fork.len() == 3
            && fork
                .first()
                .is_some_and(|l| l
                    .eq_ignore_ascii_case(&format!("Forking licence {open} as Sentinel Fork.")))
            && fork.get(1..) == Some(&[QUEUED.to_owned(), "Done.".to_owned()][..]),
        "{fork:?}"
    );
    check_eq!(
        rows(&w.launch.store, "roll", "roll_mind_draft_parent")
            .await?
            .len(),
        1,
        "an attributed draft in the buyer's library"
    );

    // Restart: holdings and `+minds` unchanged.
    let views_before = views(w, &[author, buyer]).await?;
    let minds_before = w.server.command(buyer, "+minds".into()).await?;
    let money_before = (
        balances(w, author).await?,
        minor(w, buyer).await?,
        total(w).await?,
    );
    restart(w, cy).await?;
    check_eq!(views(w, &[author, buyer]).await?, views_before);
    check_eq!(
        w.server.command(buyer, "+minds".into()).await?,
        minds_before
    );
    check_eq!(
        (
            balances(w, author).await?,
            minor(w, buyer).await?,
            total(w).await?
        ),
        money_before
    );
    Ok(Sold {
        filed: filed.to_owned(),
        sealed_code: code,
        sealed,
        open,
        cy,
    })
}

/// Part 3 (17g C2d's brief, the C2e plan §5.2, the lead's ruling 2 after
/// C2g, built by 17g C2h): two resale offers at tiny prices to Cy, Cy's
/// listing and confirmed accept (one balanced split), the stale offer and
/// the `transfer=no` licence refused, the self-offer, renewal not due,
/// withdrawal, the charter offer and accept, the renewal after the term
/// (clearing what stands, the holder's retry, a strict decline, the retry,
/// the offer at the market row's frozen price, the payment, funded, the
/// grant; the front's words reaching the holder alone), revocation and
/// recall, and a restart. `NO_RENEWAL` is pinned in term.
async fn part_three(w: &mut World, sold: &Sold) -> TestResult {
    let p = w.launch.principals;
    let (author, buyer, cy) = (p.author, p.buyer, sold.cy);
    let (filed, sealed, open, code) = (&sold.filed, &sold.sealed, &sold.open, &sold.sealed_code);

    // Two offers of the sealed licence to Cy, at 5.00 and at 0.21.
    for (typed, price, amount) in [("5.00", "5cr", "5"), ("0.21", "0.21cr", "0.21")] {
        check_eq!(
            submitted(
                w,
                buyer,
                format!("+minds/resell {sealed}={CY} price={typed}")
            )
            .await?,
            vec![
                format!("Offering licence {sealed} to {CY} for {price}."),
                format!(
                    "Once that is confirmed, they accept with +minds/resell/accept {sealed} price={amount}."
                ),
                QUEUED.to_owned(),
                "Done.".to_owned()
            ]
        );
    }
    // The self-offer edge (the lead's ruling 2 after C2g): refused before
    // anything is submitted.
    check_eq!(
        w.server
            .command(buyer, format!("+minds/resell {sealed}=LaunchBuyer price=1"))
            .await?,
        vec!["No player is named LaunchBuyer.".to_owned()]
    );
    // Cy lists both offers, cheapest first, and nothing moves.
    let before = (
        minor(w, buyer).await?,
        minor(w, cy).await?,
        registry_minor(w, author).await?,
        total(w).await?,
    );
    let listed = w
        .server
        .command(cy, format!("+minds/resell/accept {sealed}"))
        .await?;
    let head = format!("Licence {sealed} is offered to you for ");
    check!(
        listed.len() == 4
            && listed.first().is_some_and(|l| offered(
                l,
                &format!("{head}0.21cr; the offer is open "),
                " more minutes.",
                1_400,
                1_440
            ))
            && listed.get(1).map(String::as_str)
                == Some(
                    format!("Accept it with +minds/resell/accept {sealed} price=0.21.").as_str()
                )
            && listed.get(2).is_some_and(|l| offered(
                l,
                &format!("{head}5cr; the offer is open "),
                " more minutes.",
                1_400,
                1_440
            ))
            && listed.get(3).map(String::as_str)
                == Some(format!("Accept it with +minds/resell/accept {sealed} price=5.").as_str()),
        "{listed:?}"
    );
    check_eq!(
        (
            minor(w, buyer).await?,
            minor(w, cy).await?,
            registry_minor(w, author).await?,
            total(w).await?
        ),
        before
    );
    // Cy confirms 0.21: Cy holds the licence and the buyer does not; the
    // Registry +0.01cr and the buyer +0.20cr at the provisional rates, one
    // balanced split (tiny amounts).
    check_eq!(
        submitted(w, cy, format!("+minds/resell/accept {sealed} price=0.21")).await?,
        vec![
            format!("Accepting the offer for licence {sealed} at 0.21cr."),
            QUEUED.to_owned(),
            "Done.".to_owned()
        ]
    );
    check_eq!(holds(w, cy, 1).await?, vec![sealed.parse::<u64>()?]);
    check!(
        !paid_licences(w, buyer)
            .await?
            .contains(&sealed.parse::<u64>()?)
    );
    check_eq!(
        (
            minor(w, buyer).await?,
            minor(w, cy).await?,
            registry_minor(w, author).await?,
            total(w).await?
        ),
        (before.0 + 20, before.1 - 21, before.2 + 1, before.3)
    );
    // The 5.00 offer can no longer be honoured: nothing moves, whatever the
    // reply (the provider's refusal while the offer is still listed, or no
    // offer once it is not).
    let no_offer = format!("You have no offer for licence {sealed}.");
    let stale = later(
        w,
        cy,
        format!("+minds/resell/accept {sealed} price=5"),
        |l| answered(l) || l == no_offer,
    )
    .await?;
    check!(
        stale
            == vec![
                format!("Accepting the offer for licence {sealed} at 5cr."),
                QUEUED.to_owned(),
                DRIVER.to_owned()
            ]
            || stale == vec![no_offer.clone()],
        "{stale:?}"
    );
    check_eq!(holds(w, cy, 1).await?, vec![sealed.parse::<u64>()?]);
    check_eq!(
        (
            minor(w, buyer).await?,
            minor(w, cy).await?,
            registry_minor(w, author).await?,
            total(w).await?
        ),
        (before.0 + 20, before.1 - 21, before.2 + 1, before.3)
    );
    // The transfer=no licence cannot be resold (the provider's refusal).
    check_eq!(
        submitted(w, buyer, format!("+minds/resell {open}={CY} price=1")).await?,
        vec![
            format!("Offering licence {open} to {CY} for 1cr."),
            format!(
                "Once that is confirmed, they accept with +minds/resell/accept {open} price=1."
            ),
            QUEUED.to_owned(),
            DRIVER.to_owned()
        ]
    );
    // Renewal is not due in term: the service's front answers after the
    // hand-off, and no renewal operation appears.
    let not_due = |l: &str| {
        l.starts_with(&format!(
            "Licence {sealed} is not due for renewal; its term has "
        )) && l.ends_with(" IC days left.")
    };
    let renew = private_later(w, cy, format!("+minds/renew {sealed}"), not_due).await?;
    check!(
        renew.len() == 1 && renew.iter().all(|l| not_due(l)),
        "{renew:?}"
    );
    // With no offer waiting, `/pay` and `/decline` are TacSim's own
    // synchronous refusal (`NO_RENEWAL`), and nothing is handed over.
    let no_renewal = format!(
        "Licence {sealed} has no renewal offer waiting for you. Ask for one with +minds/renew {sealed}."
    );
    for form in ["pay", "decline"] {
        check_eq!(
            w.server
                .command(cy, format!("+minds/renew/{form} {sealed}"))
                .await?,
            vec![no_renewal.clone()],
            "/renew/{form} in term"
        );
    }
    check!(
        renewals(w, sealed).await?.is_empty(),
        "no renewal operation"
    );

    // The author withdraws: the Registry stops new sales of the revision
    // (`withdraw_authored_mind`; the entitlement reader refuses a quote on a
    // withdrawn revision), but the counter is policy-current only (17g C2e:
    // `current_policy` reads the listing row, never the revision's status),
    // so both listings still read byte-for-byte, a fresh quote on the sealed
    // one is refused by the provider (the driver's line) with no quote row
    // added, and Cy still holds the licence. Hiding a withdrawn revision's
    // listings is a C4 ledger item (17g section 7, C2e as built).
    let counter = w.server.command(cy, "+minds".into()).await?;
    check!(
        counter.len() == 4
            && counter.first().map(String::as_str)
                == Some("Minds for sale here (term and Grace in IC days):"),
        "{counter:?}"
    );
    let quotes = rows(&w.launch.store, "market", "market_mind_entitlement_quote")
        .await?
        .len();
    check_eq!(
        submitted(w, author, format!("@mind/withdraw {filed}")).await?,
        vec![
            format!("Withdrawing filed revision {filed}."),
            QUEUED.to_owned(),
            "Done.".to_owned()
        ]
    );
    check_eq!(w.server.command(cy, "+minds".into()).await?, counter);
    check_eq!(
        submitted(w, cy, format!("+minds/buy {code}")).await?,
        vec![
            format!(
                "Asking for a quote on listing {code}. +minds {code} shows it once it is confirmed."
            ),
            QUEUED.to_owned(),
            DRIVER.to_owned()
        ]
    );
    check_eq!(
        rows(&w.launch.store, "market", "market_mind_entitlement_quote")
            .await?
            .len(),
        quotes,
        "no quote row for a withdrawn revision"
    );
    check_eq!(holds(w, cy, 1).await?, vec![sealed.parse::<u64>()?]);

    // The charter: the author offers it to Cy at 1.00; Cy reads the offer
    // and accepts it; the clerk's service settles (no sortie is out). The
    // owner row shows in neither holdings view afterwards: the revision is
    // withdrawn and unpinned, which `owner_holdings::check_status` reads as
    // not held (for the author before the transfer, for Cy after it), so the
    // settlement is read from the store's pending row.
    check_eq!(
        submitted(w, author, format!("+minds/charter {filed}={CY} price=1.00")).await?,
        vec![
            format!("Offering filed mind {filed} and its charter to {CY} for 1cr."),
            format!(
                "Once that is confirmed, they accept with +minds/charter/accept {filed} price=1."
            ),
            QUEUED.to_owned(),
            "Done.".to_owned()
        ]
    );
    let listed = w
        .server
        .command(cy, format!("+minds/charter/accept {filed}"))
        .await?;
    check!(
        listed.len() == 2
            && listed.first().is_some_and(|l| offered(
                l,
                &format!(
                    "Filed mind {filed} and its charter are offered to you for 1cr; the offer is open "
                ),
                " more minutes.",
                1_400,
                1_440
            ))
            && listed.get(1).map(String::as_str)
                == Some(format!("Accept it with +minds/charter/accept {filed} price=1.").as_str()),
        "{listed:?}"
    );
    check_eq!(
        submitted(w, cy, format!("+minds/charter/accept {filed} price=1")).await?,
        vec![
            format!(
                "Paying 1cr for filed mind {filed} into escrow; the transfer settles once no sortie under it is out."
            ),
            QUEUED.to_owned(),
            "Done.".to_owned()
        ]
    );
    // No sortie is out, so the clerk's last-claim-closed reason applies at
    // once: the one pending row leaves `funded` for `settled`, its transfer a
    // later ownership generation than the offer's.
    let end = Instant::now() + STEP;
    let settled = loop {
        let _ = w.server.take_seen(cy)?;
        let pending = rows(&w.launch.store, "roll", "roll_mind_charter_pending").await?;
        if let Some(row) = pending
            .iter()
            .find(|row| row.pointer("/state/phase").and_then(Value::as_str) == Some("settled"))
        {
            break row.clone();
        }
        check!(
            Instant::now() < end,
            "the charter settles to Cy; the pending rows: {pending:?}"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    };
    let generation = |path: &str| settled.pointer(path).and_then(Value::as_u64);
    check!(
        generation("/state/transfer/generation") > generation("/ownership/generation"),
        "{settled}"
    );
    for (who, name) in [(author, "the author"), (cy, "Cy")] {
        let held = w.server.command(who, "+minds/holdings".into()).await?;
        check!(
            !held.iter().any(|l| l.starts_with(&owner_prefix(filed))),
            "{name}'s holdings after the transfer of a withdrawn revision: {held:?}"
        );
    }

    // The renewal after the term. The IC clock runs fast (staff's
    // `@tacsim/time`, one IC hour per real second) to four IC hours before
    // the sealed licence's term ends, then back at the launch's rate (24),
    // past the end, so the renewal offer is drafted and paid under the last
    // world-IC generation (a rate change opens one, and an offer drafted
    // under an earlier one is the service's to close).
    let end_of_term = term_end(w, sealed, "1").await?;
    rate(w, 3600).await?;
    let guard = Instant::now() + Duration::from_secs(3 * 3_600);
    while ic_now(w).await?.0 < end_of_term - 4 * 3_600 {
        check!(
            Instant::now() < guard,
            "the fast IC clock reaches the term's end"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    rate(w, 24).await?;
    while ic_now(w).await?.0 < end_of_term + 60 {
        check!(Instant::now() < guard, "the term ends");
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    // The service may draft an offer on its own once the term has ended (it
    // never pays one, D-UM-39); give it a moment, then read what stands.
    tokio::time::sleep(Duration::from_secs(20)).await;
    let renewing = format!(
        "Asking for a renewal offer on licence {sealed}. +minds/renew {sealed} shows it once it is confirmed."
    );
    let how = format!(
        "Pay it with +minds/renew/pay {sealed}, or turn it down with +minds/renew/decline {sealed}."
    );
    let unavailable = "Renewal is unavailable or has changed.";
    let shown = |l: &str| l == how || l == unavailable || answered(l);
    let mut offer = None;
    for _ in 0..12 {
        let _ = w.server.take_seen(cy)?;
        let reply = later(w, cy, format!("+minds/renew {sealed}"), shown).await?;
        if reply.last().map(String::as_str) == Some(how.as_str()) {
            offer = Some(reply);
            break;
        }
        if reply.first() == Some(&renewing) {
            check_eq!(
                reply,
                vec![renewing.clone(), QUEUED.to_owned(), "Done.".to_owned()]
            );
            renewal_reaches(w, sealed, "drafted").await?;
        } else {
            // The uniform refusal while the service's own attempt is in
            // flight, or that attempt's own `Queued.` and confirmation
            // reaching Cy first: read again shortly.
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }
    let offer = offer.ok_or("a renewal offer stands")?;
    check_eq!(offer.len(), 2, "{offer:?}");
    // Clear what stands. The offer may be the service's own draft, or one
    // drafted under an earlier world-IC generation that the service closes
    // by itself (`close_reason`), so this first decline may be refused; it
    // is NOT the decline's proof, which is the strict one below, on an offer
    // the holder drafted under this generation.
    let declined = later(w, cy, format!("+minds/renew/decline {sealed}"), |l| {
        answered(l) || l == unavailable || l == no_renewal
    })
    .await?;
    let declining = format!("Turning down the renewal offer on licence {sealed}.");
    check!(
        declined.first() == Some(&declining)
            || declined == vec![unavailable.to_owned()]
            || declined == vec![no_renewal.clone()],
        "{declined:?}"
    );
    let end = Instant::now() + STEP;
    loop {
        let all = renewals(w, sealed).await?;
        if all.last().is_some_and(|(k, _)| k.starts_with("unfunded_")) {
            break;
        }
        check!(Instant::now() < end, "the standing offer closes: {all:?}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    // The holder's own retry (`+minds/renew` is request-or-retry), drafted
    // under the current generation, read, then turned down, strictly: the
    // holder's closure (`unfunded_closed`, not the service's).
    settle(w, cy).await?;
    check_eq!(
        later(w, cy, format!("+minds/renew {sealed}"), answered).await?,
        vec![renewing.clone(), QUEUED.to_owned(), "Done.".to_owned()]
    );
    drafted_now(w, sealed).await?;
    settle(w, cy).await?;
    let own_offer = private_later(w, cy, format!("+minds/renew {sealed}"), |l| l == how).await?;
    check!(
        own_offer.len() == 2 && own_offer.get(1) == Some(&how),
        "the holder's own offer: {own_offer:?}"
    );
    check_eq!(
        private_later(w, cy, format!("+minds/renew/decline {sealed}"), answered).await?,
        vec![declining.clone(), QUEUED.to_owned(), "Done.".to_owned()]
    );
    renewal_reaches(w, sealed, "unfunded_closed").await?;
    // Ask again, read the offer (its frozen price), pay it.
    settle(w, cy).await?;
    check_eq!(
        later(w, cy, format!("+minds/renew {sealed}"), answered).await?,
        vec![renewing.clone(), QUEUED.to_owned(), "Done.".to_owned()]
    );
    drafted_now(w, sealed).await?;
    settle(w, cy).await?;
    let offer = private_later(w, cy, format!("+minds/renew {sealed}"), |l| l == how).await?;
    let price = offer
        .first()
        .and_then(|l| l.strip_prefix(&format!("Licence {sealed} renews for ")))
        .and_then(|rest| rest.split_once(" (90+3 IC days); the offer is open "))
        .and_then(|(price, rest)| rest.ends_with(" more IC hours.").then_some(price))
        .ok_or_else(|| format!("RENEW_OFFER: {offer:?}"))?
        .to_owned();
    check_eq!(offer.get(1), Some(&how));
    let price_minor = minor_of(&price)?;
    check_eq!(
        frozen_price(w, sealed).await?,
        price_minor,
        "RENEW_OFFER shows the market row's frozen price"
    );
    let held = w.server.command(cy, "+minds/holdings".into()).await?;
    check!(
        held.iter()
            .any(|l| l.starts_with(&paid_prefix(sealed, "sealed")) && l.ends_with("; renew offer")),
        "{held:?}"
    );
    // Pay it: the frozen price into escrow (the Fund, whose `Done.` is
    // commit-bound, so the newest operation is funded, or already granted by
    // the service's own continuation); the service grants it.
    let paid_before = (
        minor(w, cy).await?,
        minor(w, author).await?,
        minor(w, buyer).await?,
        total(w).await?,
    );
    settle(w, cy).await?;
    check_eq!(
        private_later(w, cy, format!("+minds/renew/pay {sealed}"), answered).await?,
        vec![
            format!(
                "Paying {price} for the renewal of licence {sealed} into escrow; it completes on its own."
            ),
            QUEUED.to_owned(),
            "Done.".to_owned()
        ]
    );
    let funded = renewals(w, sealed).await?.last().map(|(k, _)| k.clone());
    check!(
        matches!(funded.as_deref(), Some("funded" | "granted")),
        "the Fund committed: {funded:?}"
    );
    renewal_reaches(w, sealed, "granted").await?;
    check!(
        rows(&w.launch.store, "roll", "roll_mind_renewal_operation")
            .await?
            .iter()
            .any(
                |row| row.pointer("/binding/licence").and_then(Value::as_str)
                    == Some(sealed.as_str())
                    && row.pointer("/state/kind").and_then(Value::as_str) == Some("granted")
                    && row.pointer("/state/funding").is_some()
            ),
        "the grant carries the holder's funding"
    );
    // The money: every account's sum unchanged; the author and the buyer
    // untouched (the renewal's remainder goes to the charter's CURRENT title
    // owner, the renewal reader's seller, which since the charter transfer
    // above is Cy, so Cy's net debit is the Registry's and the venue's cuts
    // alone, at most the frozen price).
    let debit = paid_before.0 - minor(w, cy).await?;
    check!(
        (0..=price_minor).contains(&debit),
        "Cy's net debit {debit} within the frozen price {price_minor}"
    );
    check_eq!(
        (
            minor(w, author).await?,
            minor(w, buyer).await?,
            total(w).await?
        ),
        (paid_before.1, paid_before.2, paid_before.3),
        "the renewal moves money, it makes none"
    );
    term_end(w, sealed, "2").await?;
    settle(w, cy).await?;
    let renewed = private_later(w, cy, format!("+minds/renew {sealed}"), not_due).await?;
    check!(
        renewed.len() == 1 && renewed.iter().all(|l| not_due(l)),
        "{renewed:?}"
    );

    // The Registry operator revokes Cy's licence (Cy's row reads `; revoked`)
    // and recalls the filed revision; the buyer's identical recall reads
    // like a recall of a number that does not exist.
    let operator = p.operator;
    check_eq!(
        submitted(w, operator, format!("@registry/revoke {sealed}")).await?,
        vec![
            format!("Revoking licence {sealed}."),
            QUEUED.to_owned(),
            "Done.".to_owned()
        ]
    );
    let _ = w.server.take_seen(cy)?;
    let held = w.server.command(cy, "+minds/holdings".into()).await?;
    check!(
        held.iter()
            .any(|l| l.starts_with(&paid_prefix(sealed, "sealed")) && l.ends_with("; revoked")),
        "{held:?}"
    );
    check_eq!(
        submitted(w, operator, format!("@registry/recall {filed}")).await?,
        vec![
            format!("Recalling filed revision {filed}."),
            QUEUED.to_owned(),
            "Done.".to_owned()
        ]
    );
    const MISSING: &str = "999";
    let foreign = submitted(w, buyer, format!("@registry/recall {filed}")).await?;
    let missing = submitted(w, buyer, format!("@registry/recall {MISSING}")).await?;
    check!(
        same_refusal(&foreign, filed, &missing, MISSING),
        "{foreign:?} vs {missing:?}"
    );

    // Restart: every holdings view and balance is unchanged.
    let views_before = views(w, &[author, buyer, cy]).await?;
    let money_before = (
        balances(w, author).await?,
        minor(w, buyer).await?,
        minor(w, cy).await?,
        total(w).await?,
    );
    restart(w, cy).await?;
    check_eq!(views(w, &[author, buyer, cy]).await?, views_before);
    check_eq!(
        (
            balances(w, author).await?,
            minor(w, buyer).await?,
            minor(w, cy).await?,
            total(w).await?
        ),
        money_before
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "17g C2 command story: one genuine Registry rating; run by the mind-commerce-acceptance dispatch"]
async fn player_market_commands_story_two_accounts_alt_stale_tiny_and_restart() -> TestResult {
    let market = tacsim::launch::MarketTerms {
        quote_seconds: 30,
        ..tacsim::launch::MarketTerms::PROVISIONAL
    };
    let mut w = world_with_terms("mush-c2-story", market).await?;
    let result = timeout(STORY_GUARD, async {
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
        // C2c: part 2 (publish, +minds, the alt pair, stale terms, the
        // purchase, holdings, the open listing and the paid fork, a restart).
        let sold = Box::pin(part_two(&mut w, &filed_number)).await?;
        // C2d: part 3 (resale offers, renewal, the withdrawal, the charter
        // transfer, the renewal after the term, revocation and recall, a
        // restart).
        Box::pin(part_three(&mut w, &sold)).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })
    .await;
    w.server.stop().await?;
    let _ = std::fs::remove_dir_all(&w.data_dir);
    result?
}
