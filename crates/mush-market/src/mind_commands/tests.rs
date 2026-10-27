//! 17g C2c tests 1 and 2: the command read's paging, code lookup and privacy
//! over real listing rows (`mind_entitlement::tests::listing_fixture`, whose
//! frozen product, cuts and royalty chain pass `validate`), through an
//! in-memory listing family that records every scan it is asked for; and the
//! captured entitlement actions.
use super::*;
use crate::mind_entitlement::tests::listing_fixture;
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use std::cell::RefCell;
use std::collections::BTreeMap;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// A listing family in key order, as the typed store scans one: `after`
/// excluded, at most `limit` rows, and whether more follow.
#[derive(Default)]
struct Memory {
    rows: BTreeMap<String, EntitlementListingV1>,
    scans: RefCell<Vec<(Option<String>, usize)>>,
}
impl<'a> ListingSource<'a> for &'a Memory {
    fn scan(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<&'a EntitlementListingV1>, bool), OwnerReadError> {
        self.scans
            .borrow_mut()
            .push((after.map(str::to_owned), limit));
        let mut rows = self
            .rows
            .iter()
            .filter(|(key, _)| after.is_none_or(|after| key.as_str() > after))
            .map(|(_, row)| row);
        let page: Vec<_> = rows.by_ref().take(limit).collect();
        Ok((page, rows.next().is_some()))
    }
}
impl Memory {
    fn add(&mut self, row: EntitlementListingV1) {
        self.rows.insert(row.publication().to_string(), row);
    }
    fn take_scans(&self) -> Vec<(Option<String>, usize)> {
        std::mem::take(&mut *self.scans.borrow_mut())
    }
}
fn token(prefix: [u8; 4], n: u8) -> CommitToken {
    let mut bytes = [n; 16];
    bytes[..4].copy_from_slice(&prefix);
    CommitToken::from_bytes(bytes)
}
fn key(token: Option<CommitToken>) -> Option<String> {
    token.map(|t| t.to_string())
}
fn code(token: CommitToken) -> String {
    token.to_string().chars().take(CODE_CHARACTERS).collect()
}
fn listing(publication: CommitToken) -> TestResult<EntitlementListingV1> {
    listing_fixture(
        EntitlementProtocol::DerivedV1,
        1,
        PaidForm::Open {},
        publication,
    )
}

