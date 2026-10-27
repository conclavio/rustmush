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
/// `SPLIT` under 100 with both optional cuts, `HOLD_OWNER` 80, `HOLD_PAID`
/// 114 (with C2d's `; no resale; renewal offered`, the longest legal pair)
/// and `HOLD_WAIT` 96; any line over 120 fails rather than wraps. The words
/// still pending the indexes (`words::pending`) are measured too.
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
    // The pending paid row: every legal flag combination (a non-transferable
    // or revoked licence is never selling; a revoked one is not renewed; one
    // renewal state at a time).
    let name = long.chars().take(20).collect::<String>();
    let mut widest = 0;
    for kind in ["paid", "House Pin"] {
        for flags in [
            "",
            "; no resale",
            "; revoked",
            "; selling",
            "; no resale; revoked",
            "; renewal offered",
            "; renewal paid",
            "; no resale; renewal offered",
            "; no resale; renewal paid",
            "; selling; renewal offered",
            "; selling; renewal paid",
        ] {
            let row = fill(
                words::pending::HOLD_PAID,
                &[
                    ("licence", "99999999"),
                    ("kind", kind),
                    ("name", &name),
                    ("form", "open-resale"),
                    ("phase", "Active, 90 days left"),
                    ("flags", flags),
                ],
            );
            check!(width(&row) <= 120, "{row}");
            widest = widest.max(width(&row));
        }
    }
    check_eq!(widest, 114, "HOLD_PAID at the maxima");
    let wait = fill(words::pending::HOLD_WAIT, &[("code", "feed0001")]);
    check_eq!(width(&wait), 96, "{wait}");
    // Every other sentence at its longest fill.
    let code = [("code", "feed0001")];
    for line in [
        fill(words::MORE, &code),
        fill(words::NO_LISTING, &code),
        fill(words::AMBIGUOUS, &code),
        fill(words::QUOTING, &code),
        fill(
            words::PUBLISHING,
            &[("filed", "99999999"), ("form", "open-resale")],
        ),
        fill(words::TERMS, &[("price", "999999.99cr"), ("term", "90")]),
        fill(
            words::HOLD_MORE,
            &[("section", "owner"), ("cursor", "4294967295")],
        ),
        fill(
            words::pending::YOUR_QUOTE,
            &[
                ("price", "999999.99cr"),
                ("seconds", "600"),
                ("code", "feed0001"),
            ],
        ),
        fill(words::pending::QUOTE_EXPIRED, &code),
        fill(words::pending::YOUR_ESCROW, &code),
        fill(words::pending::ASK, &code),
        fill(words::pending::NO_QUOTE, &code),
        fill(words::pending::PAYING, &code),
        fill(words::pending::NO_PURCHASE, &code),
        fill(words::pending::COMPLETING, &code),
        fill(words::pending::REFUNDING, &code),
        words::pending::HOLD_WAIT_HEAD.to_owned(),
        words::pending::HOLD_EMPTY.to_owned(),
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
        ("12 form=open seats=0", R::Usage),
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
                .any(|l| l == words::EMPTY || l == words::pending::HOLD_EMPTY)
        );
    }
    Ok(())
}
