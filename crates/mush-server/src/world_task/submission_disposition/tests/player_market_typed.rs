//! 17g C3a (the C3 plan §4): the typed market surfaces over C1's launch
//! harness, through `boot::game_modules` and the production launch document.
//! Every typed request is an `Inbound::Invoke` (`MindRequest::Market`) or an
//! `Inbound::Query` (`mind.market`, `mind.holdings`) on the connection the
//! harness attached for the character, exactly as the gateway forwards them
//! (`Running::invoke`, `Running::query`), beside C2's `Inbound::Cmd` lines.
//!
//! The four focused tests need no listing, so they fit the focused gate; the
//! positive money path (a typed publish, quote, pay and complete, the panels
//! against the line, a typed resale offer and accept, the withdrawal and a
//! restart) needs one genuine Registry rating, so it is the `#[ignore]`d
//! story at the end, named verbatim in `.github/workflows/ci.yml`'s
//! `mind-commerce-acceptance` matrix ("typed market surfaces"), which the
//! `ci_gated_test_inventory` contract requires of an ignored lib test. C2's
//! own story (`player_market_story`) is not touched.
//!
//! The comparison rule is C2's (`same_refusal`, `assert_private`): a typed
//! refusal is the line's refusal byte for byte, its `code` the word's
//! constant name in lower case, and a foreign number reads exactly as a
//! missing one, in `code` and in `message`. The focused tests can only send
//! missing numbers (no filing or licence exists without a genuine rating);
//! the story sends each number-taking action on another principal's real
//! licence and filed mind (`foreign_reads_as_missing`).
use super::paid_purchase::runtime::{Running, STEP, rows};
use super::player_market::{CREDIT, acknowledged};
use super::player_market_commands::{
    Known, TestResult, World, active, assert_private, await_own, known_accounts, open_shells,
    same_refusal, saved, until_own, world,
};
use mush_core::{GOD, ObjId};
use mush_economy::Account;
use mush_org::PrincipalId;
use mush_proto::{
    InvokeRequest, InvokeResult, ListingCode, MarketAction, MarketListingForm, MindRequest,
    MindResponse, OutputEvent,
};
use mush_roll::commerce::rating::{RatingJob, RatingJobState};
use mush_store::ObjectStore;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::time::{Instant, timeout};

const NOT_OPEN: &str = "The mind market is not open here.";
const COUNTER: &str = "Listing and buying minds happen at a market counter.";
const EMPTY: &str = "No minds are for sale at this counter.";
/// The engine's ingress refusal of a request that fails `validate`.
const INVALID: &str = "Invalid or oversized structured request.";
/// Both panels' one refusal of a malformed request.
const NO_PAGE: &str = "No such market page.";
const DRIVER: &str = "That did not go through. Nothing changed.";
const QUEUED: &str = "Queued.";
/// The typed story's guard: one genuine rating (bounded at four hours, as
/// C2's story bounds it) and half an hour of typed steps, inside the
/// dispatch job's 360 minutes.
const TYPED_STORY_GUARD: Duration = Duration::from_secs(14_400 + 1_800);
/// The launch filing fee: 100cr.
const FEE_MINOR: i64 = 10_000;

/// One typed market action as `actor`, its reply.
async fn typed(server: &Running, actor: ObjId, action: MarketAction) -> TestResult<MindResponse> {
    match server
        .invoke(actor, InvokeRequest::Mind(MindRequest::Market { action }))
        .await?
    {
        InvokeResult::Mind(response) => Ok(response),
        other => Err(format!("a mind result, not {other:?}").into()),
    }
}
/// A typed refusal's code and message, or a failure naming what came back.
fn refused(response: MindResponse) -> TestResult<(String, String)> {
    match response {
        MindResponse::Error { code, message } => Ok((code, message)),
        other => Err(format!("a refusal, not {other:?}").into()),
    }
}
/// One panel query as `actor`: the keyframe's data on `feed`.
async fn panel(server: &Running, actor: ObjId, feed: &str, params: Value) -> TestResult<Value> {
    match server.query(actor, feed, params).await? {
        OutputEvent::Panel {
            feed: got, data, ..
        } if got.as_str() == feed => Ok(data),
        other => Err(format!("a {feed} keyframe, not {other:?}").into()),
    }
}
/// The listing code `code`.
fn code(text: &str) -> TestResult<ListingCode> {
    ListingCode::try_from(text).map_err(|e| e.to_string().into())
}
/// The actor's own lines since the last drain, the invoke results and query
/// replies (recorded as `[invoke_result <id>]` and `[reply <id>] ...`) left
/// out: what a typed request printed on the actor's ordinary output.
fn printed(server: &Running, actor: ObjId) -> TestResult<Vec<String>> {
    Ok(server
        .take_seen(actor)?
        .own
        .into_iter()
        .filter(|line| !line.starts_with("[invoke_result ") && !line.starts_with("[reply "))
        .collect())
}

/// The word each of `thirteen`'s actions refuses with on a number (or code)
/// the actor has no record of, in `thirteen`'s order.
const MISSING_CODES: [&str; 13] = [
    "no_listing",
    "no_quote",
    "no_purchase",
    "no_purchase",
    "no_licence",
    "no_licence",
    "no_licence",
    "no_licence",
    "no_offer",
    "no_filed",
    "no_charter_offer",
    "no_filed",
    "no_filed",
];
/// The observer rule on everything since the last drain (the C3 plan §2.6):
/// no event `actor`'s typed request or line caused reached another of
/// `room`, and the actor's own events pass `assert_private`.
fn quiet(server: &Running, actor: ObjId, room: &[ObjId], known: &Known) -> TestResult {
    let seen = server.event_seen(actor)?;
    for observer in room.iter().filter(|o| **o != actor) {
        let reached = seen.reaching(*observer);
        check!(
            reached.is_empty(),
            "#{}'s request reached #{}: {reached:?}",
            actor.0,
            observer.0
        );
    }
    assert_private(&seen.own, known)
}