/// 17g C2c test 1 (C2 plan §2.4, §2.7, §2.8).
///
/// - `Listings` pages 20 current rows at a time in key order, with the last
///   row's token as the cursor, skipping non-current rows, and ends with no
///   cursor; every scan is one bounded page of 64.
/// - A code lookup is one scan of two rows after the code: one current
///   listing is `One`, two tokens sharing the 8 characters are `Ambiguous`,
///   none or a non-current one is `Refused`, and a 7- or 9-character (or
///   upper-case) code is refused before any scan.
/// - The serialized results carry no seller, account id, revision, rating or
///   digest, and the view carries the venue cut, the levy and the parents'
///   royalty.
///
/// - The C2c fix round (reviewer F4): a run of more non-current listings
///   than one read's scan budget (8 scans of 64) ends the page with no rows
///   and the last key scanned as its cursor, not `EMPTY`'s end, and
///   `ListingsAfter` on that cursor's code (a listing no longer current)
///   resolves it in one scan of two rows and pages on to the current ones
///   with the 7 scans left.
///
/// `OwnPurchase` and `OwnOpen` are not built (the per-buyer index C2b did
/// not land), so their own-only cases are not here.
#[test]
fn market_command_reader_is_bounded_and_seller_free() -> TestResult {
    let mut memory = Memory::default();
    // 45 current listings and 5 non-current ones (one of each 10).
    let mut stale = BTreeSet::new();
    for n in 1..=50_u8 {
        let publication = token([n, 0, 0, 0], n);
        if n % 10 == 0 {
            stale.insert(publication);
        }
        memory.add(listing(publication)?);
    }
    let current = |row: &EntitlementListingV1| !stale.contains(&row.publication());
    let source = &memory;

    let first = listings(&source, None, SCANS, current)?;
    check_eq!(first.rows().len(), LISTINGS_PER_PAGE);
    check!(
        first
            .rows()
            .iter()
            .all(|v| !stale.contains(&v.publication()))
    );
    let keys: Vec<_> = first.rows().iter().map(|v| v.publication()).collect();
    let mut sorted = keys.clone();
    sorted.sort();
    check_eq!(&keys, &sorted, "key order");
    check_eq!(
        first.next(),
        keys.last().copied(),
        "the cursor is the last row"
    );
    check_eq!(memory.take_scans(), vec![(None, 64)], "one bounded scan");

    let second = listings(&source, key(first.next()), SCANS, current)?;
    check_eq!(second.rows().len(), LISTINGS_PER_PAGE);
    check!(second.rows()[0].publication() > first.rows()[19].publication());
    let third = listings(&source, key(second.next()), SCANS, current)?;
    check_eq!(third.rows().len(), 5, "45 current rows in all");
    check_eq!(third.next(), None, "the end has no cursor");
    for (_, limit) in memory.take_scans() {
        check_eq!(limit, 64);
    }

    // Code lookups: one, two sharing 8 characters, none, not current.
    let shared = [0xab, 0xcd, 0xef, 0x01];
    memory.add(listing(token(shared, 1))?);
    memory.add(listing(token(shared, 2))?);
    let unique = token([0xfe, 0xed, 0x00, 0x01], 3);
    memory.add(listing(unique)?);
    let source = &memory;
    let lookup = |code: &str| by_code(&source, code, current);
    let one = lookup("feed0001")?;
    check!(matches!(&one, CodeLookup::One(v) if v.publication() == unique));
    check_eq!(
        memory.take_scans(),
        vec![(Some("feed0001".to_owned()), CODE_ROWS)],
        "one scan of two rows after the code"
    );
    check_eq!(lookup("abcdef01")?, CodeLookup::Ambiguous);
    check_eq!(lookup("00000000"), Err(OwnerReadError::Refused), "none");
    let stale_code: String = stale
        .iter()
        .next()
        .ok_or("a stale row")?
        .to_string()
        .chars()
        .take(8)
        .collect();
    check_eq!(
        lookup(&stale_code),
        Err(OwnerReadError::Refused),
        "not current"
    );
    check_eq!(memory.take_scans().len(), 3);
    for code in ["feed000", "feed00011", "FEED0001", "feed000g", ""] {
        check_eq!(lookup(code), Err(OwnerReadError::Refused), "{code:?}");
    }
    check!(
        memory.take_scans().is_empty(),
        "a malformed code reads nothing"
    );
    check!(is_code("feed0001") && !is_code("feed000") && !is_code("feed00011"));

    // A run of 600 stale listings before 3 current ones (reviewer F4).
    let mut long = Memory::default();
    let mut current_keys = BTreeSet::new();
    for n in 0..603_u16 {
        let [hi, lo] = n.to_be_bytes();
        let publication = token([0x10, hi, lo, 0], 1);
        if n >= 600 {
            current_keys.insert(publication);
        }
        long.add(listing(publication)?);
    }
    let live = |row: &EntitlementListingV1| current_keys.contains(&row.publication());
    let source = &long;
    let stalled = listings(&source, None, SCANS, live)?;
    check!(
        stalled.rows().is_empty(),
        "no current listing in the budget"
    );
    let scanned = long.take_scans();
    check_eq!(scanned.len(), SCANS, "the whole budget, 64 rows a scan");
    check!(scanned.iter().all(|(_, limit)| *limit == SCAN_ROWS));
    let cursor = stalled.next().ok_or("a stalled page still goes on")?;
    check_eq!(
        Some(cursor),
        long.rows
            .values()
            .nth(SCANS * SCAN_ROWS - 1)
            .map(EntitlementListingV1::publication),
        "the cursor is the 512th key scanned"
    );
    check!(
        !current_keys.contains(&cursor),
        "a listing no longer current"
    );
    check_eq!(
        by_code(&source, &code(cursor), live),
        Err(OwnerReadError::Refused),
        "the detail of a stale listing is still refused"
    );
    long.take_scans();
    let rest = listings_after(&source, &code(cursor), live)?;
    check_eq!(
        rest.rows()
            .iter()
            .map(PublicListingView::publication)
            .collect::<BTreeSet<_>>(),
        current_keys,
        "the current listings after the run"
    );
    check_eq!(rest.next(), None);
    let scanned = long.take_scans();
    check_eq!(
        scanned.first(),
        Some(&(Some(code(cursor)), CODE_ROWS)),
        "the cursor resolved in one scan of two rows"
    );
    check!(scanned.len() <= SCANS, "{} scans", scanned.len());
    check!(
        scanned.iter().skip(1).all(|(_, limit)| *limit == SCAN_ROWS),
        "then whole scans"
    );
    check_eq!(
        listings_after(&source, "0fffffff", live),
        Err(OwnerReadError::Refused),
        "no listing starts with the code"
    );
    for malformed in ["1000000", "100000000", "FFFFFFFF"] {
        check_eq!(
            listings_after(&source, malformed, live),
            Err(OwnerReadError::Refused)
        );
    }

    // The view: every public term, the venue cut included, and no private one.
    let CodeLookup::One(view) = one else {
        return Err("one listing".into());
    };
    check_eq!(view.code(), "feed0001");
    check_eq!(view.protocol(), Protocol::Derived);
    check!(view.derived());
    check_eq!(view.form(), ListingForm::Open);
    check_eq!(view.class(), RatingClass::II);
    check!(
        !view.transferable(),
        "the fixture's frozen terms are marked"
    );
    check_eq!(
        (view.term_seconds(), view.grace_seconds()),
        (900, 90),
        "the frozen product's term and Grace"
    );
    check_eq!(view.price_minor(), 400);
    check_eq!(
        (
            view.levy_permille(),
            view.venue_permille(),
            view.parent_royalty_permille()
        ),
        (50, 20, 100),
        "levy, venue cut and parents"
    );
    let page = listings(&source, None, SCANS, |_| true)?;
    for value in [
        serde_json::to_value(MarketCommandResult::ListingByCode(CodeLookup::One(
            view.clone(),
        )))?,
        serde_json::to_value(MarketCommandResult::Listings(page))?,
    ] {
        let text = value.to_string();
        for private in [
            "seller", "author", "account", "revision", "rating", "digest",
        ] {
            check!(!text.contains(private), "{private} in {text}");
        }
        // Every digest is 64 lowercase characters; a token is 32.
        let longest = text
            .split(|c: char| !(c.is_ascii_digit() || ('a'..='f').contains(&c)))
            .map(str::len)
            .max()
            .unwrap_or(0);
        check!(longest < 64, "a digest-length run in {text}");
        // The fixture's accounts (9, 11, 12) and seller (7) as numbers.
        for id in [7, 9, 11, 12] {
            check!(!text.contains(&format!(":{id},")) && !text.contains(&format!(":{id}}}")));
        }
    }
    check_eq!(
        serde_json::to_value(&view)?
            .as_object()
            .ok_or("an object")?
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        [
            "publication",
            "protocol",
            "charter",
            "class",
            "form",
            "transferable",
            "term_seconds",
            "grace_seconds",
            "price_minor",
            "levy_permille",
            "venue_permille",
            "parent_royalty_permille",
            "derived",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>(),
        "exactly the public fields"
    );
    Ok(())
}

/// 17g C2e test 6 (C2e plan §2.1 item 4a, review FE-F13): `by_code` counts
/// only CURRENT rows toward ambiguity, within its unchanged window of
/// `CODE_ROWS` rows.
///
/// - A row no longer current (a listing a policy change left behind) sharing
///   the code with one current row resolves to the current row, whichever
///   of the two sorts first, and whether the family goes on past them or
///   ends with them.
/// - Two current rows sharing the code are `Ambiguous`.
/// - A stale exact match with no current row is `Refused`, the not-found
///   answer.
/// - The stated residual: a current listing that sits beyond two stale rows
///   sharing its code is outside the two-row window and still not found
///   (`CODE_ROWS` is in the read's descriptor and stays 2).
/// - A `One` is never a guess (the C2e fix round, reviewer correctness/F1):
///   when the window is two rows sharing the code, one current, and the
///   family goes on, the one row after the window is probed, and a third
///   row sharing the code makes the answer `Ambiguous` (stale, current,
///   current; current, stale, current; and, since the read cannot see past
///   it, stale, current, stale).
///
/// Every lookup is one scan of two rows after the code, plus that one-row
/// probe after the window's last key only when the window is two rows
/// sharing the code with exactly one current and more rows follow.
#[test]
fn by_code_counts_only_current_rows_for_ambiguity() -> TestResult {
    let mut memory = Memory::default();
    let mut stale = BTreeSet::new();
    // Each prefix's rows by ordinal: `true` is current.
    let families: [([u8; 4], &[bool]); 9] = [
        // Stale first, then current; the family goes on.
        ([0xaa, 0, 0, 1], &[false, true]),
        // Current first, then stale; the family goes on.
        ([0xbb, 0, 0, 1], &[true, false]),
        // Two current.
        ([0xcc, 0, 0, 1], &[true, true]),
        // A stale exact match alone.
        ([0xdd, 0, 0, 1], &[false]),
        // Stale, current, current: the second current is past the window.
        ([0xe0, 0, 0, 1], &[false, true, true]),
        // Current, stale, current.
        ([0xe1, 0, 0, 1], &[true, false, true]),
        // Stale, current, stale: past the window the read cannot tell.
        ([0xe2, 0, 0, 1], &[false, true, false]),
        // Two stale rows before a current one (the residual).
        ([0xee, 0, 0, 1], &[false, false, true]),
        // Stale, then current, and the family ends with them.
        ([0xff, 0, 0, 1], &[false, true]),
    ];
    for (prefix, rows) in families {
        for (n, is_current) in (1..).zip(rows.iter()) {
            let publication = token(prefix, n);
            if !is_current {
                stale.insert(publication);
            }
            memory.add(listing(publication)?);
        }
    }

    let current = |row: &EntitlementListingV1| !stale.contains(&row.publication());
    let source = &memory;
    let lookup = |code: &str| -> (Result<Option<CommitToken>, OwnerReadError>, _) {
        let found = by_code(&source, code, current).map(|found| match found {
            CodeLookup::One(view) => Some(view.publication()),
            CodeLookup::Ambiguous => None,
        });
        (found, memory.take_scans())
    };
    let window = |code: &str| (Some(code.to_owned()), CODE_ROWS);
    let probe = |prefix: [u8; 4], n: u8| (key(Some(token(prefix, n))), 1);

    for (code, expected, scans, why) in [
        (
            "aa000001",
            Ok(Some(token([0xaa, 0, 0, 1], 2))),
            vec![window("aa000001"), probe([0xaa, 0, 0, 1], 2)],
            "a stale row before the current one",
        ),
        (
            "bb000001",
            Ok(Some(token([0xbb, 0, 0, 1], 1))),
            vec![window("bb000001"), probe([0xbb, 0, 0, 1], 2)],
            "a stale row after the current one",
        ),
        (
            "cc000001",
            Ok(None),
            vec![window("cc000001")],
            "two current",
        ),
        (
            "dd000001",
            Err(OwnerReadError::Refused),
            vec![window("dd000001")],
            "a stale exact match alone is not found",
        ),
        (
            "e0000001",
            Ok(None),
            vec![window("e0000001"), probe([0xe0, 0, 0, 1], 2)],
            "stale, current, current: never the first current alone",
        ),
        (
            "e1000001",
            Ok(None),
            vec![window("e1000001"), probe([0xe1, 0, 0, 1], 2)],
            "current, stale, current",
        ),
        (
            "e2000001",
            Ok(None),
            vec![window("e2000001"), probe([0xe2, 0, 0, 1], 2)],
            "a third row with the code past the window, current or not",
        ),
        (
            "ee000001",
            Err(OwnerReadError::Refused),
            vec![window("ee000001")],
            "the residual: the current row is beyond the two-row window",
        ),
        (
            "ff000001",
            Ok(Some(token([0xff, 0, 0, 1], 2))),
            vec![window("ff000001")],
            "the family ends with the pair: no probe",
        ),
    ] {
        check_eq!(lookup(code), (expected, scans), "{code}: {why}");
    }
    Ok(())
}

/// 17g C2c test 2: each step of each protocol is exactly the market's own
/// registered action (`<step>_<protocol>_mind`, fund `purchase_`, grant
/// `settle_`; original v3, derived v1). Another owner's same id, another
/// version or kind is absent, as is every step of an empty composition.
#[test]
fn market_command_actions_capture_the_entitlement_steps() -> TestResult {
    let mut builder = DefinitionRegistryBuilder::new();
    let mut add = |owner: &str, kind: &str, id: &str, version: u32| -> TestResult<DefinitionRef> {
        Ok(builder.register(
            OwnerId::new(owner)?,
            DefinitionDescriptor::new(
                DefinitionKind::new(kind)?,
                DefinitionId::new(id)?,
                version,
                serde_json::json!({"fixture":"identity only"}),
            )?,
        )?)
    };
    let mut expected = Vec::new();
    for (product, version, protocol) in [
        ("original", 3, Protocol::Original),
        ("derived", 1, Protocol::Derived),
    ] {
        for (id, step) in [
            ("list", Step::List),
            ("quote", Step::Quote),
            ("purchase", Step::Fund),
            ("settle", Step::Grant),
            ("refund", Step::Refund),
        ] {
            let name = format!("{id}_{product}_mind");
            expected.push((protocol, step, add("market", "action", &name, version)?));
            // Decoys: Roll's same id, another version, an outcome.
            add("roll", "action", &name, version)?;
            add("market", "action", &name, version + 1)?;
            add("market", "outcome", &name, version)?;
        }
    }
    // v2's original listing is not the v3 step.
    add("market", "action", "list_original_mind", 2)?;
    let registry = builder.freeze()?;
    let actions = MarketCommandActions::capture(&registry);
    for (protocol, step, reference) in expected {
        check_eq!(
            actions.entitlement(protocol, step),
            Some(reference),
            "{protocol:?} {step:?}"
        );
    }
    let empty = MarketCommandActions::capture(&DefinitionRegistryBuilder::new().freeze()?);
    check_eq!(&empty, &MarketCommandActions::default());
    check_eq!(empty.entitlement(Protocol::Original, Step::List), None);
    Ok(())
}

/// 17g C2d test 3: each paid-resale step is exactly the market's own
/// registered action (`offer_`, `accept_` and `fund_paid_mind_resale`,
/// version 1, `mush_roll::commerce::paid_resale::protocol`'s ids); Roll's
/// same id, another version, an outcome and the service's settle and refund
/// steps are not; an empty composition has none.
#[test]
fn market_command_actions_capture_the_resale_kinds() -> TestResult {
    use mush_roll::commerce::paid_resale::protocol;
    let mut builder = DefinitionRegistryBuilder::new();
    let mut add = |owner: &str, kind: &str, id: &str, version: u32| -> TestResult<DefinitionRef> {
        Ok(builder.register(
            OwnerId::new(owner)?,
            DefinitionDescriptor::new(
                DefinitionKind::new(kind)?,
                DefinitionId::new(id)?,
                version,
                serde_json::json!({"fixture":"identity only"}),
            )?,
        )?)
    };
    let mut expected = Vec::new();
    for (id, kind) in [
        (protocol::OFFER, ResaleKind::Offer),
        (protocol::ACCEPT, ResaleKind::Accept),
        (protocol::FUND, ResaleKind::Fund),
    ] {
        expected.push((kind, add("market", "action", id, 1)?));
        add("roll", "action", id, 1)?;
        add("market", "action", id, 2)?;
        add("market", "outcome", id, 1)?;
    }
    // The service-only terminals are no command's step.
    add("market", "action", protocol::SETTLE, 1)?;
    add("market", "action", protocol::REFUND, 1)?;
    let registry = builder.freeze()?;
    let actions = MarketCommandActions::capture(&registry);
    for (kind, reference) in &expected {
        check_eq!(actions.resale(*kind), Some(reference.clone()), "{kind:?}");
    }
    // The three are distinct actions.
    check!(
        actions.resale(ResaleKind::Offer) != actions.resale(ResaleKind::Accept)
            && actions.resale(ResaleKind::Accept) != actions.resale(ResaleKind::Fund)
    );
    // The entitlement steps are not captured from resale ids.
    check_eq!(actions.entitlement(Protocol::Original, Step::Fund), None);
    let empty = MarketCommandActions::capture(&DefinitionRegistryBuilder::new().freeze()?);
    for kind in [ResaleKind::Offer, ResaleKind::Accept, ResaleKind::Fund] {
        check_eq!(empty.resale(kind), None, "{kind:?}");
    }
    Ok(())
}

/// 17g C2c (the C2c fix round, reviewers F5 and T-F3): the registered read's
/// own answer (`answer`, which `MarketCommandReader::read` calls with the
/// owner lens and the venue gate), over an in-memory listing family:
/// - `AtCounter` reports the venue gate both ways;
/// - `Listings` and `ListingsAfter` away from a counter are `Refused` before
///   any scan;
/// - "current" is the deployment's `current_policy`: a listing at an older
///   price (and so an older policy reference) is not listed, cannot be looked
///   up by its code, and only serves as a cursor;
/// - `ListingByCode` needs no counter.
///
/// The owner lens itself (`Lens::scan`'s key check, the real venue gate) is
/// read through the composed grant on the format-2 Active launch since 17g
/// C2e, by mush-server's `player_market_commands` tests
/// `listings_are_identical_for_every_principal_at_the_counter` and
/// `market_commands_refuse_without_own_records` (the C2c brief's tests 7 and
/// 8).
#[test]
fn market_command_read_answers_the_counter_rule_and_the_policy() -> TestResult {
    use crate::mind_entitlement::tests::{command_config, current_listing_fixture};
    let config = command_config(400)?;
    let older = command_config(300)?;
    let mut memory = Memory::default();
    let listed = token([0x20, 0, 0, 1], 1);
    let repriced = token([0x10, 0, 0, 1], 1);
    memory.add(current_listing_fixture(&config, listed)?);
    memory.add(current_listing_fixture(&older, repriced)?);
    check!(
        crate::mind_entitlement::current_policy(
            &config,
            &current_listing_fixture(&older, repriced)?
        )
        .is_err()
    );
    let source = &memory;
    let ask = |here: bool, request| answer(&source, || here, &config, request);

    check_eq!(
        ask(true, MarketCommandRequest::at_counter())?.at_counter(),
        Some(true)
    );
    check_eq!(
        ask(false, MarketCommandRequest::at_counter())?.at_counter(),
        Some(false)
    );
    check_eq!(
        ask(false, MarketCommandRequest::listings(None)),
        Err(OwnerReadError::Refused)
    );
    check_eq!(
        ask(false, MarketCommandRequest::listings_after(&code(repriced))),
        Err(OwnerReadError::Refused)
    );
    check!(memory.take_scans().is_empty(), "refused before any scan");

    let page = ask(true, MarketCommandRequest::listings(None))?
        .into_listings()
        .ok_or("a page")?;
    check_eq!(
        page.rows()
            .iter()
            .map(PublicListingView::publication)
            .collect::<Vec<_>>(),
        vec![listed],
        "the older-price listing is not current"
    );
    check_eq!(
        page.rows().first().map(PublicListingView::price_minor),
        Some(400)
    );
    check_eq!(page.next(), None);
    let after = ask(true, MarketCommandRequest::listings_after(&code(repriced)))?
        .into_listings()
        .ok_or("a page")?;
    check_eq!(
        after
            .rows()
            .iter()
            .map(PublicListingView::publication)
            .collect::<Vec<_>>(),
        vec![listed],
        "a stale listing still serves as a cursor"
    );
    check_eq!(
        ask(true, MarketCommandRequest::listing_by_code(&code(repriced))),
        Err(OwnerReadError::Refused),
        "nor is it found by its code"
    );
    memory.take_scans();
    let found = ask(false, MarketCommandRequest::listing_by_code(&code(listed)))?
        .into_listing()
        .ok_or("a lookup")?;
    check!(
        matches!(&found, CodeLookup::One(view) if view.publication() == listed),
        "no counter needed for a code"
    );
    check_eq!(memory.take_scans(), vec![(Some(code(listed)), CODE_ROWS)]);
    Ok(())
}
