#![cfg(test)]

//! 17g C2a: the command adapter's pure parts. The commands' composition is
//! proven in `composition::mind_commerce::tests`, and their behaviour over a
//! real server in mush-server's `player_market_commands`.
use super::author::{
    charter_name, file_arguments, fork_arguments, number, revision_reference, update_arguments,
};
use super::next_key;
use super::words::{self, fill};
use mush_core::action::IdempotencyKeyText;
use std::sync::Mutex;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// 17g C2a test 1: every argument parses strictly.
#[test]
fn author_arguments_parse_strictly() -> TestResult {
    // Library references: a saved selector is required.
    let named = revision_reference("Sentinel@1").ok_or("a reference")?;
    check_eq!(named.name(), "sentinel", "canonical");
    check!(
        revision_reference("  Sentinel@first  ").is_some(),
        "a named version"
    );
    check!(revision_reference("Sentinel").is_none(), "the working copy");
    check!(revision_reference("Sentinel@").is_none());
    check!(revision_reference("Sentinel@1@2").is_none());
    check!(revision_reference("#99/secret@1").is_none());
    // Filed and licence numbers: decimal, nonzero, no sign or padding.
    check_eq!(number("1"), Some(1));
    check_eq!(number("18446744073709551615"), Some(u64::MAX));
    for bad in [
        "0",
        "-1",
        "+1",
        "01",
        " 1",
        "1 ",
        "1.0",
        "0x1",
        "1e3",
        "one",
        "",
        "18446744073709551616",
    ] {
        check_eq!(number(bad), None, "{bad:?}");
    }
    // Charter names: trimmed, 1 to 80 bytes, no control characters.
    let eighty = "c".repeat(80);
    check_eq!(
        charter_name(None),
        Ok(None),
        "the default is the mind's name"
    );
    check_eq!(charter_name(Some(&eighty)), Ok(Some(eighty.clone())));
    check_eq!(
        charter_name(Some(&format!("  {eighty}  "))),
        Ok(Some(eighty.clone())),
        "padding is trimmed"
    );
    check_eq!(charter_name(Some(&"c".repeat(81))), Err(()));
    check_eq!(charter_name(Some("   ")), Err(()));
    check_eq!(charter_name(Some("Line\tName")), Err(()));
    check_eq!(charter_name(Some("Line\u{7}Name")), Err(()));
    check_eq!(
        charter_name(Some("équipe étoile")),
        Ok(Some("équipe étoile".to_owned()))
    );
    let (reference, charter) = file_arguments("Sentinel@1= Sentinel Line ").ok_or("file")?;
    check_eq!(reference.name(), "sentinel");
    check_eq!(charter, Some("Sentinel Line".to_owned()));
    check_eq!(
        file_arguments("Sentinel@1").map(|(_, charter)| charter),
        Some(None)
    );
    check!(file_arguments("Sentinel=Line").is_none());
    check!(file_arguments(&format!("Sentinel@1={}", "c".repeat(81))).is_none());
    // Update: `<filed> <name>@<rev>`.
    let (filed, reference) = update_arguments("17 Sentinel@2").ok_or("update")?;
    check_eq!((filed, reference.name()), (17, "sentinel"));
    for bad in [
        "17",
        "017 Sentinel@2",
        "17 Sentinel",
        "-17 Sentinel@2",
        "Sentinel@2 17",
    ] {
        check!(update_arguments(bad).is_none(), "{bad:?}");
    }
    // Fork: `<licence>=<name>`, the name through `canonical_name`.
    check_eq!(
        fork_arguments("12=Sentinel Fork"),
        Some((12, "sentinel fork".to_owned()))
    );
    for bad in ["12", "0=Fork", "+12=Fork", "12=", "12=  ", "x=Fork"] {
        check!(fork_arguments(bad).is_none(), "{bad:?}");
    }
    check!(
        fork_arguments(&format!("12={}", "n".repeat(65))).is_none(),
        "a name past the library's bound"
    );
    // The words fill only their own placeholders, never typed text.
    check_eq!(
        fill(words::NO_FILED, &[("filed", "{filed}")]),
        "You own no filed mind {filed}."
    );
    check_eq!(
        fill(
            words::FILING,
            &[("reference", "A@1"), ("charter", "{reference}")]
        ),
        "Filing A@1 as {reference}."
    );
    Ok(())
}

/// 17g C2a test 2: two keys in one IC second take ordinals 0 and 1, a new
/// second restarts at 0, and every key is in the idempotency key grammar.
#[test]
fn command_keys_differ_per_retype_and_fit_the_key_grammar() -> TestResult {
    let attempts = Mutex::new((i64::MIN, 0));
    let at = 1_234_567_i64;
    let first = next_key(&attempts, "file", "7.1", at).map_err(|e| e.to_string())?;
    let second = next_key(&attempts, "file", "7.1", at).map_err(|e| e.to_string())?;
    check_eq!(first.as_str(), "mind.file.7.1.12d687.0");
    check_eq!(second.as_str(), "mind.file.7.1.12d687.1");
    check_ne!(first, second, "a retyped command is a new request");
    let later = next_key(&attempts, "reserve", "7.1", at + 1).map_err(|e| e.to_string())?;
    check_eq!(later.as_str(), "mind.reserve.7.1.12d688.0");
    // Every verb and subject the commands use stays in the grammar, at the
    // extremes of the IC clock and the ordinal.
    for (verb, subject) in [
        ("reserve", "18446744073709551615.18446744073709551615"),
        ("file", "1.1"),
        ("update", "18446744073709551615.1.1"),
        ("withdraw", "18446744073709551615"),
        ("attest", "1"),
        ("fork", "18446744073709551615"),
    ] {
        for at in [i64::MIN, -1, 0, i64::MAX] {
            let key = next_key(&attempts, verb, subject, at).map_err(|e| e.to_string())?;
            check!(
                IdempotencyKeyText::parse_canonical(key.as_str()).is_ok(),
                "{key}"
            );
        }
    }
    Ok(())
}

/// Reviewer F2: `@mind/rate` answers `RATING_BUSY` only for the character's
/// own Registry job; every other Busy (bulk recovery, an `@mind/prove` run or
/// the alt's job on the same login, a full queue) and every other error is
/// the service's unavailable line. `held` is consulted only for a Busy.
#[test]
fn rate_refuses_busy_only_for_the_characters_own_registry_job() -> TestResult {
    use super::author::rate_refusal;
    use crate::minds::bulk::BulkError;
    check_eq!(rate_refusal(&BulkError::Busy, || true), words::RATING_BUSY);
    check_eq!(
        rate_refusal(&BulkError::Busy, || false),
        words::RATING_UNAVAILABLE
    );
    for error in [
        BulkError::Missing,
        BulkError::QueueFull,
        BulkError::Quota,
        BulkError::Stale,
    ] {
        let asked = std::cell::Cell::new(false);
        let held = || {
            asked.set(true);
            true
        };
        check_eq!(rate_refusal(&error, held), words::RATING_UNAVAILABLE);
        check!(!asked.get(), "held is asked only for a Busy");
    }
    Ok(())
}

