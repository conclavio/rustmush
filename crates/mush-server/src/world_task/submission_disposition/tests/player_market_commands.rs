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
    auth_account, open_wallet,
    runtime::{Running, STEP, Seen, rows},
};
use super::player_market::{
    LAUNCH_AUTHOR, Launch, acknowledged, character, initialized_launch_with, launch_active, scratch,
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
/// The renewal services' one uniform refusal (`@mindrenewal` and
/// `@housemindrenewal`, their `front` switch among them).
const RENEWAL_UNAVAILABLE: &str = "Renewal is unavailable or has changed.";
/// 17g C2h: the renewal services' typed `front` forms (`{n}` the licence),
/// each step of both services and a malformed argument; the operation is a
/// well-formed token no offer carries.
const FRONT_FORMS: [&str; 6] = [
    "@mindrenewal/front {n} request",
    "@mindrenewal/front {n} accept 0123456789abcdef0123456789abcdef",
    "@mindrenewal/front {n} decline 0123456789abcdef0123456789abcdef",
    "@housemindrenewal/front {n} request",
    "@housemindrenewal/front {n} accept 0123456789abcdef0123456789abcdef",
    "@mindrenewal/front 0{n} request",
];

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

/// The market's replies (`tacsim::minds::market_commands::words`) the 17g
/// C2e tests pin through the real granted read.
const EMPTY: &str = "No minds are for sale at this counter.";
const COUNTER: &str = "Listing and buying minds happen at a market counter.";
const HOLD_HEAD: &str = "Your mind licences:";
/// 17g C2g: bare `+minds/holdings` with nothing in either section.
const HOLD_EMPTY: &str = "You hold no mind licences.";
/// 17g C2g: `/buy/pay` without exactly one personal credit account.
const ACCOUNT: &str = "This needs exactly one personal credit account in your name.";
fn no_quote(code: &str) -> String {
    format!("You have no quote on listing {code}. Ask for one with +minds/buy {code}.")
}
fn no_purchase(code: &str) -> String {
    format!("You have no paid purchase waiting on listing {code}.")
}
/// `TERMS` filled with the provisional counter's price (400cr) and the
/// personal term (90 IC days).
const TERMS_400: &str = "Listings here sell at 400cr for one seat and 90 IC days; per-listing terms are not available yet.";
fn no_listing(code: &str) -> String {
    format!("No listing here has the code {code}.")
}

/// 17g C2c test 7 under its binding name, real since 17g C2e (C2e plan §2.4
/// item 3, reviews FE-F1, FE-F4 and AD-F10). On the format-2 Active launch
/// the registry composes 33 owner-read services and 35 grants, so the
/// `+minds` read is the composed one, granted to TacSim, and not a fixture.
/// At the counter, `+minds` reads the real listing family through the
/// granted reader (`Lens::scan` over the live store, the venue gate through
/// the real `EntitlementGuard`) and answers `EMPTY`, byte-identical for the
/// author, the buyer and the bystander (two logins and a third); away from
/// the counter (Limbo) each reads `COUNTER`, alike. After a restart the same
/// boot composes the same counts and every principal reads the same lines
/// again. A listing to show needs a genuine Registry rating, so the rows
/// themselves are story part 2's (C2h).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn listings_are_identical_for_every_principal_at_the_counter() -> TestResult {
    let mut w = world("mush-c2e-listings", 2).await?;
    let result = timeout(STEP * 10, async {
        let p = w.launch.principals;
        let principals = [p.author, p.buyer, w.bystander];
        let venue = w.launch.deployment.venue.room;
        for restarted in [false, true] {
            check_eq!(
                w.server.owner_read_counts(),
                Some((33, 35)),
                "the composed reads (restarted: {restarted})"
            );
            let mut at_counter = Vec::new();
            for actor in principals {
                at_counter.push(w.server.command(actor, "+minds".into()).await?);
            }
            for lines in &at_counter {
                check_eq!(lines, &vec![EMPTY.to_owned()], "restarted: {restarted}");
            }
            let mut away = Vec::new();
            for actor in principals {
                w.server
                    .command(GOD, format!("@teleport #{}=#0", actor.0))
                    .await?;
                away.push(w.server.command(actor, "+minds".into()).await?);
                w.server
                    .command(GOD, format!("@teleport #{}=#{}", actor.0, venue.0))
                    .await?;
            }
            for lines in &away {
                check_eq!(lines, &vec![COUNTER.to_owned()], "restarted: {restarted}");
            }
            if !restarted {
                w.server.stop().await?;
                w.server = active(&w.launch, w.alt, w.bystander).await?;
            }
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })
    .await;
    w.server.stop().await?;
    let _ = std::fs::remove_dir_all(&w.data_dir);
    result?
}

/// 17g C2c test 8 under its binding name (the publish half 17g C2e's, C2e
/// plan §2.4 item 4; the `/buy` half 17g C2g's, §4.4), through the real
/// granted reads on the format-2 Active launch:
/// - `@mind/publish 1 form=sealed` away from the counter is `COUNTER`;
/// - at the counter, a `price=`, `seats=` or `term=` other than the
///   market's own is `TERMS` filled with the counter's price and term, and
///   a zero (`price=0`, `seats=0`, `term=0`) is the same `TERMS` (edge b);
/// - the buyer naming a filed number is `NO_FILED`, alike under the
///   comparison rule (`same_refusal`) for the first number Roll would
///   allocate and a far one (both missing here, as in C2a's test 9; the real
///   foreign filed number is story part 2's);
/// - `+minds feed0001` and `+minds/next feed0001` are `NO_LISTING`.
/// - 17g C2g, the `/buy` half (no own record exists in the fast suite: a
///   listing needs a genuine rating, story part 2's): at the counter the
///   buyer's `+minds/buy feed0001` is `NO_LISTING`; `+minds/buy/pay
///   feed0001` is `NO_QUOTE` (the market read's own-purchase kind over the
///   buyer's own index, which holds nothing); `/buy/complete` and
///   `/buy/refund` are `NO_PURCHASE`; and none prints a request line;
/// - `/buy/complete` by another character (the author) on nothing reads
///   byte-identically under `same_refusal`, and the author's `/buy/refund`
///   of another code alike;
/// - `/buy/pay` by a character with two personal credit accounts (the
///   author's alt, given two by GOD) is `ACCOUNT`, before any own-record
///   read;
/// - away from the counter (Limbo), `+minds/buy`, `/buy/pay` and
///   `/buy/complete` are `COUNTER` and `/buy/refund` (anywhere) is
///   `NO_PURCHASE`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn market_commands_refuse_without_own_records() -> TestResult {
    let mut w = world("mush-c2e-refusals", 2).await?;
    let result = timeout(STEP * 10, async {
        let p = w.launch.principals;
        check_eq!(
            w.launch
                .deployment
                .market
                .as_ref()
                .map(|market| market.terms.price_minor),
            Some(40_000),
            "TERMS_400 names the provisional price"
        );
        w.server
            .command(GOD, format!("@teleport #{}=#0", p.buyer.0))
            .await?;
        check_eq!(
            w.server
                .command(p.buyer, "@mind/publish 1 form=sealed".into())
                .await?,
            vec![COUNTER.to_owned()],
            "away from the counter"
        );
        // 17g C2g (the C2g fix round's tests/T4): the buy steps' counter
        // rule, away from the counter: `/buy`, `/pay` and `/complete` are
        // `COUNTER` before any read; `/refund` runs anywhere, so it reaches
        // the own-purchase read and is `NO_PURCHASE`.
        for (line, reply) in [
            ("+minds/buy feed0001", COUNTER.to_owned()),
            ("+minds/buy/pay feed0001", COUNTER.to_owned()),
            ("+minds/buy/complete feed0001", COUNTER.to_owned()),
            ("+minds/buy/refund feed0001", no_purchase("feed0001")),
        ] {
            check_eq!(
                w.server.command(p.buyer, line.into()).await?,
                vec![reply],
                "away from the counter: {line}"
            );
        }
        w.server
            .command(
                GOD,
                format!(
                    "@teleport #{}=#{}",
                    p.buyer.0, w.launch.deployment.venue.room.0
                ),
            )
            .await?;
        for line in [
            "@mind/publish 1 form=sealed price=1",
            "@mind/publish 1 form=sealed price=0",
            "@mind/publish 1 form=sealed seats=0",
            "@mind/publish 1 form=sealed term=0",
            "@mind/publish 1 form=open seats=2",
            "@mind/publish 1 form=open term=30",
        ] {
            check_eq!(
                w.server.command(p.buyer, line.into()).await?,
                vec![TERMS_400.to_owned()],
                "{line}"
            );
        }
        let mut replies = Vec::new();
        for typed in ["1", "987654"] {
            let line = format!("@mind/publish {typed} form=sealed");
            let lines = w.server.command(p.buyer, line.clone()).await?;
            check_eq!(
                lines,
                vec![format!("You own no filed mind {typed}.")],
                "{line}"
            );
            replies.push((lines, typed));
        }
        let [(a, typed_a), (b, typed_b)] =
            <[_; 2]>::try_from(replies).map_err(|_| "two replies")?;
        check!(
            same_refusal(&a, typed_a, &b, typed_b),
            "NO_FILED reads alike: {a:?} vs {b:?}"
        );
        for line in ["+minds feed0001", "+minds/next feed0001"] {
            check_eq!(
                w.server.command(p.buyer, line.into()).await?,
                vec![no_listing("feed0001")],
                "{line}"
            );
        }
        // 17g C2g: the `/buy` half.
        for (line, reply) in [
            ("+minds/buy feed0001", no_listing("feed0001")),
            ("+minds/buy/pay feed0001", no_quote("feed0001")),
            ("+minds/buy/complete feed0001", no_purchase("feed0001")),
            ("+minds/buy/refund feed0001", no_purchase("feed0001")),
        ] {
            check_eq!(
                w.server.command(p.buyer, line.into()).await?,
                vec![reply],
                "{line}"
            );
        }
        let buyer = w
            .server
            .command(p.buyer, "+minds/buy/complete feed0001".into())
            .await?;
        let author = w
            .server
            .command(p.author, "+minds/buy/complete feed0001".into())
            .await?;
        check_eq!(
            &buyer,
            &author,
            "another character on nothing: the same bytes"
        );
        let other_code = w
            .server
            .command(p.author, "+minds/buy/refund 0fffffff".into())
            .await?;
        check_eq!(&other_code, &vec![no_purchase("0fffffff")]);
        check!(
            same_refusal(
                &w.server
                    .command(p.buyer, "+minds/buy/refund feed0001".into())
                    .await?,
                "feed0001",
                &other_code,
                "0fffffff"
            ),
            "NO_PURCHASE reads alike for any code"
        );
        for _ in 0..2 {
            open_wallet(
                &w.server,
                &w.launch.store,
                mush_org::PrincipalId::Object(w.alt),
            )
            .await?;
        }
        check_eq!(
            w.server
                .command(w.alt, "+minds/buy/pay feed0001".into())
                .await?,
            vec![ACCOUNT.to_owned()],
            "two personal accounts"
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })
    .await;
    w.server.stop().await?;
    let _ = std::fs::remove_dir_all(&w.data_dir);
    result?
}

/// M8c B3: `org=<org>` on the buy steps, the renewal steps, `+minds/holdings`
/// and `@mind/attest` is parsed before any read (a bare `org=` is the form's
/// usage), then resolved: an unknown organisation and one the actor is not
/// granted `roll.holder.org` for read alike, `NOT_FOR_ORG`; granted on the
/// Guild's whole scope tree, every form reads the organisation's own
/// nothing with the character's own words (the fast suite has no listing,
/// no licence and no filed mind: the organisation's buy, holding and House
/// attestation are the composition tests'), another character stays refused,
/// and the grant changes nothing without `org=`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn org_forms_refuse_without_the_holder_grant() -> TestResult {
    const NOT_FOR_ORG: &str = "You do not act for that organisation.";
    const USAGE_FILED: &str = "@mind/update <filed> <name>@<rev>; @mind/withdraw <filed>; @mind/attest <filed> [org=<org>]";
    let mut w = world("mush-m8c-org-forms", 2).await?;
    let result = timeout(STEP * 10, async {
        let p = w.launch.principals;
        w.server
            .command(
                GOD,
                format!(
                    "@teleport #{}=#{}",
                    p.buyer.0, w.launch.deployment.venue.room.0
                ),
            )
            .await?;
        for (line, reply) in [
            ("+minds/buy feed0001 org=", MINDS_HELP),
            ("+minds/buy/pay feed0001 org=", MINDS_HELP),
            ("+minds/holdings org=", MINDS_HELP),
            ("+minds/holdings owner 3 org=", MINDS_HELP),
            ("+minds/renew 1 org=", MINDS_HELP),
            ("@mind/attest 1 org=", USAGE_FILED),
        ] {
            check_eq!(
                w.server.command(p.buyer, line.into()).await?,
                vec![reply.to_owned()],
                "a bare org=: {line}"
            );
        }
        acknowledged(&w.server, "@org/create Guild".into(), "Created org #").await?;
        let forms = [
            "+minds/buy feed0001 org=Guild",
            "+minds/buy/pay feed0001 org=Guild",
            "+minds/buy/complete feed0001 org=Guild",
            "+minds/buy/refund feed0001 org=Guild",
            "+minds/holdings org=Guild",
            "+minds/holdings paid 7 org=Guild",
            "+minds/renew 1 org=Guild",
            "+minds/renew/pay 1 org=Guild",
            "+minds/renew/decline 1 org=Guild",
            "@mind/attest 1 org=Guild",
        ];
        for line in forms {
            check_eq!(
                w.server.command(p.buyer, line.into()).await?,
                vec![NOT_FOR_ORG.to_owned()],
                "ungranted: {line}"
            );
            let unknown = line.replace("org=Guild", "org=Nowhere");
            check_eq!(
                w.server.command(p.buyer, unknown.clone()).await?,
                vec![NOT_FOR_ORG.to_owned()],
                "unknown organisation: {unknown}"
            );
        }
        acknowledged(
            &w.server,
            format!("@org/grant #{}=Guild roll.holder.org", p.buyer.0),
            "Granted",
        )
        .await?;
        for (line, reply) in [
            ("+minds/buy feed0001 org=Guild", no_listing("feed0001")),
            ("+minds/buy/pay feed0001 org=Guild", no_quote("feed0001")),
            (
                "+minds/buy/complete feed0001 org=Guild",
                no_purchase("feed0001"),
            ),
            (
                "+minds/buy/refund feed0001 org=Guild",
                no_purchase("feed0001"),
            ),
            ("+minds/holdings org=Guild", HOLD_EMPTY.to_owned()),
            (
                "+minds/renew 1 org=Guild",
                "You hold no licence 1.".to_owned(),
            ),
            (
                "+minds/renew/pay 1 org=Guild",
                "You hold no licence 1.".to_owned(),
            ),
            (
                "@mind/attest 1 org=Guild",
                "You own no filed mind 1.".to_owned(),
            ),
        ] {
            check_eq!(
                w.server.command(p.buyer, line.into()).await?,
                vec![reply],
                "granted, nothing held: {line}"
            );
        }
        check_eq!(
            w.server
                .command(p.author, "+minds/holdings org=Guild".into())
                .await?,
            vec![NOT_FOR_ORG.to_owned()],
            "another character stays refused"
        );
        check_eq!(
            w.server.command(p.buyer, "+minds/holdings".into()).await?,
            vec![HOLD_EMPTY.to_owned()],
            "the grant changes nothing without org="
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })
    .await;
    w.server.stop().await?;
    let _ = std::fs::remove_dir_all(&w.data_dir);
    result?
}

/// 17g C2c test 10: a format-1 (House-only) Active launch composes no player
/// market and so no command slot: every `+minds` form (bare, a code, `/next`,
/// `/holdings` and its sections, `/buy` and its three steps) and
/// `@mind/publish` refuse `NOT_OPEN` for the author, the buyer and the
/// bystander, byte for byte, and `+minds` itself is registered (an unknown
/// switch set answers its help, not "Huh?").
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn market_commands_are_not_open_on_a_house_only_launch() -> TestResult {
    let mut w = world("mush-c2c-house-only", 1).await?;
    let result = timeout(STEP * 10, async {
        let p = w.launch.principals;
        for actor in [p.author, p.buyer, w.bystander] {
            for line in [
                "+minds",
                "+minds feed0001",
                "+minds/next feed0001",
                "+minds/holdings",
                "+minds/holdings owner 1",
                "+minds/holdings paid 1",
                "+minds/buy feed0001",
                "+minds/buy/pay feed0001",
                "+minds/buy/complete feed0001",
                "+minds/buy/refund feed0001",
                "@mind/publish 1 form=sealed",
                "@mind/publish 1 form=open royalty=100 transfer=no",
            ] {
                check_eq!(
                    w.server.command(actor, line.into()).await?,
                    vec![NOT_OPEN.to_owned()],
                    "#{} {line}",
                    actor.0
                );
            }
            check_eq!(
                w.server.command(actor, "+minds/next/pay x".into()).await?,
                vec![MINDS_HELP.to_owned()],
                "#{} an unknown switch set",
                actor.0
            );
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })
    .await;
    w.server.stop().await?;
    let _ = std::fs::remove_dir_all(&w.data_dir);
    result?
}
/// 17g C2c test 9 (C2 plan §4; the C2c fix round, reviewer T-F6): on the
/// production composition (format 2, Active), every `+minds` form and
/// `@mind/publish` answers its actor alone. Through `command_seen`, for the
/// author, the buyer and the bystander, no line of theirs reaches any other
/// principal in the counter room (the alt, the bystander, the buyer, the
/// author), and every line passes `assert_private` against every account id
/// in the store; room speech is the positive control.
///
/// The lines, re-blessed in 17g C2e (C2e plan §2.5, review FE-F4) now the
/// launch grants TacSim the market read; the observer and privacy assertions
/// stand as they were. Every principal stands at the counter: `+minds` is
/// `EMPTY`; `+minds feed0001` and `+minds/next feed0001` are `NO_LISTING`;
/// `+minds/holdings owner 1` is `HOLD_HEAD` alone (the live Roll read, for a
/// character with no charters: no row and no `HOLD_MORE`); both
/// `@mind/publish` forms are `NO_FILED` (the counter admits; nobody owns
/// filed mind 1); a malformed form is `BAD_FORM` and an unknown switch set
/// the `+minds` help.
///
/// 17g C2g (C2e plan §4.4) re-blesses the forms it opens, each printing its
/// refusal and never a request line with no own records: bare
/// `+minds/holdings` is `HOLD_EMPTY` alone (nothing in either section, no
/// waiting purchase); `+minds/holdings paid 1` is `HOLD_HEAD` alone (the
/// live paid section past an empty prefix: no row and no `HOLD_MORE`);
/// `+minds/buy feed0001` is `NO_LISTING`; `+minds/buy/pay feed0001` is
/// `NO_QUOTE` for the author and the buyer and `ACCOUNT` for the bystander,
/// who has no personal credit account; `/buy/complete` and `/buy/refund` are
/// `NO_PURCHASE`.
///
/// The refusals 17g C2e made reachable are under the same lens (the C2e fix
/// round, reviewer tests/T3): `@mind/publish` with a zero price (edge b) or
/// a seat count other than the market's is `TERMS` at the counter; and each
/// actor, taken to Limbo, reads `COUNTER` for bare `+minds` and for
/// `@mind/publish`, reaching nobody else and passing `assert_private`,
/// before it is taken back to the counter.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn market_replies_reach_only_the_actor() -> TestResult {
    let mut w = world("mush-c2c-observer", 2).await?;
    let result = timeout(STEP * 10, async {
        let p = w.launch.principals;
        let mut accounts = Vec::new();
        for (key, row) in w.launch.store.module_json_list("economy").await? {
            if key.starts_with("account:")
                && let Some(id) = row.get("id").and_then(Value::as_u64)
            {
                accounts.push(id);
            }
        }
        let known = Known {
            digests: Vec::new(),
            accounts,
        };
        let room = [
            (p.author, "author"),
            (p.buyer, "buyer"),
            (w.alt, "alt"),
            (w.bystander, "bystander"),
        ];
        let said = w
            .server
            .command_seen(p.author, "say Testing the counter room.".into())
            .await?;
        for (observer, name) in room.iter().filter(|(o, _)| *o != p.author) {
            check!(
                said.reaching(*observer)
                    .iter()
                    .any(|line| line.contains("Testing the counter room.")),
                "the {name} hears the room: {said:?}"
            );
        }
        let no_listing = no_listing("feed0001");
        let no_quote = no_quote("feed0001");
        let no_purchase = no_purchase("feed0001");
        let lines = [
            ("+minds", EMPTY),
            ("+minds feed0001", no_listing.as_str()),
            ("+minds/next feed0001", no_listing.as_str()),
            ("+minds/holdings", HOLD_EMPTY),
            ("+minds/holdings owner 1", HOLD_HEAD),
            ("+minds/holdings paid 1", HOLD_HEAD),
            ("+minds/buy feed0001", no_listing.as_str()),
            ("+minds/buy/pay feed0001", no_quote.as_str()),
            ("+minds/buy/complete feed0001", no_purchase.as_str()),
            ("+minds/buy/refund feed0001", no_purchase.as_str()),
            ("+minds/next/pay x", MINDS_HELP),
            ("@mind/publish 1 form=sealed", "You own no filed mind 1."),
            (
                "@mind/publish 1 form=open royalty=100 transfer=no",
                "You own no filed mind 1.",
            ),
            ("@mind/publish 1 form=sealed price=0", TERMS_400),
            ("@mind/publish 1 form=sealed seats=2", TERMS_400),
            (
                "@mind/publish 1 form=bogus",
                "form is sealed, open or open-resale.",
            ),
        ];
        let quiet = |seen: &Seen, actor: ObjId, line: &str| -> TestResult {
            for (observer, name) in room.iter().filter(|(o, _)| *o != actor) {
                let reached = seen.reaching(*observer);
                check!(
                    reached.is_empty(),
                    "#{} {line} reached the {name}: {reached:?}",
                    actor.0
                );
            }
            assert_private(&seen.own, &known)
        };
        for actor in [p.author, p.buyer, w.bystander] {
            for (line, reply) in lines {
                // The settle window: whatever arrived since the last check.
                let late = w.server.take_seen(actor)?;
                quiet(&late, actor, "the settle window")?;
                let seen = w.server.command_seen(actor, line.into()).await?;
                quiet(&seen, actor, line)?;
                // The bystander has no personal credit account (17g C2g).
                let reply = if actor == w.bystander && line == "+minds/buy/pay feed0001" {
                    ACCOUNT
                } else {
                    reply
                };
                check_eq!(seen.own, vec![reply.to_owned()], "#{} {line}", actor.0);
            }
        }
        // Away from the counter: the teleports' own room lines are drained
        // by `command`, so only the actor's two commands are under the lens.
        let venue = w.launch.deployment.venue.room;
        for actor in [p.author, p.buyer, w.bystander] {
            w.server
                .command(GOD, format!("@teleport #{}=#0", actor.0))
                .await?;
            for line in ["+minds", "@mind/publish 1 form=sealed"] {
                let late = w.server.take_seen(actor)?;
                quiet(&late, actor, "the settle window")?;
                let seen = w.server.command_seen(actor, line.into()).await?;
                quiet(&seen, actor, line)?;
                check_eq!(
                    seen.own,
                    vec![COUNTER.to_owned()],
                    "#{} {line} in Limbo",
                    actor.0
                );
            }
            w.server
                .command(GOD, format!("@teleport #{}=#{}", actor.0, venue.0))
                .await?;
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })
    .await;
    w.server.stop().await?;
    let _ = std::fs::remove_dir_all(&w.data_dir);
    result?
}

/// The account ids the store holds (the `assert_private` lens).
pub(super) async fn known_accounts(w: &World) -> TestResult<Known> {
    let mut accounts = Vec::new();
    for (key, row) in w.launch.store.module_json_list("economy").await? {
        if key.starts_with("account:")
            && let Some(id) = row.get("id").and_then(Value::as_u64)
        {
            accounts.push(id);
        }
    }
    Ok(Known {
        digests: Vec::new(),
        accounts,
    })
}

/// 17g C2d test 10 (C2 plan §2.2 departure 8, §2.8): `@registry/recall` and
/// `@registry/revoke` make no operator pre-check, so a number nobody holds
/// reads the same bytes for the buyer, God and the deployment's granted
/// operator: the request line, the engine's `Queued.` and the driver's one
/// line (the buyer and God are not offered the action; the operator is, and
/// its provider refuses the missing target). The buyer's recall of the first
/// filed number Roll would allocate reads like its recall of `999` under
/// `same_refusal`. The fast suite has no filing (a filing needs a genuine
/// rating), so that number is missing too; the buyer's recall of a REAL
/// filed number, and the operator's real revocation and recall, are the
/// story's (`player_market_story`, part 3, which waits with part 2).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn registry_commands_refuse_alike_for_every_principal_on_unknown_targets() -> TestResult {
    let mut w = world("mush-c2d-registry", 2).await?;
    let result = timeout(STEP * 10, async {
        let p = w.launch.principals;
        let known = known_accounts(&w).await?;
        for (template, request) in [
            ("@registry/recall {n}", "Recalling filed revision {n}."),
            ("@registry/revoke {n}", "Revoking licence {n}."),
        ] {
            let mut replies = Vec::new();
            for (actor, name) in [(p.buyer, "buyer"), (p.god, "god"), (p.operator, "operator")] {
                let line = template.replace("{n}", "999");
                let lines = w.server.command(actor, line.clone()).await?;
                let lines = until_own(&w.server, actor, lines, |l| l == DRIVER).await?;
                assert_private(&lines, &known)?;
                check_eq!(
                    lines,
                    vec![
                        request.replace("{n}", "999"),
                        QUEUED.to_owned(),
                        DRIVER.to_owned()
                    ],
                    "{name}: {line}"
                );
                replies.push(lines);
            }
            check!(
                replies.windows(2).all(|pair| pair.first() == pair.get(1)),
                "{template}: every principal reads the same bytes"
            );
        }
        // The buyer: another number reads like 999.
        let mut replies = Vec::new();
        for typed in ["1", "999"] {
            let line = format!("@registry/recall {typed}");
            let lines = w.server.command(p.buyer, line).await?;
            replies.push((
                until_own(&w.server, p.buyer, lines, |l| l == DRIVER).await?,
                typed,
            ));
        }
        let [(a, typed_a), (b, typed_b)] =
            <[_; 2]>::try_from(replies).map_err(|_| "two replies")?;
        check!(
            same_refusal(&a, typed_a, &b, typed_b),
            "the buyer's recall reads alike: {a:?} vs {b:?}"
        );
        // A malformed number is the help, for anyone.
        check_eq!(
            w.server
                .command(p.buyer, "@registry/recall x".into())
                .await?,
            vec![REGISTRY_HELP.to_owned()]
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })
    .await;
    w.server.stop().await?;
    let _ = std::fs::remove_dir_all(&w.data_dir);
    result?
}

/// 17g C2d test 11 (C2 plan §4): the trade commands refuse without the
/// actor's own records, each with its own sentence, alike for a missing and
/// a foreign subject:
/// - `+minds/renew 999` reads `NO_LICENCE`, as do `/renew/pay` and
///   `/renew/decline`, and the first licence number Roll would allocate
///   reads like `999` (`same_refusal`), for the buyer and the author;
/// - `+minds/charter` of a filed mind the actor does not own is `NO_FILED`,
///   `+minds/resell` of a licence the actor does not hold `NO_LICENCE`;
/// - a missing player is `NO_PLAYER`, checked before the actor's own record
///   (a player's existence is public);
/// - `price=0` is `BAD_PRICE`, before anything is read;
/// - 17g C2g: the two accepts read the offers naming the actor through the
///   per-buyer offer indexes (the market's `ResaleOffers`, Roll's
///   `CharterOffers`), so with none `+minds/resell/accept 999` is
///   `NO_OFFER` and `+minds/charter/accept 999` `NO_CHARTER_OFFER`, with or
///   without `price=` (a filed number that does not exist reads as one with
///   no offer); a malformed accept is still the help.
///
/// The fast suite has no filing or licence (a genuine rating), so "foreign"
/// is the story's; here both numbers are missing, as in C2a's test 9.
///
/// 17g C2h: `+minds/renew` is live (the lead's ruling R3: it hands the step
/// to the renewal service's delegated `front`), and `NO_LICENCE` is still its
/// first refusal, before anything is handed over: `+minds/renew 999`,
/// `/renew/pay 1` and `/renew/decline 1` (and the other number of each) read
/// `NO_LICENCE` for EVERY principal (God, the author and the alt, the buyer,
/// the clerk, the operator, the accountless character and the bystander),
/// alike for the two numbers, and no later line arrives (nothing was
/// scheduled). The services' own `front` switch, which any player can type
/// (a spelling of the same step for the same actor, no connection guard, the
/// C2h brief), refuses a licence the actor does not hold with the service's
/// one uniform line, `Renewal is unavailable or has changed.`, for every
/// principal and both services, alike for the two numbers, and so does a
/// malformed argument (a leading zero); nothing later arrives either. The
/// dispatched renewal flow is the story's part 3.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trade_commands_refuse_without_own_records() -> TestResult {
    let mut w = world("mush-c2d-refusals", 2).await?;
    let result = timeout(STEP * 10, async {
        let p = w.launch.principals;
        for (actor, name) in [(p.buyer, "buyer"), (p.author, "author")] {
            for (template, reply) in [
                ("+minds/renew {n}", "You hold no licence {n}."),
                ("+minds/renew/pay {n}", "You hold no licence {n}."),
                ("+minds/renew/decline {n}", "You hold no licence {n}."),
                (
                    "+minds/resell {n}=LaunchAuthor price=5",
                    "You hold no licence {n}.",
                ),
                (
                    "+minds/charter {n}=LaunchAuthor price=1",
                    "You own no filed mind {n}.",
                ),
            ] {
                let mut replies = Vec::new();
                for typed in ["1", "999"] {
                    let line = template.replace("{n}", typed);
                    let lines = w.server.command(actor, line.clone()).await?;
                    check_eq!(lines, vec![reply.replace("{n}", typed)], "{name}: {line}");
                    replies.push((lines, typed));
                }
                let [(a, typed_a), (b, typed_b)] =
                    <[_; 2]>::try_from(replies).map_err(|_| "two replies")?;
                check!(
                    same_refusal(&a, typed_a, &b, typed_b),
                    "{name}: {template} reads alike"
                );
            }
            for (line, reply) in [
                (
                    "+minds/resell 999=Nobody price=5",
                    "No player is named Nobody.",
                ),
                (
                    "+minds/charter 999=#999999 price=1",
                    "No player is named #999999.",
                ),
                (
                    "+minds/resell 999=LaunchAuthor price=0",
                    "price is in credits, from 0.01 to 1000000, such as price=12.50.",
                ),
                (
                    "+minds/charter 999=LaunchAuthor price=1.234",
                    "price is in credits, from 0.01 to 1000000, such as price=12.50.",
                ),
                ("+minds/resell/accept 999", NO_OFFER_999),
                ("+minds/resell/accept 999 price=0.21", NO_OFFER_999),
                ("+minds/charter/accept 999", NO_CHARTER_OFFER_999),
                ("+minds/charter/accept 999 price=1", NO_CHARTER_OFFER_999),
            ] {
                check_eq!(
                    w.server.command(actor, line.into()).await?,
                    vec![reply.to_owned()],
                    "{name}: {line}"
                );
            }
            check_eq!(
                w.server
                    .command(actor, "+minds/resell/accept 999 cost=1".into())
                    .await?,
                vec![MINDS_HELP.to_owned()],
                "{name}: a malformed accept"
            );
        }
        // 17g C2h: every principal, alike.
        let everyone = p
            .all()
            .into_iter()
            .chain([("alt", w.alt), ("bystander", w.bystander)]);
        for (name, actor) in everyone {
            for template in [
                "+minds/renew {n}",
                "+minds/renew/pay {n}",
                "+minds/renew/decline {n}",
            ] {
                let mut replies = Vec::new();
                for typed in ["1", "999"] {
                    let line = template.replace("{n}", typed);
                    let lines = w.server.command(actor, line.clone()).await?;
                    check_eq!(
                        lines,
                        vec![format!("You hold no licence {typed}.")],
                        "{name}: {line}"
                    );
                    replies.push((lines, typed));
                }
                let [(a, typed_a), (b, typed_b)] =
                    <[_; 2]>::try_from(replies).map_err(|_| "two replies")?;
                check!(
                    same_refusal(&a, typed_a, &b, typed_b),
                    "{name}: {template} reads alike"
                );
            }
            // 17g C2h: the typed front, refused before any step.
            for template in FRONT_FORMS {
                let mut replies = Vec::new();
                for typed in ["1", "999"] {
                    let line = template.replace("{n}", typed);
                    let lines = w.server.command(actor, line.clone()).await?;
                    check_eq!(
                        lines,
                        vec![RENEWAL_UNAVAILABLE.to_owned()],
                        "{name}: {line}"
                    );
                    replies.push((lines, typed));
                }
                let [(a, typed_a), (b, typed_b)] =
                    <[_; 2]>::try_from(replies).map_err(|_| "two replies")?;
                check!(
                    same_refusal(&a, typed_a, &b, typed_b),
                    "{name}: {template} reads alike"
                );
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
            let late = w.server.take_seen(actor)?;
            check!(
                late.own.is_empty(),
                "{name}: nothing was handed over: {late:?}"
            );
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })
    .await;
    w.server.stop().await?;
    let _ = std::fs::remove_dir_all(&w.data_dir);
    result?
}

/// 17g C2d test 12 (C2 plan §2.8, the observer rule): every C2d command's
/// reply, the driver's later line included, reaches its actor alone: for the
/// author, the buyer and the bystander, through `command_seen`, no line
/// reaches any other principal in the counter room, and every line passes
/// `assert_private` against every account id the store holds. The seller's
/// chosen buyer may be named only on `RESELLING` and `CHARTERING` (the
/// buyer's name is then exempt from the check below); no other line names
/// another character. Room speech is the positive control.
///
/// 17g C2g extends the table to the forms it opens: the two accepts now
/// read the per-buyer offer indexes (`NO_OFFER`, `NO_CHARTER_OFFER`), and a
/// seller form naming a licence the actor does not hold (`+minds/resell
/// 1=LaunchAlt price=1`) is `NO_LICENCE`, exactly, under the same lens.
///
/// 17g C2h: the three renewal forms, live through the renewal services'
/// delegated front, read exactly `NO_LICENCE` for each actor here, and
/// nothing reaches anyone else (the settle window before the next line
/// would catch a late line from a handed-over step); the services' typed
/// `front` forms read exactly the uniform refusal, to the actor alone.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trade_replies_reach_only_the_actor() -> TestResult {
    let mut w = world("mush-c2d-observer", 2).await?;
    let result = timeout(STEP * 10, async {
        let p = w.launch.principals;
        let known = known_accounts(&w).await?;
        let room = [
            (p.author, "author"),
            (p.buyer, "buyer"),
            (w.alt, "alt"),
            (w.bystander, "bystander"),
        ];
        let said = w
            .server
            .command_seen(p.author, "say Testing the counter room.".into())
            .await?;
        for (observer, name) in room.iter().filter(|(o, _)| *o != p.author) {
            check!(
                said.reaching(*observer)
                    .iter()
                    .any(|line| line.contains("Testing the counter room.")),
                "the {name} hears the room: {said:?}"
            );
        }
        let quiet = |seen: &Seen, actor: ObjId, line: &str| -> TestResult {
            for (observer, name) in room.iter().filter(|(o, _)| *o != actor) {
                let reached = seen.reaching(*observer);
                check!(
                    reached.is_empty(),
                    "#{} {line} reached the {name}: {reached:?}",
                    actor.0
                );
            }
            assert_private(&seen.own, &known)?;
            for own in &seen.own {
                // The seller's chosen buyer (every seller form below names
                // `LaunchAlt`) is exempt on `RESELLING` and `CHARTERING`
                // only; every other name is checked on every line (the C2d
                // fix round, reviewer T5).
                let seller_line =
                    own.starts_with("Offering licence ") || own.starts_with("Offering filed mind ");
                for other in [
                    "LaunchAuthor",
                    "LaunchBuyer",
                    "LaunchAlt",
                    "LaunchBystander",
                ] {
                    if seller_line && other == "LaunchAlt" {
                        continue;
                    }
                    check!(!own.contains(other), "{line} names {other}: {own:?}");
                }
            }
            Ok(())
        };
        let lines = [
            ("+minds/renew 999", false),
            ("+minds/renew/pay 999", false),
            ("+minds/renew/decline 999", false),
            ("+minds/resell 999=LaunchAlt price=5", false),
            ("+minds/resell/accept 999", false),
            ("+minds/resell/accept 999 price=0.21", false),
            ("+minds/charter 999=LaunchAlt price=1", false),
            ("+minds/charter/accept 999 price=1", false),
            ("+minds/renew/resell x", false),
            ("+minds/resell 1=LaunchAlt price=1", false),
            ("@registry/recall 999", true),
            ("@registry/revoke 999", true),
            ("@mindrenewal/front 999 request", false),
            (
                "@mindrenewal/front 999 accept 0123456789abcdef0123456789abcdef",
                false,
            ),
            ("@housemindrenewal/front 999 request", false),
            (
                "@housemindrenewal/front 999 decline 0123456789abcdef0123456789abcdef",
                false,
            ),
        ];
        for actor in [p.author, p.buyer, w.bystander] {
            for (line, submits) in lines {
                // The settle window: whatever arrived since the last check.
                let late = w.server.take_seen(actor)?;
                quiet(&late, actor, "the settle window")?;
                let mut seen = w.server.command_seen(actor, line.into()).await?;
                if submits && !seen.own.iter().any(|l| l == DRIVER) {
                    seen.extend(await_own(&w.server, actor, |l| l == DRIVER).await?);
                }
                check!(!seen.own.is_empty(), "#{} {line} answers", actor.0);
                quiet(&seen, actor, line)?;
                // 17g C2g: the live lines, exactly.
                let exact = match line {
                    "+minds/resell/accept 999" | "+minds/resell/accept 999 price=0.21" => {
                        Some(NO_OFFER_999)
                    }
                    "+minds/charter/accept 999 price=1" => Some(NO_CHARTER_OFFER_999),
                    "+minds/resell 1=LaunchAlt price=1" => Some("You hold no licence 1."),
                    // 17g C2h: live, and refused before any hand-off.
                    "+minds/renew 999" | "+minds/renew/pay 999" | "+minds/renew/decline 999" => {
                        Some("You hold no licence 999.")
                    }
                    _ if line.contains("renewal/front ") => Some(RENEWAL_UNAVAILABLE),
                    _ => None,
                };
                if let Some(exact) = exact {
                    check_eq!(seen.own, vec![exact.to_owned()], "#{} {line}", actor.0);
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

/// 17g C2d test 13: a format-1 (House-only) Active launch composes no player
/// market and so no command slot: every C2d `+minds` form refuses
/// `NOT_OPEN`, and `@registry/recall` and `@registry/revoke` (the Registry is
/// composed there) refuse `REGISTRY_OFF`, for the author, the buyer and God,
/// byte for byte. House Pin renewals stay on `@housemindrenewal` there.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trade_commands_are_not_open_on_a_house_only_launch() -> TestResult {
    let mut w = world("mush-c2d-house-only", 1).await?;
    let result = timeout(STEP * 10, async {
        let p = w.launch.principals;
        for actor in [p.author, p.buyer, p.god] {
            for line in [
                "+minds/renew 1",
                "+minds/renew/pay 1",
                "+minds/renew/decline 1",
                "+minds/resell 1=LaunchBuyer price=5",
                "+minds/resell/accept 1",
                "+minds/resell/accept 1 price=5",
                "+minds/charter 1=LaunchBuyer price=1",
                "+minds/charter/accept 1 price=1",
            ] {
                check_eq!(
                    w.server.command(actor, line.into()).await?,
                    vec![NOT_OPEN.to_owned()],
                    "#{} {line}",
                    actor.0
                );
            }
            for line in ["@registry/recall 1", "@registry/revoke 1"] {
                check_eq!(
                    w.server.command(actor, line.into()).await?,
                    vec![REGISTRY_OFF.to_owned()],
                    "#{} {line}",
                    actor.0
                );
            }
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })
    .await;
    w.server.stop().await?;
    let _ = std::fs::remove_dir_all(&w.data_dir);
    result?
}

/// 17g C2g: the accepts with no offer naming the actor.
const NO_OFFER_999: &str = "You have no offer for licence 999.";
const NO_CHARTER_OFFER_999: &str = "You have no offer for filed mind 999.";

/// `@registry`'s help (17g C2d).
const REGISTRY_HELP: &str = "@registry/rate <owned mind number>/<saved revision>; @registry/cancel\n@registry/recall <filed> and @registry/revoke <licence> (Registry operators)";
/// The Registry commands' refusal without the player market.
const REGISTRY_OFF: &str = "Registry recall and revocation are not configured here.";

/// `+minds`'s help, its reply to any other switch set (17g C2d: its last
/// three lines, the renewal and trade forms).
const MINDS_HELP: &str = "+minds [<code>]; +minds/next <code>; +minds/holdings [owner|paid <cursor>] [org=<org>]\n+minds/buy <code>; +minds/buy/pay <code>; +minds/buy/complete <code>; +minds/buy/refund <code> (each [org=<org>])\n+minds/renew <licence> [org=<org>] (then /pay or /decline with the same licence)\n+minds/resell <licence>=<player> price=<credits>; +minds/resell/accept <licence> [price=<credits>]\n+minds/charter <filed>=<player> price=<credits>; +minds/charter/accept <filed> [price=<credits>]";

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
