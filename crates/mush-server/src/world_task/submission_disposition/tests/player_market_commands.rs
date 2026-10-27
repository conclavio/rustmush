//! 17g C2a (C2 plan §4): the author's line-mode commands over C1's launch
//! harness, through `boot::game_modules` and the production launch document
//! (format 2, Active, the operator granted by `@org/grant`). Every command is
//! an `Inbound::Cmd` line typed by a connected character: an author and a
//! buyer on two login accounts, the author's alt on the author's login, a
//! bystander in the counter room, God, the clerk and the operator. None of
//! these needs a Registry rating, so they fit the focused gate; the positive
//! filing is the ignored story's (`player_market_story`).
//!
//! The comparison rule (plan §2.8) is `same_refusal` and `assert_private`;
//! C2c and C2d reuse both, with `world` and `saved`.
use super::paid_purchase::{
    auth_account,
    runtime::{Running, STEP, Seen, rows},
};
use super::player_market::{
    LAUNCH_AUTHOR, Launch, character, initialized_launch_with, launch_active, scratch,
};
use mush_core::{GOD, ObjId};
use mush_store::ObjectStore;
use serde_json::Value;
use std::time::Duration;
use tokio::time::{Instant, timeout};

pub(super) type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// What no reply may carry: the known revision and rating digests and the
/// known account ids (plan §2.8).
#[derive(Clone, Debug, Default)]
pub(super) struct Known {
    pub(super) digests: Vec<String>,
    pub(super) accounts: Vec<u64>,
}