/// 17g C2c test 4 (the C2c brief's widths, the 120-column floor): every
/// table row and line at its maxima, with every legal flag combination.
/// 8-digit numbers, the longest names (cut to 24 and 20), a 12-column price
/// (`{price:>12}`; the longest price `credits` prints is `999999.99cr`),
/// `open-resale`, class III, a two-digit Grace. The brief's figures are
/// recomputed here and pinned: `ROW` 106, `DETAIL` 86, `TERMS_LINE` 76,
/// `SPLIT` under 100 with both optional cuts, `HOLD_OWNER` 80 and
/// `HOLD_WAIT` 96; any line over 120 fails rather than wraps. The own
/// purchase, buy-step and holdings words (live since 17g C2g) are measured
/// too.
///
/// 17g C2g (the lead's ruling R4): the paid row `HOLD_PAID` is no longer
/// enumerated here. Its one enumerator, under the one legality rule
/// `holdings::paid_flags` states, is `trade_lines_fit_one_hundred_and_
/// twenty_columns` (C2d test 8), which varies the phase and pins the widest
/// legal row at 118; this test enumerated a different flag list, without
/// the reachable `; no resale; revoked; renewal …` row in Grace.
#[test]
fn market_rows_fit_one_hundred_and_twenty_columns() -> TestResult {
    use super::holdings::owner_row;
    use super::market::{Shown, detail_text, row_text};
    use mush_market::mind_commands::ListingForm;
    use mush_roll::commerce::rating::RatingClass;
    let width = |line: &str| line.chars().count();
    let long = "A charter name far longer than any column allows";
    let day = 86_400;
    let mut shown = Shown {
        code: "feed0001".to_owned(),
        name: long,
        form: ListingForm::OpenResale,
        class: RatingClass::III,
        price_minor: 99_999_999,
        term_seconds: 90 * day,
        grace_seconds: 99 * day,
        transferable: false,
        derived: true,
        levy_permille: 125,
        venue_permille: 125,
        parent_royalty_permille: 305,
    };
    let mut widest = 0;
    for (transferable, derived) in [(true, false), (false, false), (true, true), (false, true)] {
        shown.transferable = transferable;
        shown.derived = derived;
        let row = row_text(&shown);
        check!(width(&row) <= 120, "{row}");
        widest = widest.max(width(&row));
    }
    check_eq!(widest, 106, "ROW at the maxima");
    let [detail, terms, split] = detail_text(&shown);
    check_eq!(width(&detail), 86, "{detail}");
    check_eq!(width(&terms), 76, "{terms}");
    check!(width(&split) < 100, "{split}");
    check!(split.contains(", venue 12.5%") && split.contains(", parent royalties 30.5%"));
    // Holdings: owner rows, both kinds, both phases, selling or not.
    let mut widest = 0;
    for dependent in [false, true] {
        for phase in ["Active, 90 days left", "Grace, 99 days left", "Lapsed"] {
            for selling in [false, true] {
                let row = owner_row(99_999_999, dependent, long, phase, selling);
                check!(width(&row) <= 120, "{row}");
                widest = widest.max(width(&row));
            }
        }
    }
    check_eq!(widest, 80, "HOLD_OWNER at the maxima");
    let wait = fill(words::HOLD_WAIT, &[("code", "feed0001")]);
    check_eq!(width(&wait), 96, "{wait}");
    // Every other sentence at its longest fill.
    let code = [("code", "feed0001")];
    for line in [
        fill(words::MORE, &code),
        fill(words::NO_LISTING, &code),
        fill(words::AMBIGUOUS, &code),
        fill(words::QUOTING, &code),
        super::market::publishing_line("99999999", "open-resale", false, true),
        fill(words::TERMS, &[("price", "999999.99cr"), ("term", "90")]),
        fill(
            words::HOLD_MORE,
            &[("section", "owner"), ("cursor", "4294967295")],
        ),
        fill(
            words::HOLD_MORE,
            &[("section", "paid"), ("cursor", "18446744073709551615")],
        ),
        fill(
            words::YOUR_QUOTE,
            &[
                ("price", "999999.99cr"),
                ("seconds", "600"),
                ("code", "feed0001"),
            ],
        ),
        fill(words::QUOTE_EXPIRED, &code),
        fill(words::YOUR_ESCROW, &code),
        fill(words::ASK, &code),
        fill(words::NO_QUOTE, &code),
        fill(words::PAYING, &code),
        fill(words::NO_PURCHASE, &code),
        fill(words::COMPLETING, &code),
        fill(words::REFUNDING, &code),
        words::HOLD_WAIT_HEAD.to_owned(),
        words::HOLD_EMPTY.to_owned(),
        super::market::HELP.to_owned(),
        super::market::USAGE_PUBLISH.to_owned(),
    ] {
        for part in line.lines() {
            check!(width(part) <= 120, "{part}");
        }
    }
    Ok(())
}

/// 17g C2c: `fill`'s padding (`{key:<n}`, `{key:>n}`) and `cut`.
#[test]
fn fill_pads_to_the_named_width() -> TestResult {
    check_eq!(fill("[{a:<4}]", &[("a", "x")]), "[x   ]");
    check_eq!(fill("[{a:>4}]", &[("a", "x")]), "[   x]");
    check_eq!(fill("[{a:<2}]", &[("a", "long")]), "[long]", "never cut");
    check_eq!(fill("[{a:^4}]", &[("a", "x")]), "[{a:^4}]", "not a fill");
    check_eq!(
        fill("[{a}]", &[("a", "{a:<4}")]),
        "[{a:<4}]",
        "typed text stays"
    );
    check_eq!(words::cut("abcdef", 3), "abc");
    Ok(())
}