/// Each of the thirteen actions on the number `n` (a code where the action
/// takes one), and the equivalent line.
fn thirteen(n: u64, at: &str) -> TestResult<Vec<(MarketAction, String)>> {
    let c = || code(at);
    Ok(vec![
        (
            MarketAction::Quote { code: c()? },
            format!("+minds/buy {at}"),
        ),
        (
            MarketAction::Pay { code: c()? },
            format!("+minds/buy/pay {at}"),
        ),
        (
            MarketAction::Complete { code: c()? },
            format!("+minds/buy/complete {at}"),
        ),
        (
            MarketAction::Refund { code: c()? },
            format!("+minds/buy/refund {at}"),
        ),
        (
            MarketAction::Renew { licence: n },
            format!("+minds/renew {n}"),
        ),
        (
            MarketAction::RenewPay { licence: n },
            format!("+minds/renew/pay {n}"),
        ),
        (
            MarketAction::RenewDecline { licence: n },
            format!("+minds/renew/decline {n}"),
        ),
        (
            MarketAction::Resell {
                licence: n,
                player: "LaunchAuthor".into(),
                price: "5".into(),
            },
            format!("+minds/resell {n}=LaunchAuthor price=5"),
        ),
        (
            MarketAction::ResellAccept {
                licence: n,
                price: Some("0.21".into()),
            },
            format!("+minds/resell/accept {n} price=0.21"),
        ),
        (
            MarketAction::Charter {
                filed: n,
                player: "LaunchAuthor".into(),
                price: "1".into(),
            },
            format!("+minds/charter {n}=LaunchAuthor price=1"),
        ),
        (
            MarketAction::CharterAccept {
                filed: n,
                price: None,
            },
            format!("+minds/charter/accept {n}"),
        ),
        (
            MarketAction::Publish {
                filed: n,
                form: MarketListingForm::Sealed,
                royalty: None,
                transfer: None,
            },
            format!("@mind/publish {n} form=sealed"),
        ),
        (
            MarketAction::Withdraw { filed: n },
            format!("@mind/withdraw {n}"),
        ),
    ])
}

/// 17g C3a test 1 (the C3 plan §4.1): on the format-2 Active launch with no
/// listing, each of the thirteen typed actions refuses exactly as its line
/// does: `Error.message` is the line's one refusal line, byte for byte, and
/// `Error.code` the word's name in lower case (`no_listing`, `no_quote`,
/// `no_purchase`, `no_licence`, `no_offer`, `no_filed`, `no_charter_offer`).
/// Each takes two numbers, the first Roll would allocate (1) and a far one
/// (999), for two characters of two logins (the buyer and the author); the
/// fast suite has no filing and no licence (a filing needs a genuine
/// rating), so both numbers are missing ones, as in C2's tests 8 and 9 (the
/// foreign half, another principal's real licence and filed mind, is the
/// story's `foreign_reads_as_missing`), and the two read alike under
/// `same_refusal` with equal `code`; across the two characters the code and
/// message are the same bytes. Away from the counter a typed `Pay` is
/// `counter`, as the line. A `Publish` with royalty 301 and a `Resell`
/// priced `1 cr` are refused by `validate` before the world: the engine's
/// `invalid` reply, no C2 word, nothing printed or submitted. No event of a
/// typed request or of its line reaches another character in the room
/// (`quiet`).
///
/// The dispatch sits before the workshop's principal gate (the C3a brief
/// §4): the accountless character, whom that gate refuses (it needs an
/// account), reads C2's words on the typed surface: a `Quote` and a
/// `Renew` exactly as the line refuses them (`no_listing`, `no_licence`),
/// and a `Publish` C2's `no_filed`, where the line's `@mind/publish` meets
/// the workshop's gate first and prints its sentence instead (the recorded
/// difference of `market_commands::typed`'s doc).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn typed_refusals_are_the_line_refusals_byte_for_byte() -> TestResult {
    let mut w = world("mush-c3a-refusals", 2).await?;
    let result = timeout(STEP * 10, async {
        let p = w.launch.principals;
        let known = known_accounts(&w).await?;
        let room = [p.author, p.buyer, p.accountless, w.alt, w.bystander];
        let mut by_actor: Vec<Vec<(String, String)>> = Vec::new();
        for (actor, name) in [(p.buyer, "buyer"), (p.author, "author")] {
            let mut replies = Vec::new();
            let (first, far) = (thirteen(1, "feed0001")?, thirteen(999, "0fffffff")?);
            for (index, ((one, line_one), (other, line_other))) in
                first.into_iter().zip(far).enumerate()
            {
                let mut pair = Vec::new();
                for (action, line, typed_n) in [
                    (one, line_one, if index < 4 { "feed0001" } else { "1" }),
                    (
                        other,
                        line_other,
                        if index < 4 { "0fffffff" } else { "999" },
                    ),
                ] {
                    let (code, message) = refused(typed(&w.server, actor, action).await?)?;
                    check_eq!(
                        printed(&w.server, actor)?,
                        Vec::<String>::new(),
                        "{name}: a typed refusal prints nothing: {line}"
                    );
                    let lines = w.server.command(actor, line.clone()).await?;
                    check_eq!(
                        lines,
                        vec![message.clone()],
                        "{name}: the typed message is the line's refusal: {line}"
                    );
                    quiet(&w.server, actor, &room, &known)?;
                    check_eq!(
                        code.as_str(),
                        *MISSING_CODES.get(index).ok_or("a code")?,
                        "{name}: {line}"
                    );
                    pair.push((code, message, typed_n));
                }
                let [(code_a, a, typed_a), (code_b, b, typed_b)] =
                    <[_; 2]>::try_from(pair).map_err(|_| "two replies")?;
                check_eq!(code_a, code_b, "{name}: equal codes");
                check!(
                    same_refusal(std::slice::from_ref(&a), typed_a, &[b], typed_b),
                    "{name}: the two numbers read alike: {a:?}"
                );
                replies.push((code_a, a));
            }
            by_actor.push(replies);
        }
        let [buyer, author] = <[_; 2]>::try_from(by_actor).map_err(|_| "two actors")?;
        check_eq!(
            buyer,
            author,
            "two characters of two logins: the same bytes"
        );
        // Away from the counter, a typed `Pay` is `COUNTER`, as the line.
        w.server
            .command(GOD, format!("@teleport #{}=#0", p.buyer.0))
            .await?;
        let away = refused(
            typed(
                &w.server,
                p.buyer,
                MarketAction::Pay {
                    code: code("feed0001")?,
                },
            )
            .await?,
        )?;
        check_eq!(away, ("counter".to_owned(), COUNTER.to_owned()));
        check_eq!(
            w.server
                .command(p.buyer, "+minds/buy/pay feed0001".into())
                .await?,
            vec![COUNTER.to_owned()]
        );
        quiet(&w.server, p.buyer, &room, &known)?;
        w.server
            .command(
                GOD,
                format!(
                    "@teleport #{}=#{}",
                    p.buyer.0, w.launch.deployment.venue.room.0
                ),
            )
            .await?;
        // `validate` refuses before the world.
        for action in [
            MarketAction::Publish {
                filed: 1,
                form: MarketListingForm::Open,
                royalty: Some(301),
                transfer: None,
            },
            MarketAction::Resell {
                licence: 1,
                player: "LaunchAuthor".into(),
                price: "1 cr".into(),
            },
        ] {
            check_eq!(
                refused(typed(&w.server, p.buyer, action.clone()).await?)?,
                ("invalid".to_owned(), INVALID.to_owned()),
                "{action:?}"
            );
            tokio::time::sleep(Duration::from_millis(200)).await;
            check_eq!(
                printed(&w.server, p.buyer)?,
                Vec::<String>::new(),
                "{action:?}: nothing printed or submitted"
            );
            quiet(&w.server, p.buyer, &room, &known)?;
        }
        // Before the workshop's principal gate: the accountless character.
        for (action, line, word) in [
            (
                MarketAction::Quote {
                    code: code("feed0001")?,
                },
                "+minds/buy feed0001",
                "no_listing",
            ),
            (
                MarketAction::Renew { licence: 999 },
                "+minds/renew 999",
                "no_licence",
            ),
        ] {
            let (got, message) = refused(typed(&w.server, p.accountless, action).await?)?;
            check_eq!(got.as_str(), word, "accountless: {line}");
            check_eq!(
                w.server.command(p.accountless, line.into()).await?,
                vec![message],
                "accountless: the line's bytes: {line}"
            );
            quiet(&w.server, p.accountless, &room, &known)?;
        }
        let (got, message) = refused(
            typed(
                &w.server,
                p.accountless,
                MarketAction::Publish {
                    filed: 999,
                    form: MarketListingForm::Sealed,
                    royalty: None,
                    transfer: None,
                },
            )
            .await?,
        )?;
        check_eq!(
            got.as_str(),
            "no_filed",
            "accountless typed Publish: {message}"
        );
        let gated = w
            .server
            .command(p.accountless, "@mind/publish 999 form=sealed".into())
            .await?;
        check!(
            gated != vec![message.clone()],
            "the line meets the workshop's gate first: {gated:?} vs {message:?}"
        );
        quiet(&w.server, p.accountless, &room, &known)?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })
    .await;
    w.server.stop().await?;
    let _ = std::fs::remove_dir_all(&w.data_dir);
    result?
}

