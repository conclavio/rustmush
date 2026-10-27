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