/// 17g C2c test 5: `@mind/publish`'s arguments parse strictly: the three
/// forms (`open-resale` as typed, `open_resale` on the wire), the royalty
/// bounds and the sealed zero, the transfer words, `price=`, `seats=` and
/// `term=` accepted only at the market's values, and unknown or repeated
/// keys refused with the usage line. Also `+minds/holdings`' arguments.
#[test]
fn publish_arguments_parse_strictly() -> TestResult {
    use super::holdings::{Holdings, holdings_arguments};
    use super::market::{PublishRefusal as R, credits_minor, publish_arguments, terms_match};
    let ok = |text: &str| publish_arguments(text).map_err(|e| format!("{text}: {e:?}"));
    let sealed = ok("12 form=sealed")?;
    check_eq!(
        (
            sealed.filed,
            sealed.form,
            sealed.shown,
            sealed.royalty,
            sealed.transferable
        ),
        (12, "sealed", "sealed", 0, true)
    );
    let open = ok("12 form=open royalty=300 transfer=no")?;
    check_eq!(
        (open.form, open.royalty, open.transferable),
        ("open", 300, false)
    );
    let resale = ok("  12   form=open-resale   royalty=0 transfer=yes ")?;
    check_eq!(
        (
            resale.form,
            resale.shown,
            resale.royalty,
            resale.transferable
        ),
        ("open_resale", "open-resale", 0, true)
    );
    for (text, refusal) in [
        ("12 form=open_resale", R::Form),
        ("12 form=Sealed", R::Form),
        ("12 form=", R::Form),
        ("12 form=sealed royalty=1", R::Royalty),
        ("12 form=open royalty=301", R::Royalty),
        ("12 form=open royalty=-1", R::Royalty),
        ("12 form=open royalty=01", R::Royalty),
        ("12 form=open royalty=ten", R::Royalty),
        ("12 form=open transfer=No", R::Transfer),
        ("12 form=open transfer=false", R::Transfer),
        ("12 form=open transfer=", R::Transfer),
        ("", R::Usage),
        ("form=open", R::Usage),
        ("0 form=open", R::Usage),
        ("012 form=open", R::Usage),
        ("12", R::Usage),
        ("12 royalty=10", R::Usage),
        ("12 form=open form=open", R::Usage),
        ("12 form=open royalty=1 royalty=1", R::Usage),
        ("12 form=open seller=7", R::Usage),
        ("12 form=open sealed", R::Usage),
        ("12 form=open price=", R::Usage),
        ("12 form=open price=4.001", R::Usage),
        ("12 form=open seats=00", R::Usage),
        ("12 form=open term=00", R::Usage),
        ("12 form=open price=00", R::Usage),
        ("12 form=open term=ninety", R::Usage),
    ] {
        check_eq!(publish_arguments(text), Err(refusal), "{text:?}");
    }
    // Only the market's own values: 400cr, one seat, 90 IC days.
    for text in [
        "12 form=open",
        "12 form=open price=400",
        "12 form=open price=400cr",
        "12 form=open price=400.00 seats=1 term=90",
        "12 form=open price=400.0cr term=90",
    ] {
        check!(terms_match(&ok(text)?, 40_000, 90), "{text}");
    }
    for text in [
        "12 form=open price=1",
        "12 form=open price=400.01",
        "12 form=open seats=2",
        "12 form=open term=30",
    ] {
        check!(!terms_match(&ok(text)?, 40_000, 90), "{text}");
    }
    // The credit grammar (C2 plan §2.7).
    for (text, minor) in [
        ("0.21cr", Some(21)),
        ("0.2", Some(20)),
        ("1", Some(100)),
        ("1000000", Some(100_000_000)),
        ("999999.99cr", Some(99_999_999)),
        ("0", None),
        ("0.00", None),
        ("1000000.01", None),
        ("01", None),
        (".5", None),
        ("1.", None),
        ("1.234", None),
        ("-1", None),
        ("1 cr", None),
        ("cr", None),
    ] {
        check_eq!(credits_minor(text), minor, "{text:?}");
    }
    // `+minds/holdings`' arguments.
    check_eq!(holdings_arguments(""), Some(Holdings::All));
    check_eq!(holdings_arguments("owner 41"), Some(Holdings::Owner(41)));
    check_eq!(holdings_arguments("paid 7"), Some(Holdings::Paid(7)));
    for bad in [
        "owner",
        "paid",
        "owner 0",
        "owner 4 5",
        "licences 4",
        "owner x",
    ] {
        check_eq!(holdings_arguments(bad), None, "{bad:?}");
    }
    Ok(())
}

/// 17g C2e test 7 (C2e plan §2.1 items 4b and 4c, review FE-F13).
///
/// - `price=0`, `seats=0` and `term=0` (and `price=0cr`, `price=0.00`)
///   parse, a zero being a value of `@mind/publish`'s own parse, and
///   `terms_match` refuses each against the counter's values, so `publish`
///   answers `TERMS` with the counter's price and term, not the usage line.
///   The trade price grammar (`credits_minor`) still refuses a zero.
/// - The `PUBLISHING` request line carries the listing row's flags as `ROW`
///   prints them: `; no resale` for `transfer=no`, `; derived` for a
///   dependent line, both together; and for a transferable listing of an
///   original line it is byte-identical to the line before 17g C2e.
#[test]
fn publish_refuses_zero_terms() -> TestResult {
    use super::market::{credits_minor, publish_arguments, publishing_line, terms_match};
    for (text, price, seats, term) in [
        ("12 form=open price=0", Some(0), None, None),
        ("12 form=open price=0cr", Some(0), None, None),
        ("12 form=open price=0.00", Some(0), None, None),
        ("12 form=open seats=0", None, Some(0), None),
        ("12 form=open term=0", None, None, Some(0)),
    ] {
        let args = publish_arguments(text).map_err(|e| format!("{text}: {e:?}"))?;
        check_eq!(
            (args.price, args.seats, args.term),
            (price, seats, term),
            "{text}"
        );
        check!(!terms_match(&args, 40_000, 90), "{text}");
    }
    check_eq!(credits_minor("0"), None, "the trade grammar is unchanged");
    check_eq!(
        fill(words::TERMS, &[("price", "400cr"), ("term", "90")]),
        "Listings here sell at 400cr for one seat and 90 IC days; per-listing terms are not \
         available yet."
    );
    check_eq!(
        publishing_line("12", "open", true, false),
        "Listing filed mind 12 here as open.",
        "unchanged for a transferable original listing"
    );
    check_eq!(
        publishing_line("12", "open-resale", false, false),
        "Listing filed mind 12 here as open-resale; no resale."
    );
    check_eq!(
        publishing_line("12", "open", true, true),
        "Listing filed mind 12 here as open; derived."
    );
    check_eq!(
        publishing_line("12", "sealed", false, true),
        "Listing filed mind 12 here as sealed; no resale, derived."
    );
    Ok(())
}

/// 17g C2e (the C2e fix round, reviewer tests/T2): `publish`'s request is
/// built by one pure function (`list_request`) from the parsed arguments and
/// the filed line's own facts, so the `PUBLISHING` line it prints names the
/// same transfer mark and derivation as the List offer it submits and the
/// protocol it picks. Over the four combinations: a dependent line lists
/// through the derived protocol and its line says `derived`; `transfer=no`
/// is `"transferable": false` in the offer and `no resale` in the line; the
/// offer carries the List wire's form and royalty and the owner licence.
#[test]
fn publish_request_line_matches_the_offer() -> TestResult {
    use super::market::{list_request, publish_arguments};
    use mush_market::mind_commands::Protocol;
    let licence = mush_roll::commerce::LicenceId::new(7)?;
    for (text, dependent, protocol, line, form, royalty, transferable) in [
        (
            "12 form=open",
            false,
            Protocol::Original,
            "Listing filed mind 12 here as open.",
            "open",
            0,
            true,
        ),
        (
            "12 form=open transfer=no",
            false,
            Protocol::Original,
            "Listing filed mind 12 here as open; no resale.",
            "open",
            0,
            false,
        ),
        (
            "12 form=open-resale royalty=300",
            true,
            Protocol::Derived,
            "Listing filed mind 12 here as open-resale; derived.",
            "open_resale",
            300,
            true,
        ),
        (
            "12 form=sealed transfer=no",
            true,
            Protocol::Derived,
            "Listing filed mind 12 here as sealed; no resale, derived.",
            "sealed",
            0,
            false,
        ),
    ] {
        let args = publish_arguments(text).map_err(|e| format!("{text}: {e:?}"))?;
        let request = list_request(&args, dependent, licence);
        check_eq!(request.protocol, protocol, "{text}");
        check_eq!(request.line, line, "{text}");
        check_eq!(
            request.parameters,
            serde_json::json!({
                "owner_licence": licence,
                "form": form,
                "royalty_permille": royalty,
                "transferable": transferable,
            }),
            "{text}"
        );
    }
    Ok(())
}