/// The closed `mind.market` keyframe (a format-1 launch).
fn closed_market() -> Value {
    json!({
        "schema": {"id": "mind.market", "version": 1}, "open": false, "notice": NOT_OPEN,
        "rows": [], "after": null, "own": {"quotes": [], "waiting": []},
    })
}
/// The closed `mind.holdings` keyframe for `section`.
fn closed_holdings(section: &str) -> Value {
    json!({
        "schema": {"id": "mind.holdings", "version": 1}, "open": false, "notice": NOT_OPEN,
        "section": section, "rows": [], "after": null,
    })
}

/// 17g C3a test 2 (the C3 plan §4.1): a format-1 (House-only) Active launch
/// composes no player market and so no command slot: each of the thirteen
/// typed actions answers `Error { code: "not_open" }` with the `NOT_OPEN`
/// bytes for the author, the buyer and the bystander; both panel queries
/// answer the closed keyframe (`"open": false`, the `NOT_OPEN` notice, no
/// rows, an empty own strip), for both holdings sections; and the booted
/// registry still lists both feeds, so the manifest is the same on every
/// launch format.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn typed_market_on_format_one_refuses_not_open_and_the_panels_report_closed() -> TestResult {
    let mut w = world("mush-c3a-house-only", 1).await?;
    let result = timeout(STEP * 10, async {
        let p = w.launch.principals;
        for feed in ["mind.market", "mind.holdings"] {
            check!(
                w.server.panel_feeds().iter().any(|f| f == feed),
                "the manifest lists {feed}: {:?}",
                w.server.panel_feeds()
            );
        }
        for actor in [p.author, p.buyer, w.bystander] {
            for (action, line) in thirteen(1, "feed0001")? {
                check_eq!(
                    refused(typed(&w.server, actor, action).await?)?,
                    ("not_open".to_owned(), NOT_OPEN.to_owned()),
                    "#{} {line}",
                    actor.0
                );
            }
            check_eq!(
                panel(&w.server, actor, "mind.market", json!({"after": null})).await?,
                closed_market(),
                "#{}",
                actor.0
            );
            for section in ["owner", "paid"] {
                check_eq!(
                    panel(
                        &w.server,
                        actor,
                        "mind.holdings",
                        json!({"section": section, "after": null}),
                    )
                    .await?,
                    closed_holdings(section),
                    "#{} {section}",
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

/// The open `mind.market` keyframe with no rows and no own records.
fn empty_market(notice: &str) -> Value {
    json!({
        "schema": {"id": "mind.market", "version": 1}, "open": true, "notice": notice,
        "rows": [], "after": null, "own": {"quotes": [], "waiting": []},
    })
}
/// The open `mind.holdings` keyframe for `section` with no rows.
fn empty_holdings(section: &str) -> Value {
    json!({
        "schema": {"id": "mind.holdings", "version": 1}, "open": true, "notice": null,
        "section": section, "rows": [], "after": null,
    })
}

/// 17g C3a test 3 (the C3 plan §4.1, §2.6): on the format-2 Active launch
/// with no listing, through the real granted reads (the registry still
/// composes 33 owner-read services and 35 grants: C3 adds none):
/// - `mind.market` at the counter is open, `EMPTY`, no rows, no cursor and
///   an empty own strip; away from the counter its notice is `COUNTER`; an
///   `after` code no listing has is the `NO_LISTING` notice over the first
///   page; each the same bytes for the author, the buyer and the bystander;
/// - `mind.holdings` answers no rows for each section, alike;
/// - every reply's JSON passes `assert_private` against every account id in
///   the store, and no event of a query reaches another connection (the
///   room's other principals: `event_seen`);
/// - an unknown key, a wrong section, a 7-character `after` and an object
///   past 256 bytes are refused alike: `No such market page.`, and so is a
///   query whose actor is not a player (the counter room itself; the GOING
///   half of that rule is the tacsim lib test
///   `panel_params_refuse_malformed_requests_and_non_players`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn typed_market_panels_are_closed_private_and_own_only() -> TestResult {
    let mut w = world("mush-c3a-panels", 2).await?;
    let result = timeout(STEP * 10, async {
        let p = w.launch.principals;
        check_eq!(
            w.server.owner_read_counts(),
            Some((33, 35)),
            "the composed reads, unchanged by C3"
        );
        let known = known_accounts(&w).await?;
        let room = [p.author, p.buyer, w.alt, w.bystander];
        let principals = [p.author, p.buyer, w.bystander];
        let venue = w.launch.deployment.venue.room;
        let no_listing = "No listing here has the code feed0001.";
        let mut cases: Vec<(&str, Value, Value)> = vec![
            ("mind.market", json!({"after": null}), empty_market(EMPTY)),
            ("mind.market", json!({}), empty_market(EMPTY)),
            (
                "mind.market",
                json!({"after": "feed0001"}),
                empty_market(no_listing),
            ),
        ];
        for section in ["owner", "paid"] {
            cases.push((
                "mind.holdings",
                json!({"section": section, "after": null}),
                empty_holdings(section),
            ));
            cases.push((
                "mind.holdings",
                json!({"section": section, "after": 7}),
                empty_holdings(section),
            ));
        }
        for actor in principals {
            for (feed, params, expected) in &cases {
                let data = panel(&w.server, actor, feed, params.clone()).await?;
                check_eq!(&data, expected, "#{} {feed} {params}", actor.0);
                quiet(&w.server, actor, &room, &known)?;
            }
            // Away from the counter.
            w.server
                .command(GOD, format!("@teleport #{}=#0", actor.0))
                .await?;
            let away = panel(&w.server, actor, "mind.market", json!({"after": null})).await?;
            check_eq!(away, empty_market(COUNTER), "#{} in Limbo", actor.0);
            quiet(&w.server, actor, &room, &known)?;
            w.server
                .command(GOD, format!("@teleport #{}=#{}", actor.0, venue.0))
                .await?;
            // Malformed requests, one sentence.
            let long = "x".repeat(300);
            for (feed, params) in [
                ("mind.market", json!({"after": null, "extra": 1})),
                ("mind.market", json!({"after": "feed000"})),
                ("mind.market", json!({"after": "FEED0001"})),
                ("mind.market", json!({"after": null, "pad": long})),
                ("mind.holdings", json!({"section": "all", "after": null})),
                ("mind.holdings", json!({"section": "owner", "after": -1})),
                ("mind.holdings", json!({"after": null})),
            ] {
                match w.server.query(actor, feed, params.clone()).await? {
                    OutputEvent::Sys(text) => {
                        check_eq!(
                            text.to_plain_text(),
                            NO_PAGE,
                            "#{} {feed} {params}",
                            actor.0
                        );
                    }
                    other => return Err(format!("{feed} {params}: {other:?}").into()),
                }
                quiet(&w.server, actor, &room, &known)?;
            }
        }
        // A query whose actor is not a player: the counter room.
        for (feed, params) in [
            ("mind.market", json!({"after": null})),
            ("mind.holdings", json!({"section": "owner", "after": null})),
        ] {
            match w.server.query(venue, feed, params.clone()).await? {
                OutputEvent::Sys(text) => {
                    check_eq!(text.to_plain_text(), NO_PAGE, "the room: {feed} {params}");
                }
                other => return Err(format!("the room: {feed} {params}: {other:?}").into()),
            }
        }
        check_eq!(w.server.owner_read_counts(), Some((33, 35)));
        Ok::<_, Box<dyn std::error::Error>>(())
    })
    .await;
    w.server.stop().await?;
    let _ = std::fs::remove_dir_all(&w.data_dir);
    result?
}

/// 17g C3a test 4 (the C3 plan §4.1): an accept without a price on a
/// licence (or filed mind) no offer names answers the line's own refusal,
/// `Error { code: "no_offer" }` / `"no_charter_offer"`, byte for byte with
/// `+minds/resell/accept 999` / `+minds/charter/accept 999`, and nothing is
/// printed or submitted (no request line, no `Queued.`, no driver's line),
/// and no event reaches another character in the room (`quiet`). The
/// `Shown` path itself needs an offer, so it is the story's.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn typed_accept_without_a_price_shows_and_submits_nothing() -> TestResult {
    let mut w = world("mush-c3a-accept", 2).await?;
    let result = timeout(STEP * 10, async {
        let p = w.launch.principals;
        let known = known_accounts(&w).await?;
        let room = [p.author, p.buyer, w.alt, w.bystander];
        for (action, line, code) in [
            (
                MarketAction::ResellAccept {
                    licence: 999,
                    price: None,
                },
                "+minds/resell/accept 999",
                "no_offer",
            ),
            (
                MarketAction::CharterAccept {
                    filed: 999,
                    price: None,
                },
                "+minds/charter/accept 999",
                "no_charter_offer",
            ),
        ] {
            let (got, message) = refused(typed(&w.server, p.buyer, action).await?)?;
            check_eq!(got, code, "{line}");
            tokio::time::sleep(Duration::from_millis(200)).await;
            check_eq!(
                printed(&w.server, p.buyer)?,
                Vec::<String>::new(),
                "{line}: nothing printed or submitted"
            );
            check_eq!(
                w.server.command(p.buyer, line.into()).await?,
                vec![message],
                "{line}: the line's bytes"
            );
            quiet(&w.server, p.buyer, &room, &known)?;
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })
    .await;
    w.server.stop().await?;
    let _ = std::fs::remove_dir_all(&w.data_dir);
    result?
}

// ---- the typed story (the C3 plan §4.2; the CI dispatch, never the gate) ----

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
/// The sum of every credit account's posted balance, escrow included.
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
/// A filing row's number field as typed.
fn number(row: &Value, field: &str) -> TestResult<String> {
    row.get(field)
        .and_then(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .or_else(|| value.as_u64().map(|n| n.to_string()))
        })
        .ok_or_else(|| format!("the row's {field}").into())
}
/// What a command-submitted step's last line may be.
fn answered(line: &str) -> bool {
    line == "Done." || line == DRIVER || line.ends_with("(already done)")
}
/// The genuine rating of `revision`, waited for as C2's story waits for its
/// own (read, never edited): Roll's one typed job, `Queued` or `Running`
/// waiting, `Completed` of exactly this revision ending the wait, any other
/// state a failure; and the author's own `Registry work finished.`, after
/// which `@mind/file` finds the finished run.
async fn rated(w: &World, author: ObjId, revision: &mush_roll::Revision) -> TestResult {
    let identity = mush_roll::commerce::RevisionIdentity::capture(revision)?;
    let end = Instant::now() + Duration::from_secs(14_400);
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
/// A typed action that must stage: its `Staged` line, then the actor's own
/// lines until `Done.` or the driver's (the outcome on the ordinary output).
async fn staged(
    w: &World,
    actor: ObjId,
    action: MarketAction,
) -> TestResult<(String, Vec<String>)> {
    let line = match typed(&w.server, actor, action.clone()).await? {
        MindResponse::Staged { line } => line,
        other => return Err(format!("{action:?} staged, not {other:?}").into()),
    };
    let outcome = await_own(&w.server, actor, answered).await?.own;
    check!(
        !outcome.contains(&line),
        "the typed path prints no request line: {line:?} in {outcome:?}"
    );
    Ok((line, outcome))
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
/// A keyframe's rows.
fn rows_of(data: &Value) -> TestResult<Vec<Value>> {
    Ok(data
        .get("rows")
        .and_then(Value::as_array)
        .ok_or("the keyframe's rows")?
        .clone())
}
/// A string field of a row.
fn text<'a>(row: &'a Value, field: &str) -> TestResult<&'a str> {
    row.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("the row's {field}: {row}").into())
}

/// 17g C3a (the C3 plan §4.1, §2.6; reviewer findings F1 and tests/T-1):
/// `actor`'s typed actions `range` of `thirteen` on `theirs`, another
/// principal's real licence or filed mind, against the same actions on a
/// number no record has. Each pair carries the same code (the missing
/// number's word, `MISSING_CODES`) and the same message under
/// `same_refusal`; each refusal is the line's own refusal byte for byte;
/// nothing is printed; no event reaches the room (`quiet`). So "exists but
/// not yours" never reads apart from "does not exist" on the typed surface.
async fn foreign_reads_as_missing(
    w: &World,
    actor: ObjId,
    theirs: u64,
    range: std::ops::Range<usize>,
    known: &Known,
) -> TestResult {
    const MISSING: u64 = 999_999;
    let p = w.launch.principals;
    let room = [p.author, p.buyer, w.alt, w.bystander];
    let pairs = thirteen(theirs, "feed0001")?
        .into_iter()
        .zip(thirteen(MISSING, "feed0001")?);
    for (index, ((foreign, line_foreign), (missing, line_missing))) in pairs.enumerate() {
        if !range.contains(&index) {
            continue;
        }
        let mut replies = Vec::new();
        for (action, line) in [(foreign, line_foreign), (missing, line_missing)] {
            let (code, message) = refused(typed(&w.server, actor, action).await?)?;
            check_eq!(
                printed(&w.server, actor)?,
                Vec::<String>::new(),
                "#{}: a typed refusal prints nothing: {line}",
                actor.0
            );
            check_eq!(
                w.server.command(actor, line.clone()).await?,
                vec![message.clone()],
                "#{}: the typed message is the line's refusal: {line}",
                actor.0
            );
            quiet(&w.server, actor, &room, known)?;
            check_eq!(
                code.as_str(),
                *MISSING_CODES.get(index).ok_or("a code")?,
                "#{}: {line}",
                actor.0
            );
            replies.push((line, message));
        }
        let [(line, foreign), (_, missing)] =
            <[_; 2]>::try_from(replies).map_err(|_| "two replies")?;
        check!(
            same_refusal(
                std::slice::from_ref(&foreign),
                &theirs.to_string(),
                std::slice::from_ref(&missing),
                &MISSING.to_string()
            ),
            "#{}: another's number reads as a missing one: {line}: {foreign:?} vs {missing:?}",
            actor.0
        );
    }
    Ok(())
}
/// 17g C3a (reviewer finding tests/T-2): each of `others` reads an empty
/// own strip on `mind.market` while another actor holds a quote or a
/// waiting purchase: the panel reads the asking actor's own records alone.
async fn own_strip_empty(w: &World, others: &[ObjId]) -> TestResult {
    for actor in others {
        let data = panel(&w.server, *actor, "mind.market", json!({"after": null})).await?;
        check_eq!(
            data.get("own"),
            Some(&json!({"quotes": [], "waiting": []})),
            "#{}: {data}",
            actor.0
        );
    }
    Ok(())
}
/// 17g C3a (reviewer finding tests/T-7): the author's `mind.holdings` owner
/// section against `+minds/holdings`: exactly one row, for `filed`, whose
/// name (cut to 20) and phase make the very `HOLD_OWNER` row the line
/// prints, with the provisional actions (`charter` unless the charter is
/// being sold), private, and no cursor. Polled within `STEP`: the owner row
/// lands with the filing's commit.
async fn owner_row_matches_the_line(
    w: &World,
    author: ObjId,
    filed: u64,
    known: &Known,
) -> TestResult<Value> {
    let section = json!({"section": "owner", "after": null});
    let end = Instant::now() + STEP;
    let data = loop {
        let data = panel(&w.server, author, "mind.holdings", section.clone()).await?;
        if !rows_of(&data)?.is_empty() {
            break data;
        }
        check!(Instant::now() < end, "the author's owner row: {data}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_private(&[data.to_string()], known)?;
    check_eq!(data.get("after"), Some(&Value::Null), "{data}");
    let [row]: [Value; 1] = rows_of(&data)?
        .try_into()
        .map_err(|_| format!("one owner row: {data}"))?;
    check_eq!(row.get("kind"), Some(&json!("owner")), "{row}");
    check_eq!(row.get("filed"), Some(&json!(filed)), "{row}");
    check_eq!(row.get("dependent"), Some(&json!(false)), "{row}");
    let name = text(&row, "name")?;
    check!(name.chars().count() <= 20, "the name is cut to 20: {row}");
    let selling = row
        .get("selling")
        .and_then(Value::as_bool)
        .ok_or("the row's selling")?;
    let phase = match (
        row.pointer("/phase/state").and_then(Value::as_str),
        row.pointer("/phase/days_left").and_then(Value::as_u64),
    ) {
        (Some("active"), Some(days)) => format!("Active, {days} days left"),
        (Some("grace"), Some(days)) => format!("Grace, {days} days left"),
        (Some("lapsed"), None) => "Lapsed".to_owned(),
        other => return Err(format!("the row's phase {other:?}: {row}").into()),
    };
    let actions = if selling {
        json!(["publish", "withdraw"])
    } else {
        json!(["publish", "withdraw", "charter"])
    };
    check_eq!(row.get("actions"), Some(&actions), "{row}");
    let flags = if selling { "; selling" } else { "" };
    let line = format!(
        "  filed {filed:<8}  {:<9}  {name:<20}  {phase}{flags}",
        "owner"
    );
    let held = w.server.command(author, "+minds/holdings".into()).await?;
    check!(held.contains(&line), "{line:?} in {held:?}");
    Ok(row)
}

/// 17g C3a's typed story (the C3 plan §4.2): one genuine Registry rating,
/// then the positive money path through the typed surfaces alone, checked
/// against the line at every step. The author files by command (part 1 of
/// C2's story, its rating wait rewritten here over C2's helpers); then:
/// - the author's typed `Publish` stages `PUBLISHING`'s bytes, the provider
///   commits, and `mind.market`'s rows are `+minds`' rows in order (code,
///   name, price) with each row's `terms_line` and `split_line` exactly
///   `+minds <code>`'s lines 2 and 3;
/// - the buyer (another login, never the author's alt) types `Quote`,
///   `Pay` and `Complete`, each staging the line's request sentence; the own
///   strip carries the quote at the line's price, then the waiting purchase;
///   `mind.holdings` (`paid`) carries the new licence with its name and phase
///   and the actions `renew` and `resell`, as `+minds/holdings` lists it;
/// - the buyer's typed `Resell` to the author stages `RESELLING`; the
///   author's typed `ResellAccept` without a price is `Shown` with the offer
///   and its confirming line, and with the price stages `ACCEPTING` (the
///   outcome either the provider's commit or the driver's refusal, every
///   account's sum unchanged either way);
/// - the author's typed `Withdraw` stages `WITHDRAWING`, and `mind.market`
///   then lists the same rows: the Registry stops new sales, but the counter
///   is policy-current only (17g C2e; the C4 ledger item in the C2e record);
/// - after a restart, `mind.holdings` for the licence's holder is the same
///   bytes.
///
/// And, beside the money path (the reviewers' findings on the C3a build):
/// - the author's `mind.holdings` owner row is `+minds/holdings`' row, once
///   filed and again once listed (`owner_row_matches_the_line`);
/// - no staged step prints its request line on the actor's output (`staged`);
/// - while the buyer holds a quote, then a waiting purchase, the author's and
///   the bystander's own strips are empty, and once the buyer holds the
///   licence the bystander's paid section is empty and the author's has no
///   row for it (`own_strip_empty`);
/// - every number-taking action on another principal's real number reads as
///   on a missing one, on the typed surface and the line alike: the author
///   and the bystander on the buyer's licence, the buyer (and, for the two
///   `+minds/charter` forms, the bystander) on the author's filed mind
///   (`foreign_reads_as_missing`);
/// - the buyer's typed `Renew` answers `Staged` with the `RENEW_HANDED`
///   sentence and the renewal service's own not-due line follows alone,
///   while `+minds/renew` on the line prints only that service line.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "17g C3 typed market story: one genuine Registry rating; run by the mind-commerce-acceptance dispatch"]
async fn typed_market_story_quote_pay_complete_panels_resale_and_restart() -> TestResult {
    let mut w = world("mush-c3-typed-story", 2).await?;
    let result = timeout(TYPED_STORY_GUARD, async {
        let p = w.launch.principals;
        let (author, buyer) = (p.author, p.buyer);
        // Part 1 (C2's, by command): fund the fee, save, reserve, rate, file.
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
        let reserving = w
            .server
            .command(author, "@mind/file Sentinel@1".into())
            .await?;
        until_own(&w.server, author, reserving, answered).await?;
        let end = Instant::now() + STEP;
        while open_shells(&w.launch, author).await?.len() != 1 {
            check!(Instant::now() < end, "the author's one open shell");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        check_eq!(
            w.server
                .command(author, "@mind/rate Sentinel@1".into())
                .await?,
            vec!["Registry preparation queued.".to_owned()]
        );
        rated(&w, author, &revision).await?;
        let filing = w
            .server
            .command(author, "@mind/file Sentinel@1".into())
            .await?;
        let filing = until_own(&w.server, author, filing, answered).await?;
        check_eq!(
            filing.last().map(String::as_str),
            Some("Done."),
            "{filing:?}"
        );
        let end = Instant::now() + STEP;
        let filed = loop {
            let operations = rows(&w.launch.store, "roll", "roll_mind_initial_filing").await?;
            if let [operation] = operations.as_slice() {
                break number(operation, "filed")?.parse::<u64>()?;
            }
            check!(Instant::now() < end, "one initial filing row");
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        let buyer_wallet = wallet(&w, &PrincipalId::Object(buyer)).await?;
        acknowledged(
            &w.server,
            format!("@econ/mint {}={} 200000", faucet.id.0, buyer_wallet.id.0),
            "Minted",
        )
        .await?;
        let known = known_accounts(&w).await?;
        let filed_row = owner_row_matches_the_line(&w, author, filed, &known).await?;
        check_eq!(filed_row.get("selling"), Some(&json!(false)), "{filed_row}");

        // The author's typed Publish.
        let (line, outcome) = staged(
            &w,
            author,
            MarketAction::Publish {
                filed,
                form: MarketListingForm::Open,
                royalty: None,
                transfer: None,
            },
        )
        .await?;
        check_eq!(line, format!("Listing filed mind {filed} here as open."));
        check_eq!(
            outcome.last().map(String::as_str),
            Some("Done."),
            "{outcome:?}"
        );
        owner_row_matches_the_line(&w, author, filed, &known).await?;
        // The panel against the line.
        let listing = w.server.command(buyer, "+minds".into()).await?;
        let market = panel(&w.server, buyer, "mind.market", json!({"after": null})).await?;
        assert_private(&[market.to_string()], &known)?;
        let panel_rows = rows_of(&market)?;
        let line_rows: Vec<&String> = listing
            .iter()
            .filter(|l| l.starts_with("  ") && l.len() > 10)
            .collect();
        check_eq!(panel_rows.len(), line_rows.len(), "{listing:?} vs {market}");
        check_eq!(panel_rows.len(), 1, "one listing: {listing:?}");
        for (row, line) in panel_rows.iter().zip(&line_rows) {
            let code = text(row, "code")?;
            check!(
                line.starts_with(&format!("  {code}  {:<24}  ", text(row, "name")?))
                    && line.contains(text(row, "price_text")?),
                "{line} vs {row}"
            );
            let detail = w.server.command(buyer, format!("+minds {code}")).await?;
            check_eq!(
                detail.get(1).map(String::as_str),
                Some(text(row, "terms_line")?)
            );
            check_eq!(
                detail.get(2).map(String::as_str),
                Some(text(row, "split_line")?)
            );
        }
        let first = panel_rows.first().ok_or("the listing row")?;
        let at = text(first, "code")?.to_owned();
        let price_text = text(first, "price_text")?.to_owned();

        // The buyer's typed Quote, Pay and Complete.
        let before = total(&w).await?;
        let (line, outcome) = staged(&w, buyer, MarketAction::Quote { code: code(&at)? }).await?;
        check_eq!(
            line,
            format!(
                "Asking for a quote on listing {at}. +minds {at} shows it once it is confirmed."
            )
        );
        check_eq!(
            outcome.last().map(String::as_str),
            Some("Done."),
            "{outcome:?}"
        );
        let quoted = panel(&w.server, buyer, "mind.market", json!({"after": null})).await?;
        let own_quote = quoted
            .pointer("/own/quotes/0")
            .cloned()
            .ok_or("the own quote")?;
        check_eq!(text(&own_quote, "code")?, at.as_str());
        check_eq!(text(&own_quote, "price_text")?, price_text.as_str());
        check_eq!(own_quote.get("expired"), Some(&json!(false)));
        own_strip_empty(&w, &[author, w.bystander]).await?;
        let shown = w.server.command(buyer, format!("+minds {at}")).await?;
        check!(
            shown
                .last()
                .is_some_and(|l| l.starts_with(&format!("Your quote: {price_text}, good for "))),
            "{shown:?}"
        );
        let (line, outcome) = staged(&w, buyer, MarketAction::Pay { code: code(&at)? }).await?;
        check_eq!(
            line,
            format!("Paying your quote on listing {at} into escrow.")
        );
        check_eq!(
            outcome.last().map(String::as_str),
            Some("Done."),
            "{outcome:?}"
        );
        let paid = panel(&w.server, buyer, "mind.market", json!({"after": null})).await?;
        check_eq!(
            paid.pointer("/own/waiting"),
            Some(&json!([{"code": at}])),
            "{paid}"
        );
        own_strip_empty(&w, &[author, w.bystander]).await?;
        let (line, outcome) =
            staged(&w, buyer, MarketAction::Complete { code: code(&at)? }).await?;
        check_eq!(line, format!("Taking the licence from listing {at}."));
        check_eq!(
            outcome.last().map(String::as_str),
            Some("Done."),
            "{outcome:?}"
        );
        check_eq!(total(&w).await?, before, "every account's sum is unchanged");
        let [licence]: [u64; 1] = paid_licences(&w, buyer)
            .await?
            .try_into()
            .map_err(|_| "one paid licence")?;
        let held = w.server.command(buyer, "+minds/holdings".into()).await?;
        check!(
            held.iter()
                .any(|l| l.starts_with(&format!("  licence {licence:<8}  paid "))
                    && l.contains("sentinel")),
            "{held:?}"
        );
        let holdings = panel(
            &w.server,
            buyer,
            "mind.holdings",
            json!({"section": "paid", "after": null}),
        )
        .await?;
        assert_private(&[holdings.to_string()], &known)?;
        let paid_row = rows_of(&holdings)?.first().cloned().ok_or("the paid row")?;
        check_eq!(paid_row.get("licence"), Some(&json!(licence)));
        check_eq!(text(&paid_row, "name")?, "sentinel");
        check_eq!(
            paid_row.pointer("/phase/state").and_then(Value::as_str),
            Some("active")
        );
        check_eq!(paid_row.get("actions"), Some(&json!(["renew", "resell"])));
        // Own-only: nobody else's paid section carries the buyer's licence.
        let paid_section = json!({"section": "paid", "after": null});
        let bystander_paid = panel(
            &w.server,
            w.bystander,
            "mind.holdings",
            paid_section.clone(),
        )
        .await?;
        check_eq!(
            rows_of(&bystander_paid)?,
            Vec::<Value>::new(),
            "{bystander_paid}"
        );
        let author_paid = panel(&w.server, author, "mind.holdings", paid_section).await?;
        check!(
            rows_of(&author_paid)?
                .iter()
                .all(|row| row.get("licence") != Some(&json!(licence))),
            "{author_paid}"
        );

        // Another principal's real numbers read as missing ones.
        foreign_reads_as_missing(&w, author, licence, 4..9, &known).await?;
        foreign_reads_as_missing(&w, w.bystander, licence, 4..9, &known).await?;
        foreign_reads_as_missing(&w, buyer, filed, 9..13, &known).await?;
        foreign_reads_as_missing(&w, w.bystander, filed, 9..11, &known).await?;

        // The typed renewal: handed to the service, which answers alone.
        let not_due = |l: &str| {
            l.starts_with(&format!(
                "Licence {licence} is not due for renewal; its term has "
            )) && l.ends_with(" IC days left.")
        };
        match typed(&w.server, buyer, MarketAction::Renew { licence }).await? {
            MindResponse::Staged { line } => check_eq!(
                line,
                format!("Handing licence {licence} to the renewal service.")
            ),
            other => return Err(format!("the renewal handed on, not {other:?}").into()),
        }
        let answer: Vec<String> = await_own(&w.server, buyer, not_due)
            .await?
            .own
            .into_iter()
            .filter(|l| !l.starts_with("[invoke_result "))
            .collect();
        check!(
            answer.len() == 1 && answer.iter().all(|l| not_due(l)),
            "the service's line alone, no request line: {answer:?}"
        );
        let renew = w
            .server
            .command(buyer, format!("+minds/renew {licence}"))
            .await?;
        let renew = until_own(&w.server, buyer, renew, not_due).await?;
        check!(
            renew.len() == 1 && renew.iter().all(|l| not_due(l)),
            "the line prints nothing of its own: {renew:?}"
        );

        // The buyer's typed resale offer to the author, and the author's
        // typed accept: shown without a price, staged with it.
        let (line, outcome) = staged(
            &w,
            buyer,
            MarketAction::Resell {
                licence,
                player: "LaunchAuthor".into(),
                price: "0.21".into(),
            },
        )
        .await?;
        check_eq!(
            line,
            format!("Offering licence {licence} to LaunchAuthor for 0.21cr.")
        );
        check_eq!(
            outcome.last().map(String::as_str),
            Some("Done."),
            "{outcome:?}"
        );
        let lines = match typed(
            &w.server,
            author,
            MarketAction::ResellAccept {
                licence,
                price: None,
            },
        )
        .await?
        {
            MindResponse::Shown { lines } => lines,
            other => return Err(format!("the offers shown, not {other:?}").into()),
        };
        check_eq!(lines.len(), 2, "{lines:?}");
        check!(
            lines.first().is_some_and(|l| l.starts_with(&format!(
                "Licence {licence} is offered to you for 0.21cr; the offer is open "
            ))),
            "{lines:?}"
        );
        check_eq!(
            lines.get(1).map(String::as_str),
            Some(format!("Accept it with +minds/resell/accept {licence} price=0.21.").as_str())
        );
        let before = total(&w).await?;
        let (line, outcome) = staged(
            &w,
            author,
            MarketAction::ResellAccept {
                licence,
                price: Some("0.21".into()),
            },
        )
        .await?;
        check_eq!(
            line,
            format!("Accepting the offer for licence {licence} at 0.21cr.")
        );
        check!(
            outcome.iter().any(|l| l == QUEUED) || outcome.iter().any(|l| answered(l)),
            "{outcome:?}"
        );
        check_eq!(total(&w).await?, before, "every account's sum is unchanged");
        let holder = if paid_licences(&w, author).await?.contains(&licence) {
            author
        } else {
            buyer
        };

        // The author's typed withdrawal: the Registry stops new sales of the
        // revision (a fresh quote on it is refused by the provider, as the
        // command story's part 3 pins on the line), but the counter is
        // policy-current only (17g C2e: `current_policy` reads the listing
        // row, never the revision's status), so the panel reads the same rows
        // after it. Hiding a withdrawn revision's listings is a C4 ledger item
        // (17g section 7, C2e as built).
        let listed =
            rows_of(&panel(&w.server, buyer, "mind.market", json!({"after": null})).await?)?;
        check!(
            !listed.is_empty(),
            "the listing is on the counter before the withdrawal"
        );
        let (line, outcome) = staged(&w, author, MarketAction::Withdraw { filed }).await?;
        check_eq!(line, format!("Withdrawing filed revision {filed}."));
        check_eq!(
            outcome.last().map(String::as_str),
            Some("Done."),
            "{outcome:?}"
        );
        let after = panel(&w.server, buyer, "mind.market", json!({"after": null})).await?;
        check_eq!(rows_of(&after)?, listed, "{after}");

        // A restart: the holder's paid section reads the same bytes.
        let section = json!({"section": "paid", "after": null});
        let before = panel(&w.server, holder, "mind.holdings", section.clone()).await?;
        w.server.stop().await?;
        w.server = active(&w.launch, w.alt, w.bystander).await?;
        check_eq!(
            panel(&w.server, holder, "mind.holdings", section).await?,
            before,
            "the holdings survive the restart"
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })
    .await;
    w.server.stop().await?;
    let _ = std::fs::remove_dir_all(&w.data_dir);
    result?
}