/// The comparison rule: each reply with the number its actor typed replaced
/// by one placeholder, then compared whole. Two replies to a foreign and a
/// missing number must be byte-identical this way.
pub(super) fn same_refusal(a: &[String], typed_a: &str, b: &[String], typed_b: &str) -> bool {
    let mask = |lines: &[String], typed: &str| -> Vec<String> {
        lines.iter().map(|line| mask_number(line, typed)).collect()
    };
    mask(a, typed_a) == mask(b, typed_b)
}
/// `line` with every whole-number occurrence of `typed` replaced by `<n>`
/// (not a digit run that merely contains it).
fn mask_number(line: &str, typed: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(at) = rest.find(typed) {
        let (before, tail) = rest.split_at(at);
        let after = tail.get(typed.len()..).unwrap_or_default();
        let digit_before = before
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_digit());
        let digit_after = after.chars().next().is_some_and(|c| c.is_ascii_digit());
        out.push_str(before);
        if digit_before || digit_after {
            out.push_str(typed);
        } else {
            out.push_str("<n>");
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

/// No line carries a known digest, any 64-character lowercase digest-shaped
/// run, or a known account id in its delimited forms (`"payer":`,
/// `"seller_account":`, `account <id>`). Plain amounts and licence numbers
/// are never matched as account ids.
pub(super) fn assert_private(lines: &[String], known: &Known) -> TestResult {
    for line in lines {
        for digest in &known.digests {
            if line.contains(digest.as_str()) {
                return Err(format!("a known digest in {line:?}").into());
            }
        }
        let mut run = 0;
        for c in line.chars() {
            run = if c.is_ascii_digit() || ('a'..='f').contains(&c) {
                run + 1
            } else {
                0
            };
            if run >= 64 {
                return Err(format!("a digest-shaped run in {line:?}").into());
            }
        }
        for id in &known.accounts {
            for form in [
                format!("\"payer\":{id}"),
                format!("\"seller_account\":{id}"),
                format!("account {id}"),
            ] {
                let mut rest = line.as_str();
                while let Some(at) = rest.find(&form) {
                    let after = rest.get(at + form.len()..).unwrap_or_default();
                    if !after.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                        return Err(format!("account {id} in {line:?}").into());
                    }
                    rest = after;
                }
            }
        }
    }
    Ok(())
}

/// A launch for the command tests, its Active server, the author's alt and a
/// bystander. The alt (on the author's login) and the bystander (its own
/// login) are made in one extra Active boot and placed in the counter room,
/// so the identity every later boot hydrates knows both.
pub(super) struct World {
    pub(super) data_dir: std::path::PathBuf,
    pub(super) launch: Launch,
    pub(super) server: Running,
    pub(super) alt: ObjId,
    pub(super) bystander: ObjId,
}
pub(super) async fn world(prefix: &str, format: u16) -> TestResult<World> {
    world_from(prefix, format, tacsim::launch::MarketTerms::PROVISIONAL).await
}
/// `world` on format 2 with the market's `terms` (the C2 story's).
pub(super) async fn world_with_terms(
    prefix: &str,
    terms: tacsim::launch::MarketTerms,
) -> TestResult<World> {
    world_from(prefix, 2, terms).await
}
async fn world_from(
    prefix: &str,
    format: u16,
    terms: tacsim::launch::MarketTerms,
) -> TestResult<World> {
    let data_dir = scratch(prefix)?;
    let launch = initialized_launch_with(&data_dir, format, terms).await?;
    let store = launch.store.clone();
    let author_account = store
        .list_accounts()
        .await?
        .into_iter()
        .find(|account| account.name == LAUNCH_AUTHOR)
        .and_then(|account| account.id)
        .ok_or("the author's login")?;
    let bystander_account = auth_account(&store, "launch-bystander", None).await?;
    let mut server = launch_active(&launch).await?;
    let alt = character(
        &server,
        &store,
        (LAUNCH_AUTHOR, author_account),
        "LaunchAlt",
    )
    .await?;
    let bystander = character(
        &server,
        &store,
        ("launch-bystander", bystander_account),
        "LaunchBystander",
    )
    .await?;
    for actor in [alt, bystander] {
        server
            .command(
                GOD,
                format!("@teleport #{}=#{}", actor.0, launch.deployment.venue.room.0),
            )
            .await?;
    }
    server.stop().await?;
    let server = active(&launch, alt, bystander).await?;
    Ok(World {
        data_dir,
        launch,
        server,
        alt,
        bystander,
    })
}
/// One Active boot, every principal, the alt and the bystander connected.
pub(super) async fn active(launch: &Launch, alt: ObjId, bystander: ObjId) -> TestResult<Running> {
    let server = launch_active(launch).await?;
    for actor in [alt, bystander] {
        server.connect(actor).await?;
    }
    Ok(server)
}

/// `actor` creates `name` from Standard and saves it; the saved revision is
/// read back from Roll's own row once the bounded author worker publishes it.
pub(super) async fn saved(
    world: &World,
    actor: ObjId,
    name: &str,
) -> TestResult<mush_roll::Revision> {
    for line in [
        format!("@mind/new {name} FROM Standard"),
        format!("@mind/save {name}=first"),
    ] {
        world.server.command(actor, line).await?;
    }
    let end = Instant::now() + STEP;
    loop {
        let found = world
            .launch
            .store
            .module_json_list("roll")
            .await?
            .into_iter()
            .filter(|(key, _)| key.starts_with(&format!("revision:{}:", actor.0)))
            .map(|(_, row)| serde_json::from_value::<mush_roll::Revision>(row))
            .collect::<Result<Vec<_>, _>>()?;
        if let [revision] = found.as_slice() {
            return Ok(revision.clone());
        }
        if Instant::now() >= end {
            return Err(format!("{name} was not saved").into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// `actor`'s own lines until one satisfies `done` (the driver's commit-bound
/// line, a service's later line), within `STEP`. Every line any other
/// connection receives meanwhile is returned beside them.
pub(super) async fn await_own(
    server: &Running,
    actor: ObjId,
    done: impl Fn(&str) -> bool,
) -> TestResult<Seen> {
    let end = Instant::now() + STEP;
    let mut all = Seen::default();
    loop {
        let seen = server.take_seen(actor)?;
        let finished = seen.own.iter().any(|line| done(line));
        all.extend(seen);
        if finished {
            return Ok(all);
        }
        if Instant::now() >= end {
            return Err(format!("no awaited line for #{}: {:?}", actor.0, all.own).into());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// `lines` (a command's own reply), extended with `actor`'s later lines until
/// one satisfies `done`. The driver often answers within the command's own
/// step, so a line already present ends the wait at once.
pub(super) async fn until_own(
    server: &Running,
    actor: ObjId,
    mut lines: Vec<String>,
    done: impl Fn(&str) -> bool,
) -> TestResult<Vec<String>> {
    if !lines.iter().any(|line| done(line)) {
        lines.extend(await_own(server, actor, done).await?.own);
    }
    Ok(lines)
}

/// The open filing shells the store holds for `actor`.
pub(super) async fn open_shells(launch: &Launch, actor: ObjId) -> TestResult<Vec<Value>> {
    Ok(rows(&launch.store, "roll", "open_initial_filing_shells")
        .await?
        .into_iter()
        .filter(|row| row.pointer("/key/requester").and_then(Value::as_u64) == Some(actor.0.into()))
        .collect())
}

const DRIVER: &str = "That did not go through. Nothing changed.";
const NOT_OPEN: &str = "The mind market is not open here.";
/// The engine's receipt acknowledgment of every command-submitted action
/// (`Engine`, E7c2 fold R11): best-effort, between the request line and the
/// driver's line, never the confirmation.
const QUEUED: &str = "Queued.";

/// 17g C2a test 9 (plan §2.8): the missing-target half of the comparison
/// rule and its masking. The fast suite has no filing and no licence (a
/// filing needs a genuine rating), so here both numbers are missing: the
/// first filed and licence number Roll would allocate (1) and one far past
/// it, for the buyer, the author's alt (the author's login, another
/// character) and the author. Each reply is exact and the two read alike
/// under `same_refusal`. The FOREIGN half (a real filed number and a real
/// owner licence another character holds, for `/update`, `/withdraw`,
/// `/attest` and `/fork`, by the buyer and the alt) is the story's
/// (`player_market_story`, part 1), which has them (reviewer F3/T2).
/// `/update`, `/withdraw` and `/attest` answer `NO_FILED`; `/fork` prints its
/// request line, submits, and the provider refuses with the driver's line.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn author_commands_refuse_alike_for_foreign_and_missing_targets() -> TestResult {
    let mut w = world("mush-c2a-alike", 2).await?;
    let result = timeout(STEP * 10, async {
        let p = w.launch.principals;
        saved(&w, p.buyer, "Probe").await?;
        for (actor, name) in [(p.buyer, "buyer"), (w.alt, "alt"), (p.author, "author")] {
            for (template, filed) in [
                ("@mind/update {n} Probe@1", true),
                ("@mind/withdraw {n}", true),
                ("@mind/attest {n}", true),
                ("@mind/fork {n}=Probe Copy", false),
            ] {
                let mut replies = Vec::new();
                for typed in ["1", "987654"] {
                    let line = template.replace("{n}", typed);
                    let mut lines = w.server.command(actor, line.clone()).await?;
                    if !filed {
                        lines = until_own(&w.server, actor, lines, |l| l == DRIVER).await?;
                    }
                    check!(!lines.is_empty(), "{name}: {line} answers");
                    replies.push((lines, typed));
                }
                let [(a, typed_a), (b, typed_b)] =
                    <[_; 2]>::try_from(replies).map_err(|_| "two replies")?;
                check!(
                    same_refusal(&a, typed_a, &b, typed_b),
                    "{name}: {template} reads alike: {a:?} vs {b:?}"
                );
                if filed {
                    check_eq!(a, vec![format!("You own no filed mind {typed_a}.")]);
                } else {
                    check_eq!(
                        a,
                        vec![
                            format!("Forking licence {typed_a} as probe copy."),
                            QUEUED.to_owned(),
                            DRIVER.to_owned()
                        ]
                    );
                }
            }
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })
    .await;
    w.server.stop().await?;
    let _ = std::fs::remove_dir_all(&w.data_dir);
    result?
}

/// 17g C2a test 10 (plan §2.8, the observer rule): no line of any C2a
/// command, its driver's commit-bound or refusal line and the Registry
/// service's later line included, reaches the bystander, the alt or the buyer
/// (all three in the counter room with the author), and no line the author
/// receives names its revision digest or a known account id.
///
/// A positive control first proves the harness sees the room: the author's
/// `say` reaches the bystander's connection. Observers are checked by
/// recipient (`Seen::reaching`), so unrelated traffic to God, the clerk or
/// the operator cannot fail the test (reviewer T5). Lines that arrive after
/// a command's own window are collected before the next command (a settle
/// window), and the last command's later service line is awaited and checked
/// too, so a late line is never discarded unseen.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn author_command_replies_reach_only_the_actor_and_name_no_digest() -> TestResult {
    let mut w = world("mush-c2a-observer", 2).await?;
    let result = timeout(STEP * 10, async {
        let p = w.launch.principals;
        let revision = saved(&w, p.author, "Sentinel").await?;
        let mut accounts = Vec::new();
        for (key, row) in w.launch.store.module_json_list("economy").await? {
            if key.starts_with("account:")
                && let Some(id) = row.get("id").and_then(Value::as_u64)
            {
                accounts.push(id);
            }
        }
        let known = Known {
            digests: vec![revision.reference().digest.to_string()],
            accounts,
        };
        let observers = [
            (w.bystander, "bystander"),
            (w.alt, "alt"),
            (p.buyer, "buyer"),
        ];
        // The positive control: room speech reaches every observer.
        let said = w
            .server
            .command_seen(p.author, "say Testing the counter room.".into())
            .await?;
        for (observer, name) in observers {
            check!(
                said.reaching(observer)
                    .iter()
                    .any(|line| line.contains("Testing the counter room.")),
                "the {name} hears the room: {said:?}"
            );
        }
        let quiet = |seen: &Seen, line: &str| -> TestResult {
            for (observer, name) in observers {
                let reached = seen.reaching(observer);
                check!(reached.is_empty(), "{line} reached the {name}: {reached:?}");
            }
            assert_private(&seen.own, &known)
        };
        let commands = [
            ("@mind/file Sentinel@1", true),
            ("@mind/file Sentinel@1", false),
            ("@mind/update 1 Sentinel@1", false),
            ("@mind/withdraw 1", false),
            ("@mind/attest 1", false),
            ("@mind/fork 1=Sentinel Copy", true),
            ("@mind/rate Sentinel@1", false),
        ];
        let answered = |l: &str| l == DRIVER || l == "Done." || l.ends_with("(already done)");
        let mut previous = "the control";
        for (line, submits) in commands {
            // The settle window: whatever reached anyone since the last check.
            let late = w.server.take_seen(p.author)?;
            quiet(&late, previous)?;
            let mut seen = w.server.command_seen(p.author, line.into()).await?;
            if submits && !seen.own.iter().any(|l| answered(l)) {
                seen.extend(await_own(&w.server, p.author, answered).await?);
            }
            check!(!seen.own.is_empty(), "{line} answers the author");
            quiet(&seen, line)?;
            previous = line;
        }
        // The rating's later service line reaches the author alone.
        let later = await_own(&w.server, p.author, |l| {
            l == "Registry work retained and queued."
        })
        .await?;
        quiet(&later, "the Registry service's later line")?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })
    .await;
    w.server.stop().await?;
    let _ = std::fs::remove_dir_all(&w.data_dir);
    result?
}

/// 17g C2a test 11: the first `@mind/file` reserves an acknowledged charter
/// shell (the request line, then the driver's commit-bound confirmation, and
/// the author's open shell row exists); the second finds it and, with no
/// finished rating, refuses `NOT_RATED`. After a restart (boot-after-the-step)
/// the command still finds the same shell, makes no second reservation, and
/// still refuses `NOT_RATED`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn file_reserves_a_shell_then_waits_for_a_rating_across_a_restart() -> TestResult {
    let mut w = world("mush-c2a-shell", 2).await?;
    let result = timeout(STEP * 10, async {
        let p = w.launch.principals;
        saved(&w, p.author, "Sentinel").await?;
        let reserving = w
            .server
            .command(p.author, "@mind/file Sentinel@1".into())
            .await?;
        let reserving = until_own(&w.server, p.author, reserving, |l| l == "Done." || l == DRIVER).await?;
        check_eq!(
            reserving.first().map(String::as_str),
            Some("Reserving a charter for this filing first. Once that is confirmed, type @mind/file Sentinel@1 again."),
            "the request line first: {reserving:?}"
        );
        check!(
            reserving.iter().any(|line| line == "Done.") && !reserving.iter().any(|line| line == DRIVER),
            "the driver's commit-bound confirmation: {reserving:?}"
        );
        let end = Instant::now() + STEP;
        while open_shells(&w.launch, p.author).await?.is_empty() {
            check!(Instant::now() < end, "the author's open shell row");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let shell = open_shells(&w.launch, p.author).await?;
        check_eq!(shell.len(), 1);
        let not_rated = vec![
            "sentinel@1 has no finished Registry rating. Rate it with @mind/rate sentinel@1."
                .to_owned(),
        ];
        check_eq!(
            w.server
                .command(p.author, "@mind/file Sentinel@1".into())
                .await?,
            not_rated
        );
        w.server.stop().await?;
        w.server = active(&w.launch, w.alt, w.bystander).await?;
        check_eq!(
            w.server
                .command(p.author, "@mind/file Sentinel@1".into())
                .await?,
            not_rated,
            "the shell survives the restart"
        );
        check_eq!(
            open_shells(&w.launch, p.author).await?,
            shell,
            "no second reservation"
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })
    .await;
    w.server.stop().await?;
    let _ = std::fs::remove_dir_all(&w.data_dir);
    result?
}

/// Reviewer T4: `@mind/file` and `@mind/rate` resolve only the actor's own
/// library and its own open shell. The alt (the author's login, another
/// character) and the buyer naming the author's saved revision read the exact
/// `NO_REVISION` line, as does the author naming a revision it never saved;
/// after the author reserves, the alt's own mind reserves a second shell of
/// its own (one row per requester) and the alt's next `@mind/file` finds that
/// shell, not the author's, while the alt and the buyer still cannot name the
/// author's revision.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn file_and_rate_read_only_the_actors_own_revision_and_shell() -> TestResult {
    let mut w = world("mush-c2a-own", 2).await?;
    let result = timeout(STEP * 10, async {
        let p = w.launch.principals;
        saved(&w, p.author, "Sentinel").await?;
        let none = |reference: &str| vec![format!("You have no saved revision {reference}.")];
        let strangers = [(w.alt, "alt"), (p.buyer, "buyer")];
        for (actor, name) in strangers {
            for line in [
                "@mind/file Sentinel@1",
                "@mind/file Sentinel@1=Sentinel Line",
                "@mind/rate Sentinel@1",
            ] {
                check_eq!(
                    w.server.command(actor, line.into()).await?,
                    none("sentinel@1"),
                    "the {name}: {line}"
                );
            }
        }
        for line in ["@mind/file Sentinel@7", "@mind/rate Sentinel@7"] {
            check_eq!(
                w.server.command(p.author, line.into()).await?,
                none("sentinel@7"),
                "{line}"
            );
        }
        let reserved = |l: &str| l == "Done." || l == DRIVER;
        let reserving = w
            .server
            .command(p.author, "@mind/file Sentinel@1".into())
            .await?;
        let reserving = until_own(&w.server, p.author, reserving, reserved).await?;
        check!(
            reserving.iter().any(|l| l == "Done.") && !reserving.iter().any(|l| l == DRIVER),
            "the author's reservation: {reserving:?}"
        );
        saved(&w, w.alt, "Watcher").await?;
        let alt = w
            .server
            .command(w.alt, "@mind/file Watcher@1".into())
            .await?;
        let alt = until_own(&w.server, w.alt, alt, reserved).await?;
        check_eq!(
            alt,
            vec![
                "Reserving a charter for this filing first. Once that is confirmed, type @mind/file Watcher@1 again.".to_owned(),
                QUEUED.to_owned(),
                "Done.".to_owned()
            ],
            "the alt reserves its own shell"
        );
        let end = Instant::now() + STEP;
        loop {
            let (author, alt) = (
                open_shells(&w.launch, p.author).await?,
                open_shells(&w.launch, w.alt).await?,
            );
            if author.len() == 1 && alt.len() == 1 {
                check_ne!(
                    author.first().and_then(|row| row.pointer("/key/request")),
                    alt.first().and_then(|row| row.pointer("/key/request")),
                    "two reservations, one per requester"
                );
                break;
            }
            check!(Instant::now() < end, "one shell each: {author:?} {alt:?}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        check_eq!(
            w.server
                .command(w.alt, "@mind/file Watcher@1".into())
                .await?,
            vec![
                "watcher@1 has no finished Registry rating. Rate it with @mind/rate watcher@1."
                    .to_owned()
            ],
            "the alt's own shell, then its own rating"
        );
        for (actor, name) in strangers {
            check_eq!(
                w.server
                    .command(actor, "@mind/file Sentinel@1".into())
                    .await?,
                none("sentinel@1"),
                "the {name} after the reservations"
            );
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })
    .await;
    w.server.stop().await?;
    let _ = std::fs::remove_dir_all(&w.data_dir);
    result?
}

/// 17g C2a test 12: `@mind/rate` queues a genuine Registry preparation of the
/// author's own saved revision by name; once the job is admitted a second
/// `@mind/rate` refuses `RATING_BUSY`; `@registry/cancel` releases the job,
/// after which `@mind/rate` queues again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rate_queues_and_cancels_a_genuine_registry_preparation() -> TestResult {
    let mut w = world("mush-c2a-rate", 2).await?;
    let result = timeout(STEP * 10, async {
        let p = w.launch.principals;
        saved(&w, p.author, "Sentinel").await?;
        let queued = vec!["Registry preparation queued.".to_owned()];
        check_eq!(
            w.server
                .command(p.author, "@mind/rate Sentinel@1".into())
                .await?,
            queued
        );
        await_own(&w.server, p.author, |line| {
            line == "Registry work retained and queued."
        })
        .await?;
        check_eq!(
            w.server
                .command(p.author, "@mind/rate Sentinel@1".into())
                .await?,
            vec!["The Registry is already working on one of your revisions; wait until it says it is finished.".to_owned()]
        );
        check_eq!(
            w.server.command(p.author, "@registry/cancel".into()).await?,
            Vec::<String>::new(),
            "the cancel is accepted silently"
        );
        await_own(&w.server, p.author, |line| line == "Registry work finished.").await?;
        let end = Instant::now() + STEP;
        loop {
            let lines = w
                .server
                .command(p.author, "@mind/rate Sentinel@1".into())
                .await?;
            if lines == queued {
                break;
            }
            check!(Instant::now() < end, "the job was released: {lines:?}");
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })
    .await;
    w.server.stop().await?;
    let _ = std::fs::remove_dir_all(&w.data_dir);
    result?
}

/// 17g C2a test 13: a format-1 (House-only) Active launch composes no player
/// market, so no command slot: every C2a command but `@mind/rate` refuses
/// `NOT_OPEN` for the author, and `@mind/rate` gives the Registry service's
/// own result.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn author_commands_are_not_open_on_a_house_only_launch() -> TestResult {
    let mut w = world("mush-c2a-house-only", 1).await?;
    let result = timeout(STEP * 10, async {
        let p = w.launch.principals;
        saved(&w, p.author, "Sentinel").await?;
        for line in [
            "@mind/file Sentinel@1",
            "@mind/file Sentinel@1=Sentinel Line",
            "@mind/update 1 Sentinel@1",
            "@mind/withdraw 1",
            "@mind/attest 1",
            "@mind/fork 1=Sentinel Copy",
        ] {
            check_eq!(
                w.server.command(p.author, line.into()).await?,
                vec![NOT_OPEN.to_owned()],
                "{line}"
            );
        }
        check_eq!(
            w.server
                .command(p.author, "@mind/rate Sentinel@1".into())
                .await?,
            vec!["Registry preparation queued.".to_owned()]
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })
    .await;
    w.server.stop().await?;
    let _ = std::fs::remove_dir_all(&w.data_dir);
    result?
}

/// The comparison rule's own cases.
#[test]
fn comparison_rule_masks_only_the_typed_number() {
    let a = vec!["You own no filed mind 1.".to_owned()];
    let b = vec!["You own no filed mind 987654.".to_owned()];
    assert!(same_refusal(&a, "1", &b, "987654"));
    assert!(!same_refusal(
        &["You own no filed mind 12.".to_owned()],
        "1",
        &b,
        "987654"
    ));
    let known = Known {
        digests: vec!["abc".into()],
        accounts: vec![7],
    };
    assert!(assert_private(&["Filing a@1 as b.".to_owned()], &known).is_ok());
    assert!(assert_private(&["x abc y".to_owned()], &known).is_err());
    assert!(assert_private(&["account 7.".to_owned()], &known).is_err());
    assert!(assert_private(&["account 70.".to_owned()], &known).is_ok());
    assert!(assert_private(&["sells for 7cr".to_owned()], &known).is_ok());
    assert!(assert_private(&["a".repeat(64)], &Known::default()).is_err());
}