/// 17g C2c (the C2c fix round, reviewers F3, C2C-4 and F4): a page that
/// shows no rows but goes on prints its head and its cursor, never the
/// "nothing at all" sentence. Listings: `HEAD` then `MORE` with the cursor's
/// 8-character code (no `FOOT`, which names codes no row shows). A paged
/// owner section: `HOLD_HEAD` then `HOLD_MORE owner <cursor>`, and past its
/// end `HOLD_HEAD` alone, never `HOLD_EMPTY` (the bare form's sentence).
#[test]
fn an_empty_page_that_goes_on_prints_its_cursor() -> TestResult {
    use super::holdings::owner_lines;
    use super::market::page_lines;
    let cursor = mush_core::CommitToken::from_bytes([0xfe; 16]);
    check_eq!(
        page_lines(&[], Some(cursor), &[]),
        vec![
            words::HEAD.to_owned(),
            "More: +minds/next fefefefe".to_owned()
        ]
    );
    check_eq!(
        owner_lines(&[], Some(4_294_967_295)),
        vec![
            words::HOLD_HEAD.to_owned(),
            "More: +minds/holdings owner 4294967295".to_owned()
        ]
    );
    check_eq!(owner_lines(&[], None), vec![words::HOLD_HEAD.to_owned()]);
    for lines in [
        page_lines(&[], Some(cursor), &[]),
        owner_lines(&[], Some(1)),
        owner_lines(&[], None),
    ] {
        check!(
            !lines
                .iter()
                .any(|l| l == words::EMPTY || l == words::HOLD_EMPTY)
        );
    }
    Ok(())
}

/// 17g C2d test 6 (C2 plan §2.7): the trade commands' arguments parse
/// strictly. A seller's `<number>=<player> price=<credits>` (the player may
/// not hold `=`; `price=` comes last); the price grammar (`<whole>[.<one or
/// two digits>]`, an optional `cr`, 0.01 to 1,000,000); a buyer's `<number>`
/// with and without `price=`. A malformed shape is the `+minds` help
/// (`Usage`), a malformed or out-of-bounds price `BAD_PRICE` (`Price`). And
/// `amount`, the `price=` spelling the request lines print.
#[test]
fn trade_arguments_parse_strictly() -> TestResult {
    use super::trade::{
        AcceptArguments, TradeArguments, TradeRefusal, accept_arguments, amount, trade_arguments,
    };
    let seller = |subject: u64, player: &str, price_minor: i64| TradeArguments {
        subject,
        player: player.to_owned(),
        price_minor,
    };
    check_eq!(
        trade_arguments("12=Cy price=0.21"),
        Ok(seller(12, "Cy", 21))
    );
    check_eq!(
        trade_arguments(" 12 = Cy  price=5.00 "),
        Ok(seller(12, "Cy", 500)),
        "padding around the parts"
    );
    check_eq!(trade_arguments("12=#7 price=1"), Ok(seller(12, "#7", 100)));
    check_eq!(
        trade_arguments("12=Old Cy price=12.50cr"),
        Ok(seller(12, "Old Cy", 1250)),
        "a name with a space"
    );
    // The price grammar, accepted.
    for (text, minor) in [
        ("0.01", 1),
        ("0.21", 21),
        ("1000000", 100_000_000),
        ("12.50cr", 1250),
        ("12.5", 1250),
        ("7", 700),
    ] {
        check_eq!(
            trade_arguments(&format!("12=Cy price={text}")).map(|a| a.price_minor),
            Ok(minor),
            "{text}"
        );
        check_eq!(
            accept_arguments(&format!("12 price={text}")).map(|a| a.price_minor),
            Ok(Some(minor)),
            "{text}"
        );
    }
    // The price grammar, refused.
    for text in [
        "0",
        "0.00",
        "1000000.01",
        "1.234",
        "-1",
        "1,5",
        "",
        "01",
        "1.",
        ".5",
        "cr",
        "12cr5",
    ] {
        check_eq!(
            trade_arguments(&format!("12=Cy price={text}")),
            Err(TradeRefusal::Price),
            "{text:?}"
        );
        check_eq!(
            accept_arguments(&format!("12 price={text}")),
            Err(TradeRefusal::Price),
            "{text:?}"
        );
    }
    // Shapes.
    for text in [
        "",
        "12",
        "12=Cy",
        "12=price=1",
        "=Cy price=1",
        "x=Cy price=1",
        "0=Cy price=1",
        "012=Cy price=1",
        "12=Cy cost=1",
        "12=Cy=Dee price=1",
        "12=Cy price=1 now",
    ] {
        check_eq!(trade_arguments(text), Err(TradeRefusal::Usage), "{text:?}");
    }
    check_eq!(
        accept_arguments("12"),
        Ok(AcceptArguments {
            subject: 12,
            price_minor: None
        })
    );
    check_eq!(
        accept_arguments("  12   price=0.21 "),
        Ok(AcceptArguments {
            subject: 12,
            price_minor: Some(21)
        })
    );
    for text in [
        "",
        "x",
        "0",
        "12 cost=1",
        "12 price=1 more",
        "12 13",
        "price=1 12",
    ] {
        check_eq!(accept_arguments(text), Err(TradeRefusal::Usage), "{text:?}");
    }
    // The `price=` spelling.
    check_eq!(amount(21), "0.21");
    check_eq!(amount(500), "5");
    check_eq!(amount(1250), "12.50");
    check_eq!(amount(100_000_000), "1000000");
    for minor in [1, 21, 500, 1250, 99_999_999, 100_000_000] {
        check_eq!(
            super::market::credits_minor(&amount(minor)),
            Some(minor),
            "{minor} round-trips"
        );
    }
    Ok(())
}

/// 17g C2d test 7 (C2 plan §2.6): the buyer confirms the frozen price.
/// With a 0.21cr and a 5.00cr offer naming the actor: no `price=` submits
/// nothing and lists both (cheapest first, each with its confirming line);
/// `price=0.21` submits that offer with `ACCEPTING`; `price=1` refuses
/// `NO_OFFER_AT` and lists both; an equal-price tie submits the one expiring
/// last; an expired offer is not live; none at all is `NO_OFFER`. The charter
/// accept is the same over its own words (`FUNDING_CHARTER`).
#[test]
fn accept_selects_the_offer_the_buyer_confirms() -> TestResult {
    use super::trade::{AcceptPlan, Named, NamedOffer, accept_plan};
    use mush_core::CommitToken;
    let now = 1_000_000;
    let offer = |n: u8, price_minor: i64, left: i64| NamedOffer {
        offer: CommitToken::from_bytes([n; 16]),
        price_minor,
        expires_at_utc: now + left,
    };
    let cheap = offer(1, 21, 600);
    let dear = offer(2, 500, 1_201);
    let both = [dear, cheap];
    let listed = vec![
        "Licence 12 is offered to you for 0.21cr; the offer is open 10 more minutes.".to_owned(),
        "Accept it with +minds/resell/accept 12 price=0.21.".to_owned(),
        "Licence 12 is offered to you for 5cr; the offer is open 21 more minutes.".to_owned(),
        "Accept it with +minds/resell/accept 12 price=5.".to_owned(),
    ];
    check_eq!(
        accept_plan(Named::Resale, 12, &both, None, now),
        AcceptPlan::Show(listed.clone()),
        "no price= submits nothing"
    );
    check_eq!(
        accept_plan(Named::Resale, 12, &both, Some(21), now),
        AcceptPlan::Submit(
            cheap,
            "Accepting the offer for licence 12 at 0.21cr.".to_owned()
        )
    );
    check_eq!(
        accept_plan(Named::Resale, 12, &both, Some(500), now),
        AcceptPlan::Submit(
            dear,
            "Accepting the offer for licence 12 at 5cr.".to_owned()
        )
    );
    let mut refused = vec!["None of your offers for licence 12 is at 1cr.".to_owned()];
    refused.extend(listed);
    check_eq!(
        accept_plan(Named::Resale, 12, &both, Some(100), now),
        AcceptPlan::Refuse(refused)
    );
    // An equal-price tie: the later expiry.
    let later = offer(3, 21, 900);
    for offers in [[cheap, later], [later, cheap]] {
        match accept_plan(Named::Resale, 12, &offers, Some(21), now) {
            AcceptPlan::Submit(chosen, _) => check_eq!(chosen, later),
            other => return Err(format!("a tie submits: {other:?}").into()),
        }
    }
    // An expired offer is not live; none at all is NO_OFFER.
    let expired = offer(4, 21, 0);
    check_eq!(
        accept_plan(Named::Resale, 12, &[expired], Some(21), now),
        AcceptPlan::Refuse(vec!["You have no offer for licence 12.".to_owned()])
    );
    check_eq!(
        accept_plan(Named::Resale, 12, &[], None, now),
        AcceptPlan::Refuse(vec!["You have no offer for licence 12.".to_owned()])
    );
    // The charter accept.
    check_eq!(
        accept_plan(Named::Charter, 7, &both, Some(500), now),
        AcceptPlan::Submit(
            dear,
            "Paying 5cr for filed mind 7 into escrow; the transfer settles once no sortie under it is out."
                .to_owned()
        )
    );
    let AcceptPlan::Show(charter_lines) = accept_plan(Named::Charter, 7, &both, None, now) else {
        return Err("the charter listing".into());
    };
    check_eq!(
        charter_lines.first().map(String::as_str),
        Some(
            "Filed mind 7 and its charter are offered to you for 0.21cr; the offer is open 10 more minutes."
        )
    );
    check_eq!(
        charter_lines.get(1).map(String::as_str),
        Some("Accept it with +minds/charter/accept 7 price=0.21.")
    );
    check_eq!(
        accept_plan(Named::Charter, 7, &[], Some(1), now),
        AcceptPlan::Refuse(vec!["You have no offer for filed mind 7.".to_owned()])
    );
    let mut refused = vec!["None of your offers for filed mind 7 is at 0.01cr.".to_owned()];
    refused.extend(charter_lines);
    check_eq!(
        accept_plan(Named::Charter, 7, &both, Some(1), now),
        AcceptPlan::Refuse(refused)
    );
    Ok(())
}

/// The one legality rule of a paid row's flags, exactly as
/// `holdings::paid_flags`' doc states it (C2d test 8, the one width test of
/// paid rows, enumerates under it; the truth table checks it against a
/// hand-written list): at most one renewal mark (the `Option`);
/// `selling` only on a transferable, unrevoked licence; `selling` never with
/// a renewal mark; a renewal mark never with an Active phase; `revoked` with
/// a renewal mark legal.
fn legal_paid_flags(
    active: bool,
    transferable: bool,
    revoked: bool,
    selling: bool,
    renewal: Option<mush_roll::commerce::command_reader::RenewalMark>,
) -> bool {
    !(selling && (!transferable || revoked))
        && !(selling && renewal.is_some())
        && !(active && renewal.is_some())
}

/// 17g C2d test 8 (the 120-column floor): every C2d sentence at its maxima
/// fits 120 columns: 8-digit licence and filed numbers, a 12-character price
/// (`1000000.00cr`) and its 10-character `price=` spelling, the server's
/// longest player name (20 characters, `mush_config::accounts`' character
/// name rule), four-digit minutes and hours, 99 IC days; the longest is
/// `CHARTER_OFFER` at 109. The `+minds` and `@registry` help lines fit too.
///
/// 17g C2g (the lead's ruling R4): this is the ONE enumerator of the paid
/// row `HOLD_PAID`, under the one legality rule `holdings::paid_flags`
/// states (`legal_paid_flags`), over every flag combination, both kinds and
/// the PHASE at its two-digit maxima: `Active, 99 days left` (20 columns),
/// `Grace, 99 days left` (19) and `Lapsed` (6). The renewal words are
/// `; renew offer` and `; renew paid`, so the reachable C2D-R4 row (`;
/// no resale; revoked;` and a renewal mark, in Grace: a renewal is drafted
/// only after the term ends and a revocation admits a matching pending
/// renewal) is no longer filtered out: the widest legal row is that one, at
/// 85 plus 33, so 118 columns, pinned (a 3-day Grace makes it 117; the pin is the
/// two-digit maximum the other widths use). The old `set <= 2` and
/// `renewal.is_some() && (revoked || selling)` exclusions are gone.
#[test]
fn trade_lines_fit_one_hundred_and_twenty_columns() -> TestResult {
    use super::holdings::{paid_flags, paid_row};
    // 17g C2h: the renewal service prints the renewal words after its step,
    // so they are the market's now (verbatim; the pins below are unchanged).
    use mush_market::mind_renewal::player as renewal;
    use mush_roll::commerce::command_reader::RenewalMark;
    let width = |line: &str| line.chars().count();
    let n = "99999999";
    let player = "P".repeat(20);
    let all = [
        ("licence", n),
        ("filed", n),
        ("price", "1000000.00cr"),
        ("amount", "1000000.00"),
        ("player", player.as_str()),
        ("name", player.as_str()),
        ("minutes", "9999"),
        ("hours", "9999"),
        ("days", "99"),
        ("term", "99"),
        ("grace", "99"),
    ];
    let mut widest = ("", 0);
    for (name, template) in [
        ("NO_LICENCE", words::NO_LICENCE),
        ("RENEW_NOT_DUE", renewal::RENEW_NOT_DUE),
        ("RENEWING", renewal::RENEWING),
        ("RENEW_OFFER", renewal::RENEW_OFFER),
        ("RENEW_HOW", renewal::RENEW_HOW),
        ("RENEW_FUNDED", renewal::RENEW_FUNDED),
        ("NO_RENEWAL", words::NO_RENEWAL),
        ("RENEW_PAYING", renewal::RENEW_PAYING),
        ("RENEW_DECLINING", renewal::RENEW_DECLINING),
        ("RESELLING", words::RESELLING),
        ("RESELL_TELL", words::RESELL_TELL),
        ("CHARTERING", words::CHARTERING),
        ("CHARTER_TELL", words::CHARTER_TELL),
        ("OFFER", words::named::OFFER),
        ("ACCEPT_HOW", words::named::ACCEPT_HOW),
        ("NO_OFFER", words::named::NO_OFFER),
        ("NO_OFFER_AT", words::named::NO_OFFER_AT),
        ("CHARTER_OFFER", words::named::CHARTER_OFFER),
        ("CHARTER_ACCEPT_HOW", words::named::CHARTER_ACCEPT_HOW),
        ("NO_CHARTER_OFFER", words::named::NO_CHARTER_OFFER),
        ("NO_CHARTER_OFFER_AT", words::named::NO_CHARTER_OFFER_AT),
        ("ACCEPTING", words::named::ACCEPTING),
        ("FUNDING_CHARTER", words::named::FUNDING_CHARTER),
        ("NO_PLAYER", words::NO_PLAYER),
        ("BAD_PRICE", words::BAD_PRICE),
        ("RECALLING", words::RECALLING),
        ("REVOKING", words::REVOKING),
        ("REGISTRY_OFF", words::REGISTRY_OFF),
    ] {
        let line = fill(template, &all);
        check!(!line.contains('{'), "{name} is filled: {line}");
        check!(width(&line) <= 120, "{name}: {line}");
        if width(&line) > widest.1 {
            widest = (name, width(&line));
        }
    }
    check_eq!(widest, ("CHARTER_OFFER", 109), "the longest C2d sentence");
    // The paid holdings row: every legal combination, every phase.
    let renewals = [None, Some(RenewalMark::Offered), Some(RenewalMark::Paid)];
    let mut widest = ("", 0, String::new(), String::new());
    let mut legal_rows = 0;
    for (phase, active) in [
        ("Active, 99 days left", true),
        ("Grace, 99 days left", false),
        ("Lapsed", false),
    ] {
        for transferable in [true, false] {
            for revoked in [false, true] {
                for selling in [false, true] {
                    for renewal in renewals {
                        if !legal_paid_flags(active, transferable, revoked, selling, renewal) {
                            continue;
                        }
                        let flags = paid_flags(transferable, revoked, selling, renewal);
                        for house in [false, true] {
                            let row = paid_row(
                                99_999_999,
                                house,
                                "A charter name far longer than any column allows",
                                "open-resale",
                                phase,
                                &flags,
                            );
                            legal_rows += 1;
                            check!(width(&row) <= 120, "{row}");
                            if width(&row) > widest.1 {
                                widest =
                                    ("HOLD_PAID", width(&row), phase.to_owned(), flags.clone());
                            }
                        }
                    }
                }
            }
        }
    }
    check_eq!(
        legal_rows,
        2 * (5 + 13 + 13),
        "both kinds of every legal row"
    );
    check_eq!(
        widest,
        (
            "HOLD_PAID",
            118,
            "Grace, 99 days left".to_owned(),
            "; no resale; revoked; renew offer".to_owned()
        ),
        "the widest legal paid row"
    );
    check_eq!(
        paid_flags(true, false, false, Some(RenewalMark::Paid)),
        "; renew paid"
    );
    for line in super::market::HELP
        .lines()
        .chain(crate::minds::bulk::registry_service::HELP.lines())
    {
        check!(width(line) <= 120, "{line}");
    }
    Ok(())
}

/// 17g C2d fix round (reviewer T1): `+minds/renew`'s drafted-offer lines
/// from the view's raw fields. The frozen price prints as credits, the
/// renewed term and Grace in whole IC days and the offer's remaining IC hours
/// rounded up, never below 0 (an offer at or past its expiry reads 0).
///
/// 17g C2h (the C2e plan §5.1): the renewal service's `front` prints these
/// lines after its step, so the functions are the market's
/// (`mush_market::mind_renewal::player`); the pins are unchanged, and the
/// market's credit and day renderings are TacSim's own (`launch::credits`,
/// `market::days`), compared here. The `RENEWALS_OPEN` pin is gone with the
/// hold: the dispatched form (a held licence reaching `RENEW_NOT_DUE`, then
/// the offer, the payment and the grant) is the player market story's part 3.
#[test]
fn renewal_offer_lines_round_hours_up() -> TestResult {
    use mush_market::mind_renewal::player::{credits, days, hours_left, offer_text};
    const HOUR: i64 = 3_600;
    const DAY: i64 = 86_400;
    for minor in [0, 1, 99, 100, 1_250, 40_000, 100_000_000, -1, -150] {
        check_eq!(credits(minor), crate::launch::credits(minor), "{minor}");
    }
    for seconds in [-1, 0, 1, DAY - 1, DAY, DAY + 1, 90 * DAY, 90 * DAY - 1] {
        check_eq!(days(seconds), super::market::days(seconds), "{seconds}");
    }
    for (until, now, hours) in [
        (HOUR, 0, 1),
        (HOUR + 1, 0, 2),
        (1, 0, 1),
        (0, 0, 0),
        (0, 10, 0),
        (48 * HOUR, HOUR, 47),
        (48 * HOUR, HOUR - 1, 48),
    ] {
        check_eq!(hours_left(until, now), hours, "{until} at {now}");
    }
    let now = 1_000 * DAY;
    check_eq!(
        offer_text("41", 50_000, 90 * DAY, 3 * DAY, now + 24 * HOUR - 1, now),
        [
            "Licence 41 renews for 500cr (90+3 IC days); the offer is open 24 more IC hours."
                .to_owned(),
            "Pay it with +minds/renew/pay 41, or turn it down with +minds/renew/decline 41."
                .to_owned(),
        ]
    );
    check_eq!(
        offer_text("41", 1_250, 90 * DAY - 1, 0, now - 5, now)[0],
        "Licence 41 renews for 12.50cr (90+0 IC days); the offer is open 0 more IC hours."
    );
    Ok(())
}

/// 17g C2g (the lead's ruling R4): the truth table of the one legality rule
/// `holdings::paid_flags` states. Over every phase (Active, Grace, Lapsed),
/// transfer mark, revocation, sale and renewal mark, `legal_paid_flags`
/// admits exactly these rows, whose flags `paid_flags` prints in its fixed
/// order with the R4 renewal words:
/// - Active (no renewal mark): none, `no resale`, `revoked`, `selling`,
///   `no resale; revoked`;
/// - Grace and Lapsed: the same five, and each of the four unsold
///   (transfer, revocation) states with `renew offer` or `renew paid`.
///
/// So `selling` is never with `no resale`, `revoked` or a renewal mark, and
/// `revoked` with a renewal mark is legal. The admitted sets are written out
/// by hand, and of the 24 combinations per phase the rule refuses 19 in
/// Active and 11 in Grace and Lapsed (counted, not re-derived from the rule).
/// The rule is about what Roll lets a row reach (`paid_flags`' doc); this
/// test pins the rule's statement and `paid_flags`' words and order.
#[test]
fn paid_flags_follow_the_one_rule() -> TestResult {
    use super::holdings::paid_flags;
    use mush_roll::commerce::command_reader::RenewalMark;
    use std::collections::BTreeSet;
    let unsold = ["", "; no resale", "; revoked", "; no resale; revoked"];
    let mut quiet: BTreeSet<String> = unsold.iter().map(|f| (*f).to_owned()).collect();
    quiet.insert("; selling".to_owned());
    let mut renewing = quiet.clone();
    for flags in unsold {
        for mark in ["; renew offer", "; renew paid"] {
            renewing.insert(format!("{flags}{mark}"));
        }
    }
    check_eq!((quiet.len(), renewing.len()), (5, 13));
    let renewals = [None, Some(RenewalMark::Offered), Some(RenewalMark::Paid)];
    for (phase, active, expected, refused_rows) in [
        ("Active", true, &quiet, 19),
        ("Grace", false, &renewing, 11),
        ("Lapsed", false, &renewing, 11),
    ] {
        let mut admitted = BTreeSet::new();
        let mut refused = 0;
        for transferable in [true, false] {
            for revoked in [false, true] {
                for selling in [false, true] {
                    for renewal in renewals {
                        let flags = paid_flags(transferable, revoked, selling, renewal);
                        if legal_paid_flags(active, transferable, revoked, selling, renewal) {
                            check!(admitted.insert(flags.clone()), "{phase}: one row {flags:?}");
                        } else {
                            refused += 1;
                        }
                    }
                }
            }
        }
        check_eq!(&admitted, expected, "{phase}");
        check_eq!(refused, refused_rows, "{phase}: refused combinations");
    }
    check_eq!(
        paid_flags(false, true, false, Some(RenewalMark::Offered)),
        "; no resale; revoked; renew offer",
        "the widest legal flags, in order"
    );
    check_eq!(paid_flags(true, false, true, None), "; selling");
    check_eq!(paid_flags(true, false, false, None), "");
    Ok(())
}

/// 17g C2g (the C2e plan §4.1 item 2): `+minds <code>`'s own-purchase line
/// (`market::own_line`, the C2c words). A funded purchase is `YOUR_ESCROW`
/// whatever quote sits beside it; else the newest quote is `YOUR_QUOTE` with
/// its price and the whole seconds left while the world clock is before its
/// expiry, and `QUOTE_EXPIRED` at or after it (the provider's own Fund test,
/// `now >= expires_at_utc` refuses); no record is no line (then `ASK` when
/// the listing is current).
#[test]
fn own_purchase_line_follows_the_actors_record() -> TestResult {
    use super::market::own_line;
    let now = 1_000;
    check_eq!(
        own_line("feed0001", true, Some((40_000, now + 30)), now),
        Some(
            "Your payment is held in escrow. Take the licence with +minds/buy/complete feed0001."
                .to_owned()
        )
    );
    check_eq!(
        own_line("feed0001", false, Some((40_000, now + 30)), now),
        Some(
            "Your quote: 400cr, good for 30 more seconds. Pay it with +minds/buy/pay feed0001."
                .to_owned()
        )
    );
    check_eq!(
        own_line("feed0001", false, Some((21, now + 1)), now),
        Some(
            "Your quote: 0.21cr, good for 1 more seconds. Pay it with +minds/buy/pay feed0001."
                .to_owned()
        )
    );
    for expires in [now, now - 1] {
        check_eq!(
            own_line("feed0001", false, Some((40_000, expires)), now),
            Some("Your quote has expired. Ask for a new one with +minds/buy feed0001.".to_owned()),
            "expired at {expires}"
        );
    }
    check_eq!(own_line("feed0001", false, None, now), None);
    check_eq!(
        own_line("feed0001", true, None, now).as_deref(),
        Some("Your payment is held in escrow. Take the licence with +minds/buy/complete feed0001.")
    );
    Ok(())
}

/// 17g C2g (the C2e plan §4.1 item 2): bare `+minds/holdings`' lines
/// (`holdings::all_lines`). Nothing in either section is `HOLD_EMPTY`
/// alone; a section that shows no row but goes on (a cursor) is not
/// "nothing" (the C2c fix round's rule), so the head and its `HOLD_MORE`
/// print instead; rows print owner first, then paid, each section with its
/// own cursor line; the waiting purchases follow in every case, whatever the
/// sections hold. A paged paid section (`section_lines`) never prints
/// `HOLD_EMPTY`.
#[test]
fn bare_holdings_print_empty_only_with_nothing_in_either_section() -> TestResult {
    use super::holdings::{all_lines, section_lines};
    let none = || (Vec::<String>::new(), None);
    check_eq!(
        all_lines(none(), none(), &[]),
        vec![words::HOLD_EMPTY.to_owned()]
    );
    check_eq!(
        all_lines(none(), none(), &["feed0001".to_owned()]),
        vec![
            words::HOLD_EMPTY.to_owned(),
            words::HOLD_WAIT_HEAD.to_owned(),
            "  listing feed0001  paid into escrow: +minds/buy/complete feed0001 or \
             +minds/buy/refund feed0001"
                .to_owned(),
        ]
    );
    check_eq!(
        all_lines((Vec::new(), Some(16)), none(), &[]),
        vec![
            words::HOLD_HEAD.to_owned(),
            "More: +minds/holdings owner 16".to_owned()
        ],
        "an owner section that goes on is not nothing"
    );
    check_eq!(
        all_lines(
            (vec!["owner row".to_owned()], None),
            (vec!["paid row".to_owned()], Some(41)),
            &["abcdef01".to_owned(), "feed0001".to_owned()]
        ),
        vec![
            words::HOLD_HEAD.to_owned(),
            "owner row".to_owned(),
            "paid row".to_owned(),
            "More: +minds/holdings paid 41".to_owned(),
            words::HOLD_WAIT_HEAD.to_owned(),
            "  listing abcdef01  paid into escrow: +minds/buy/complete abcdef01 or \
             +minds/buy/refund abcdef01"
                .to_owned(),
            "  listing feed0001  paid into escrow: +minds/buy/complete feed0001 or \
             +minds/buy/refund feed0001"
                .to_owned(),
        ]
    );
    check_eq!(
        section_lines(Vec::new(), "paid", None),
        vec![words::HOLD_HEAD.to_owned()]
    );
    check_eq!(
        section_lines(Vec::new(), "paid", Some(7)),
        vec![
            words::HOLD_HEAD.to_owned(),
            "More: +minds/holdings paid 7".to_owned()
        ]
    );
    Ok(())
}

/// 17g C2g (the C2e plan §4.2, the C2g fix round's tests/T1 and
/// correctness/C2G-1): each buy step's and accept's (verb, subject) comes
/// from the one function its command submits with (`market::buy_key`,
/// `pay_key`, `settle_key`, `trade::accept_key`):
/// - `/buy/pay` is the quote's TOKEN, `/buy/complete` and `/buy/refund` the
///   operation's, the accepts the offer's (32 characters each, never the
///   typed code or number), and `+minds/buy` the listing's token, then the
///   token of the actor's most recently quoted own record there when it has
///   one (`OwnPurchaseView::latest`, a quote or a funded purchase; the C2g
///   fix round 2);
/// - every key stays inside the key grammar and its 128-byte bound at the
///   extremes of the IC clock (`+minds/buy` with a record is the longest);
/// - after a restart (a fresh ordinal, the IC clock stopped at one second):
///   a re-quote after a committed quote, a re-quote after that quote's Fund
///   (the quote has left the index; the purchase names the subject), a pay
///   of another quote, and a settle of another purchase are all new keys,
///   while the same step on the same token (the same request) meets its
///   earlier key;
/// - which record `latest` names in each state of the actor's index, and
///   the stated residual (`next_key`'s doc: after a Grant or Refund the
///   subject can be an earlier one's again), are pinned by mush-market's
///   `own_purchase_latest_names_the_newest_own_record`.
#[test]
fn buy_step_keys_take_the_token_as_subject() -> TestResult {
    use super::market::{Settle, buy_key, pay_key, settle_key};
    use super::trade::{Named, accept_key};
    use mush_core::CommitToken;
    let listing = CommitToken::from_bytes([0xfe; 16]);
    let quote = CommitToken::from_bytes([0x01; 16]);
    let operation = CommitToken::from_bytes([0x02; 16]);
    let offer = CommitToken::from_bytes([0x03; 16]);
    let token = |t: CommitToken| t.to_string();
    check_eq!(token(listing).len(), 32);
    let steps = [
        (buy_key(listing, None), ("buy", token(listing))),
        (
            buy_key(listing, Some(quote)),
            ("buy", format!("{}.{}", token(listing), token(quote))),
        ),
        (pay_key(quote), ("buy.pay", token(quote))),
        (
            settle_key(Settle::Complete, operation),
            ("buy.complete", token(operation)),
        ),
        (
            settle_key(Settle::Refund, operation),
            ("buy.refund", token(operation)),
        ),
        (
            accept_key(Named::Resale, offer),
            ("resell.accept", token(offer)),
        ),
        (
            accept_key(Named::Charter, offer),
            ("charter.accept", token(offer)),
        ),
    ];
    let attempts = Mutex::new((i64::MIN, 0));
    for ((verb, subject), expected) in &steps {
        check_eq!(&(*verb, subject.clone()), expected);
        for at in [i64::MIN, -1, 0, i64::MAX] {
            let key = next_key(&attempts, verb, subject, at).map_err(|e| e.to_string())?;
            check!(key.as_str().len() <= 128, "{key}");
            check!(
                IdempotencyKeyText::parse_canonical(key.as_str()).is_ok(),
                "{key}"
            );
            check!(
                key.as_str().starts_with(&format!("mind.{verb}.{subject}.")),
                "{key}"
            );
        }
    }
    // A restart under a stopped IC clock: the ordinal is back at 0.
    let key = |(verb, subject): (&str, String)| {
        next_key(&Mutex::new((i64::MIN, 0)), verb, &subject, 7).map_err(|e| e.to_string())
    };
    let other = CommitToken::from_bytes([0x04; 16]);
    check_ne!(
        key(buy_key(listing, None))?,
        key(buy_key(listing, Some(quote)))?,
        "a re-quote after a committed quote is a new key"
    );
    let firsts = [
        key(buy_key(listing, None))?,
        key(buy_key(listing, Some(quote)))?,
    ];
    check!(
        !firsts.contains(&key(buy_key(listing, Some(operation)))?),
        "a re-quote after the quote's Fund is a new key"
    );
    check_ne!(key(pay_key(quote))?, key(pay_key(other))?);
    check_ne!(
        key(settle_key(Settle::Complete, operation))?,
        key(settle_key(Settle::Complete, other))?
    );
    check_ne!(
        key(settle_key(Settle::Complete, operation))?,
        key(settle_key(Settle::Refund, operation))?
    );
    check_eq!(
        key(pay_key(quote))?,
        key(pay_key(quote))?,
        "the same pay of the same quote is the same request"
    );
    Ok(())
}

/// 17g C2g (the C2g fix round's tests/T5): the multi-line replies of
/// `+minds <code>` (`market::detail_reply`) and of an accept's refusal
/// (`market_commands::refusal` over `trade::accept_plan`'s lines):
/// - a current listing prints its three lines then the own-purchase line,
///   or `ASK` when there is none, all info lines;
/// - no current listing (`NO_LISTING`, or `AMBIGUOUS`) and no own line is
///   the returned refusal alone;
/// - no current listing and an own line (a purchase funded before its
///   listing was withdrawn) is the refusal as an ERROR line, then the own
///   line as info, and the command returns `Ok`;
/// - an accept with no live offer is the returned `NO_OFFER`; with live
///   offers at other prices, `NO_OFFER_AT` as an error line then each
///   offer's two lines as info, returning `Ok`.
#[test]
fn multi_line_replies_print_the_refusal_then_the_rest() -> TestResult {
    use super::market::detail_reply;
    use super::trade::{AcceptPlan, Named, NamedOffer, accept_plan};
    use super::{Said, refusal};
    use mush_core::CmdError;
    let code = "feed0001";
    let lines = || ["DETAIL".to_owned(), "TERMS".to_owned(), "SPLIT".to_owned()];
    let info = |text: &str| Said::Info(text.to_owned());
    let escrow = fill(words::YOUR_ESCROW, &[("code", code)]);
    let ask = fill(words::ASK, &[("code", code)]);
    check_eq!(
        detail_reply(Ok(lines()), None, code),
        Ok(vec![
            info("DETAIL"),
            info("TERMS"),
            info("SPLIT"),
            info(&ask)
        ])
    );
    check_eq!(
        detail_reply(Ok(lines()), Some(escrow.clone()), code),
        Ok(vec![
            info("DETAIL"),
            info("TERMS"),
            info("SPLIT"),
            info(&escrow)
        ])
    );
    for refused in [
        fill(words::NO_LISTING, &[("code", code)]),
        fill(words::AMBIGUOUS, &[("code", code)]),
    ] {
        check_eq!(
            detail_reply(Err(CmdError::msg(refused.clone())), None, code),
            Err(CmdError::msg(refused.clone()))
        );
        check_eq!(
            detail_reply(
                Err(CmdError::msg(refused.clone())),
                Some(escrow.clone()),
                code
            ),
            Ok(vec![Said::Error(refused.clone()), info(&escrow)]),
            "the own line after the refusal, never ASK"
        );
    }
    let now = 1_000;
    let live = NamedOffer {
        offer: mush_core::CommitToken::from_bytes([5; 16]),
        price_minor: 21,
        expires_at_utc: now + 600,
    };
    let refused_lines =
        |offers: &[NamedOffer]| match accept_plan(Named::Resale, 7, offers, Some(1), now) {
            AcceptPlan::Refuse(lines) => Ok(lines),
            other => Err(format!("a refusal, not {other:?}")),
        };
    check_eq!(
        refusal(refused_lines(&[live])?),
        Ok(vec![
            Said::Error("None of your offers for licence 7 is at 0.01cr.".to_owned()),
            info(&fill(
                words::named::OFFER,
                &[("licence", "7"), ("price", "0.21cr"), ("minutes", "10")]
            )),
            info("Accept it with +minds/resell/accept 7 price=0.21."),
        ])
    );
    let none = refused_lines(&[])?;
    check_eq!(
        refusal(none),
        Err(CmdError::msg("You have no offer for licence 7."))
    );
    Ok(())
}

/// 17g C2g (the C2e plan §4.1 item 4's edge): a seller naming itself as the
/// buyer reads `NO_PLAYER` with the name as typed (`trade::other_player`,
/// checked after the seller's own record, before any line prints); any
/// other player passes, handed back as the buyer the form submits (the C2g
/// fix round 2, reviewer tests/T-4: `resell` and `charter` take their buyer
/// from it). The dispatched proof is owed to the C2h story's part 3
/// (`trade`'s module doc).
#[test]
fn an_offer_to_oneself_is_no_player() -> TestResult {
    use super::trade::other_player;
    use mush_core::ObjId;
    let me = || (ObjId(7), "Old Cy".to_owned());
    check_eq!(
        other_player(ObjId(7), me(), "me"),
        Err(mush_core::CmdError::msg("No player is named me."))
    );
    check_eq!(
        other_player(ObjId(7), me(), "Old Cy"),
        Err(mush_core::CmdError::msg("No player is named Old Cy."))
    );
    check_eq!(
        other_player(ObjId(7), (ObjId(8), "Cy".to_owned()), "cy"),
        Ok((ObjId(8), "Cy".to_owned()))
    );
    Ok(())
}
