//! 17g C2c tests 1 and 2: the command read's paging, code lookup and privacy
//! over real listing rows (`mind_entitlement::tests::listing_fixture`, whose
//! frozen product, cuts and royalty chain pass `validate`), through an
//! in-memory listing family that records every scan it is asked for; and the
//! captured entitlement actions. 17g C2g: the own kinds over real quote,
//! purchase and resale offer rows (`command_quote`, `command_purchase`,
//! `command_offer`) and real index rows, through in-memory point reads that
//! record every read they are asked for.
use super::*;
use crate::mind_entitlement::tests::listing_fixture;
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use std::cell::RefCell;
use std::collections::BTreeMap;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// A listing family in key order, as the typed store scans one: `after`
/// excluded, at most `limit` rows, and whether more follow. 17g C2g: and the
/// own kinds' families, read by key, each read recorded as `(family, rows)`;
/// `step` is what Roll's claim census answers for every licence, and
/// `steps` counts the times it is asked.
#[derive(Default)]
struct Memory {
    rows: BTreeMap<String, EntitlementListingV1>,
    scans: RefCell<Vec<(Option<String>, usize)>>,
    actors: BTreeMap<ObjId, EntitlementActorIndex>,
    /// M8c S1: the organisations' rows and who acts for each.
    orgs: BTreeMap<mush_org::OrgId, EntitlementActorIndex>,
    members: BTreeSet<(mush_org::OrgId, ObjId)>,
    quotes: BTreeMap<CommitToken, EntitlementQuoteV1>,
    operations: BTreeMap<CommitToken, EntitlementPurchaseV1>,
    buyer_offers: BTreeMap<(ObjId, LicenceId), ResaleBuyerOffers>,
    offers: BTreeMap<CommitToken, MindPaidResaleOffer>,
    step: Option<ResaleKind>,
    reads: RefCell<Vec<(&'static str, usize)>>,
    steps: RefCell<usize>,
}
impl<'a> OwnSource<'a> for &'a Memory {
    fn actor_row(&self, actor: ObjId) -> Result<Option<&'a EntitlementActorIndex>, OwnerReadError> {
        self.reads.borrow_mut().push(("actors", 1));
        Ok(self.actors.get(&actor))
    }
    fn org_row(
        &self,
        org: mush_org::OrgId,
    ) -> Result<Option<&'a EntitlementActorIndex>, OwnerReadError> {
        self.reads.borrow_mut().push(("orgs", 1));
        Ok(self.orgs.get(&org))
    }
    fn acts_for(&self, org: mush_org::OrgId, actor: ObjId) -> Result<(), OwnerReadError> {
        if self.members.contains(&(org, actor)) {
            Ok(())
        } else {
            Err(OwnerReadError::Refused)
        }
    }
    fn quotes(
        &self,
        tokens: &[CommitToken],
    ) -> Result<Vec<Option<&'a EntitlementQuoteV1>>, OwnerReadError> {
        if !tokens.is_empty() {
            self.reads.borrow_mut().push(("quotes", tokens.len()));
        }
        Ok(tokens.iter().map(|t| self.quotes.get(t)).collect())
    }
    fn operations(
        &self,
        tokens: &[CommitToken],
    ) -> Result<Vec<Option<&'a EntitlementPurchaseV1>>, OwnerReadError> {
        if !tokens.is_empty() {
            self.reads.borrow_mut().push(("operations", tokens.len()));
        }
        Ok(tokens.iter().map(|t| self.operations.get(t)).collect())
    }
    fn buyer_offers(
        &self,
        buyer: ObjId,
        licence: LicenceId,
    ) -> Result<Option<&'a ResaleBuyerOffers>, OwnerReadError> {
        self.reads.borrow_mut().push(("buyer_offers", 1));
        Ok(self.buyer_offers.get(&(buyer, licence)))
    }
    fn resale_offers(
        &self,
        tokens: &[CommitToken],
    ) -> Result<Vec<Option<&'a MindPaidResaleOffer>>, OwnerReadError> {
        if !tokens.is_empty() {
            self.reads.borrow_mut().push(("offers", tokens.len()));
        }
        Ok(tokens.iter().map(|t| self.offers.get(t)).collect())
    }
    fn resale_step(&self, _: LicenceId) -> Result<ResaleKind, OwnerReadError> {
        *self.steps.borrow_mut() += 1;
        self.step.ok_or(OwnerReadError::Unavailable)
    }
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
    fn take_reads(&self) -> Vec<(&'static str, usize)> {
        std::mem::take(&mut *self.reads.borrow_mut())
    }
}
/// Whom the in-memory reads answer for, at world UTC second 1,000.
fn own(actor: u32) -> Own {
    Own {
        actor: ObjId(actor),
        now: 1_000,
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
///   listing is `One`, two CURRENT listings sharing the 8 characters are
///   `Ambiguous` (17g C2e: only current rows count, `by_code_counts_only_
///   current_rows_for_ambiguity`), none or a non-current one is `Refused`,
///   and a 7- or 9-character (or
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
/// 17g C2g adds the own kinds (C2c test 1's owed cases, the C2e plan §4.1
/// item 1), through `answer`, over the actor's real index row and real quote
/// and purchase rows:
/// - `OwnPurchase { code }` answers the NEWEST indexed quote on the listing
///   the code names, expired or not (the read takes no clock), beside the
///   funded purchase on it, and a funded purchase whose listing is gone (no
///   listing row at all: withdrawn after Fund) is still found;
/// - it is own-only: another actor's quote and purchase on the same listing
///   are absent, and a code only another actor holds a purchase on answers
///   byte-identically to a code nobody holds one on;
/// - it is bounded: one read of the actor's own index row, then one batched
///   read of the quotes and one of the operations the row names (at most
///   `MAX_BUYER_QUOTES` + `MAX_BUYER_PURCHASES` rows); a malformed code reads
///   nothing; an actor with no row reads one row;
/// - `OwnOpen` lists the actor's funded purchases only, with their codes;
/// - an index row naming another buyer's quote is `Integrity`;
/// - neither result carries the seller, an account, a digest or another
///   buyer.
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
    own_kinds_are_own_only_and_bounded()
}

/// A quote by `buyer` on `listing` (`command_quote`), published as the
/// token of `n`, inserted into `memory`; its token. Apart from the tests so
/// their frames stay under the M7 memory-size limit.
fn quote_into(
    memory: &mut Memory,
    n: u8,
    listing: CommitToken,
    buyer: ObjId,
    quoted: i64,
    expires: i64,
) -> TestResult<CommitToken> {
    let quote = Box::new(crate::mind_entitlement::tests::command_quote(
        token([n, 0, 0, 0], 1),
        listing,
        buyer,
        quoted,
        expires,
    )?);
    let publication = quote.publication();
    memory.quotes.insert(publication, *quote);
    Ok(publication)
}
/// A funded purchase by `buyer` on `listing` (its quote, the token of `n`,
/// quoted at `quoted`, consumed; its operation the token of `n` + 0x10),
/// inserted into `memory`; its operation token.
fn purchase_into(
    memory: &mut Memory,
    n: u8,
    listing: CommitToken,
    buyer: ObjId,
    quoted: i64,
) -> TestResult<CommitToken> {
    use crate::mind_entitlement::tests::{command_purchase, command_quote};
    let quote = command_quote(token([n, 0, 0, 0], 1), listing, buyer, quoted, quoted + 60)?;
    let purchase = Box::new(command_purchase(
        quote,
        token([n.wrapping_add(0x10), 0, 0, 0], 2),
    )?);
    let operation = purchase.operation();
    memory.operations.insert(operation, *purchase);
    Ok(operation)
}

/// The own kinds' half of `market_command_reader_is_bounded_and_seller_free`
/// (its doc), apart from it so each test's frame stays small. The C2g fix
/// round 2 adds (reviewers tests/T-2 and tests/T-3): every own kind answers
/// the same away from a counter (`here` false) as at one, so
/// `+minds/buy/refund` and `+minds <code>`'s own line work anywhere; and no
/// own kind ever scans the listing family (`by_code` included): their reads
/// are the index row and the rows it names, nothing else.
fn own_kinds_are_own_only_and_bounded() -> TestResult {
    use crate::mind_entitlement::tests::command_index;
    let config = crate::mind_entitlement::tests::command_config(400)?;
    let (me, other) = (ObjId(8), ObjId(9));
    let listed = token([0x30, 0, 0, 1], 1);
    let elsewhere = token([0x31, 0, 0, 1], 1);
    let withdrawn = token([0x32, 0, 0, 1], 1);
    let theirs = token([0x33, 0, 0, 1], 1);
    let mut memory = Memory::default();
    memory.add(listing(listed)?);
    memory.add(listing(elsewhere)?);
    // Mine: an older and a newer quote on `listed` (both past the read's
    // clock of 1,000: expired), a quote on `elsewhere`, a funded purchase
    // on `listed` and one on `withdrawn` (no listing row).
    let older = quote_into(&mut memory, 0xa0, listed, me, 100, 160)?;
    let newer = quote_into(&mut memory, 0xa1, listed, me, 120, 180)?;
    let aside = quote_into(&mut memory, 0xa2, elsewhere, me, 130, 190)?;
    let paid = purchase_into(&mut memory, 0xa3, listed, me, 90)?;
    let gone = purchase_into(&mut memory, 0xa4, withdrawn, me, 90)?;
    // Another buyer's quote on `listed` and purchase on `theirs`.
    let their_quote = quote_into(&mut memory, 0xa5, listed, other, 140, 700)?;
    let their_paid = purchase_into(&mut memory, 0xa6, theirs, other, 90)?;
    memory.actors.insert(
        me,
        command_index(me, &[older, newer, aside], &[paid, gone])?,
    );
    memory
        .actors
        .insert(other, command_index(other, &[their_quote], &[their_paid])?);
    let source = &memory;
    let ask = |actor: u32, request| answer(&source, own(actor), || true, &config, request);

    let mine = ask(8, MarketCommandRequest::own_purchase(&code(listed)))?
        .into_own_purchase()
        .ok_or("an own-purchase answer")?
        .ok_or("my record")?;
    check_eq!(mine.code(), code(listed));
    check_eq!(mine.protocol(), Protocol::Original);
    let quote = mine.quote().ok_or("my quote")?;
    check_eq!(quote.quote(), newer, "the newest quote");
    check_eq!(
        (quote.price_minor(), quote.expires_at_utc()),
        (400, 180),
        "even though it has expired"
    );
    check_eq!(mine.funded(), Some(paid));
    check_eq!(
        mine.latest(),
        newer,
        "the newest quote outranks an older Fund"
    );
    check_eq!(
        memory.take_reads(),
        vec![("actors", 1), ("quotes", 3), ("operations", 2)],
        "one index read, then one batched read each"
    );
    let gone_view = ask(8, MarketCommandRequest::own_purchase(&code(withdrawn)))?
        .into_own_purchase()
        .ok_or("an answer")?
        .ok_or("the withdrawn listing's purchase")?;
    check_eq!(
        (gone_view.quote(), gone_view.funded(), gone_view.latest()),
        (None, Some(gone), gone),
        "found through the index, with no listing row"
    );
    // Own-only: another buyer's purchase reads as none, byte for byte.
    let foreign = ask(8, MarketCommandRequest::own_purchase(&code(theirs)))?;
    let none = ask(8, MarketCommandRequest::own_purchase("0fffffff"))?;
    check_eq!(foreign.clone().into_own_purchase(), Some(None));
    check_eq!(
        serde_json::to_string(&foreign)?,
        serde_json::to_string(&none)?,
        "a foreign purchase is the same answer as none"
    );
    let their_view = ask(9, MarketCommandRequest::own_purchase(&code(listed)))?
        .into_own_purchase()
        .ok_or("an answer")?
        .ok_or("their record")?;
    check_eq!(
        (their_view.quote().map(|q| q.quote()), their_view.funded()),
        (Some(their_quote), None),
        "each actor reads its own quote on one listing"
    );
    memory.take_reads();
    check_eq!(
        ask(10, MarketCommandRequest::own_purchase(&code(listed)))?.into_own_purchase(),
        Some(None)
    );
    check_eq!(memory.take_reads(), vec![("actors", 1)], "no row, one read");
    for malformed in ["3000000", "300000011", "3000000G"] {
        check_eq!(
            ask(8, MarketCommandRequest::own_purchase(malformed)),
            Err(OwnerReadError::Refused)
        );
    }
    check!(
        memory.take_reads().is_empty(),
        "a malformed code reads nothing"
    );
    check!(
        memory.take_scans().is_empty(),
        "OwnPurchase never scans the listings (no `by_code`)"
    );

    // OwnOpen: the actor's funded purchases, with their codes.
    let open = ask(8, MarketCommandRequest::own_open())?
        .into_own_open()
        .ok_or("an open answer")?;
    check_eq!(
        open.iter()
            .map(|v| (v.operation(), v.code().to_owned(), v.protocol()))
            .collect::<Vec<_>>(),
        vec![
            (paid, code(listed), Protocol::Original),
            (gone, code(withdrawn), Protocol::Original),
        ]
    );
    check_eq!(
        ask(9, MarketCommandRequest::own_open())?
            .into_own_open()
            .map(|open| open.iter().map(OwnOpenView::operation).collect::<Vec<_>>()),
        Some(vec![their_paid])
    );
    check_eq!(
        ask(10, MarketCommandRequest::own_open())?.into_own_open(),
        Some(Vec::new())
    );
    check!(
        memory.take_scans().is_empty(),
        "OwnOpen never scans the listings"
    );

    // Away from a counter every own kind answers exactly as at one.
    let away = |actor: u32, request| answer(&source, own(actor), || false, &config, request);
    let codes = [code(listed), code(withdrawn), code(theirs)];
    for (actor, which) in [
        (8, Some(&codes[0])),
        (8, Some(&codes[1])),
        (8, Some(&codes[2])),
        (8, None),
        (9, None),
    ] {
        let request = || {
            which.map_or_else(MarketCommandRequest::own_open, |c| {
                MarketCommandRequest::own_purchase(c)
            })
        };
        check_eq!(
            away(actor, request()),
            ask(actor, request()),
            "an own kind needs no counter"
        );
    }
    memory.take_reads();
    check!(
        memory.take_scans().is_empty(),
        "nor scans the listings away from one"
    );

    // Privacy of every own answer.
    for value in [
        serde_json::to_value(ask(8, MarketCommandRequest::own_purchase(&code(listed)))?)?,
        serde_json::to_value(ask(8, MarketCommandRequest::own_open())?)?,
    ] {
        let text = value.to_string();
        for private in ["seller", "account", "payer", "digest", "buyer", "author"] {
            check!(!text.contains(private), "{private} in {text}");
        }
        for id in [7, 9, 11, 12] {
            check!(!text.contains(&format!(":{id},")) && !text.contains(&format!(":{id}}}")));
        }
    }

    // An index row naming another buyer's quote is corrupt.
    memory
        .actors
        .insert(me, command_index(me, &[their_quote], &[])?);
    let source = &memory;
    check_eq!(
        answer(
            &source,
            own(8),
            || true,
            &config,
            MarketCommandRequest::own_purchase(&code(listed))
        ),
        Err(OwnerReadError::Integrity)
    );
    Ok(())
}

/// 17g C3a (the C3 plan §2.3, §4.3): `OwnQuotes`, the `mind.market` panel's
/// own strip, beside `OwnPurchase` and `OwnOpen`, through `answer` over the
/// actor's real index row and real quote rows:
/// - every quote on the actor's own row is listed, live or expired, newest
///   first by `quoted_at_utc` and then by token (two quotes of one second:
///   the larger token first), each with its listing's code, frozen price and
///   expiry; a funded purchase is not a quote and is not listed;
/// - it is own-only: another buyer's quote on the same listing is absent
///   from the actor's answer and is the only row of theirs; an actor with no
///   index row answers the empty list after one read;
/// - it is bounded and scans nothing: one read of the index row, then one
///   batched read each of the quotes and operations it names, and it answers
///   the same away from a counter;
/// - its serialized answer carries no token, seller, account, buyer or
///   digest.
#[test]
fn own_quotes_list_the_actors_quotes_newest_first() -> TestResult {
    use crate::mind_entitlement::tests::command_index;
    let config = crate::mind_entitlement::tests::command_config(400)?;
    let (me, other) = (ObjId(8), ObjId(9));
    let listed = token([0x50, 0, 0, 1], 1);
    let elsewhere = token([0x51, 0, 0, 1], 1);
    let mut memory = Memory::default();
    memory.add(listing(listed)?);
    memory.add(listing(elsewhere)?);
    let older = quote_into(&mut memory, 0xc0, listed, me, 100, 160)?;
    let newer = quote_into(&mut memory, 0xc1, listed, me, 120, 680)?;
    let aside = quote_into(&mut memory, 0xc2, elsewhere, me, 130, 190)?;
    let twin = quote_into(&mut memory, 0xc7, elsewhere, me, 130, 690)?;
    let paid = purchase_into(&mut memory, 0xc3, listed, me, 90)?;
    let theirs = quote_into(&mut memory, 0xc5, listed, other, 140, 700)?;
    memory.actors.insert(
        me,
        command_index(me, &[older, newer, aside, twin], &[paid])?,
    );
    memory
        .actors
        .insert(other, command_index(other, &[theirs], &[])?);
    let source = &memory;
    let ask = |actor: u32, here: bool| {
        answer(
            &source,
            own(actor),
            || here,
            &config,
            MarketCommandRequest::own_quotes(),
        )
    };
    let rows = |quotes: &[OwnQuoteRowView]| -> Vec<(String, i64, i64)> {
        quotes
            .iter()
            .map(|q| (q.code().to_owned(), q.price_minor(), q.expires_at_utc()))
            .collect()
    };
    let mine = ask(8, true)?
        .into_own_quotes()
        .ok_or("an own-quotes answer")?;
    check_eq!(
        rows(&mine),
        vec![
            (code(elsewhere), 400, 690),
            (code(elsewhere), 400, 190),
            (code(listed), 400, 680),
            (code(listed), 400, 160),
        ],
        "newest first, the larger token first within one second, expired included"
    );
    check_eq!(
        memory.take_reads(),
        vec![("actors", 1), ("quotes", 4), ("operations", 1)],
        "one index read, then one batched read each"
    );
    check_eq!(
        ask(8, false)?,
        ask(8, true)?,
        "no counter is needed: the same answer away from one"
    );
    let their_rows = ask(9, true)?
        .into_own_quotes()
        .ok_or("their own-quotes answer")?;
    check_eq!(rows(&their_rows), vec![(code(listed), 400, 700)]);
    memory.take_reads();
    check_eq!(ask(10, true)?.into_own_quotes(), Some(Vec::new()));
    check_eq!(memory.take_reads(), vec![("actors", 1)], "no row, one read");
    check!(
        memory.take_scans().is_empty(),
        "OwnQuotes never scans the listings"
    );
    let text = serde_json::to_value(ask(8, true)?)?.to_string();
    for private in [
        "seller",
        "account",
        "payer",
        "digest",
        "buyer",
        "\"quote\":",
    ] {
        check!(!text.contains(private), "{private} in {text}");
    }
    for token in [older, newer, aside, twin, paid, listed] {
        check!(
            !text.contains(&token.to_string()),
            "a whole token in {text}"
        );
    }
    check!(
        text.contains("\"kind\":\"own_quotes\""),
        "the kind tag: {text}"
    );
    Ok(())
}

/// 17g C2g, the C2g fix round 2 (reviewers correctness/C2G-R2-1,
/// conformance/F1 and tests/T-1): `OwnPurchaseView::latest`, `+minds/buy`'s
/// key subject, names the actor's most recently quoted own record on the
/// listing, a funded purchase included, through the actor's index states of
/// one buyer's life on one listing:
/// - a quote Q1 names Q1; once funded (Q1 leaves the index), the purchase's
///   operation P1, never the bare listing again;
/// - a second quote Q2 beside P1 names Q2; once funded, P2 (beside P1);
/// - a quote quoted in the same second as a funded purchase's quote ranks
///   after the purchase (it was quoted after that purchase's Fund);
/// - every Quote and Fund so far made a token the listing's earlier states
///   never named, so a `+minds/buy` after any of them keys anew;
/// - the stated residual: a Grant or Refund of P2 takes it out of the index
///   and the subject is P1's again, and a Grant or Refund of the last
///   record leaves no record at all (`None`, the bare listing): a re-buy
///   after a restart under a stopped IC clock can then meet the key of an
///   earlier `+minds/buy` (tacsim's `next_key` doc).
#[test]
fn own_purchase_latest_names_the_newest_own_record() -> TestResult {
    use crate::mind_entitlement::tests::command_index;
    let config = crate::mind_entitlement::tests::command_config(400)?;
    let me = ObjId(8);
    let listed = token([0x40, 0, 0, 1], 1);
    let mut memory = Memory::default();
    let q1 = quote_into(&mut memory, 0xb0, listed, me, 100, 160)?;
    let p1 = purchase_into(&mut memory, 0xb0, listed, me, 100)?;
    let q2 = quote_into(&mut memory, 0xb1, listed, me, 120, 180)?;
    let p2 = purchase_into(&mut memory, 0xb1, listed, me, 120)?;
    let q3 = quote_into(&mut memory, 0xb2, listed, me, 120, 180)?;
    let states: [(&[CommitToken], &[CommitToken], Option<CommitToken>); 7] = [
        (&[q1], &[], Some(q1)),
        (&[], &[p1], Some(p1)),
        (&[q2], &[p1], Some(q2)),
        (&[], &[p1, p2], Some(p2)),
        (&[q3], &[p1, p2], Some(q3)),
        (&[], &[p1], Some(p1)),
        (&[], &[], None),
    ];
    let mut seen = vec![None];
    for (n, (quotes, operations, expected)) in states.into_iter().enumerate() {
        // An empty row is never written: the release deletes it.
        if quotes.is_empty() && operations.is_empty() {
            memory.actors.remove(&me);
        } else {
            memory
                .actors
                .insert(me, command_index(me, quotes, operations)?);
        }
        let source = &memory;
        let latest = answer(
            &source,
            own(8),
            || true,
            &config,
            MarketCommandRequest::own_purchase(&code(listed)),
        )?
        .into_own_purchase()
        .ok_or("an own-purchase answer")?
        .map(|view| view.latest());
        check_eq!(latest, expected, "state {n}");
        if n < 5 {
            check!(!seen.contains(&latest), "state {n} keys anew");
        } else {
            check!(seen.contains(&latest), "state {n}: the stated residual");
        }
        seen.push(latest);
    }
    Ok(())
}

/// 17g C2d test 2 under its binding name (the C2e plan §4.1 item 4, review
/// FE-F14): `ResaleOffers { licence }` lists only the live seller offers
/// naming the actor on that licence, from the actor's own `(buyer,
/// licence)` index row and the offer rows it names, through `answer`:
/// - the actor's two live offers on licence 41 are listed with their frozen
///   prices, expiries and the acceptance step Roll's claim census answers;
///   the actor's offer at its expiry is not (the economy refuses an
///   acceptance at or after it);
/// - another buyer's offer on the same licence and the actor's offer on
///   another licence are absent; a licence nobody offered the actor answers
///   byte-identically to one that does not exist;
/// - Roll is asked for the step only when a live offer naming the actor
///   exists (never for an empty or all-expired row), so the step is no
///   oracle;
/// - an index row naming an offer to another buyer is `Integrity`;
/// - no answer carries the seller, an account or a digest.
#[test]
fn market_command_reader_lists_only_the_actors_resale_offers() -> TestResult {
    use crate::mind_paid_resale::{command_buyer_offers, command_offer};
    let config = crate::mind_entitlement::tests::command_config(400)?;
    let (me, other) = (ObjId(8), ObjId(9));
    let now = own(8).now;
    let cheap = command_offer(2, me, 41, 21, now + 600)?;
    let dear = command_offer(3, me, 41, 500, now + 1_201)?;
    let lapsed = command_offer(4, me, 41, 21, now)?;
    let theirs = command_offer(5, other, 41, 21, now + 600)?;
    let aside = command_offer(6, me, 42, 21, now + 600)?;
    let lapsed_only = command_offer(7, me, 43, 21, now - 1)?;
    let mut memory = Memory {
        step: Some(ResaleKind::Fund),
        ..Memory::default()
    };
    for offer in [&cheap, &dear, &lapsed, &theirs, &aside, &lapsed_only] {
        memory.offers.insert(offer.operation(), offer.clone());
    }
    let licence = |n: u64| LicenceId::new(n);
    memory.buyer_offers.insert(
        (me, licence(41)?),
        command_buyer_offers(
            me,
            41,
            &[cheap.operation(), dear.operation(), lapsed.operation()],
        )?,
    );
    memory.buyer_offers.insert(
        (other, licence(41)?),
        command_buyer_offers(other, 41, &[theirs.operation()])?,
    );
    memory.buyer_offers.insert(
        (me, licence(42)?),
        command_buyer_offers(me, 42, &[aside.operation()])?,
    );
    memory.buyer_offers.insert(
        (me, licence(43)?),
        command_buyer_offers(me, 43, &[lapsed_only.operation()])?,
    );
    let source = &memory;
    let ask = |actor: u32, n: u64| -> Result<MarketCommandResult, Box<dyn std::error::Error>> {
        Ok(answer(
            &source,
            own(actor),
            || true,
            &config,
            MarketCommandRequest::resale_offers(licence(n)?),
        )?)
    };

    let mine = ask(8, 41)?.into_resale_offers().ok_or("offers")?;
    check_eq!(
        mine.iter()
            .map(|v| (v.offer(), v.price_minor(), v.expires_at_utc(), v.step()))
            .collect::<Vec<_>>(),
        vec![
            (cheap.operation(), 21, now + 600, ResaleKind::Fund),
            (dear.operation(), 500, now + 1_201, ResaleKind::Fund),
        ],
        "the two live offers naming me; the lapsed one is left out"
    );
    check_eq!(*memory.steps.borrow(), 1, "the step is asked once");
    check_eq!(
        memory.take_reads(),
        vec![("buyer_offers", 1), ("offers", 3)],
        "one index read and one batched offer read"
    );
    // Away from a counter the answer is the same (the C2g fix round 2,
    // reviewer tests/T-2), and no listing is ever scanned.
    let away = answer(
        &source,
        own(8),
        || false,
        &config,
        MarketCommandRequest::resale_offers(licence(41)?),
    )?;
    check_eq!(away.into_resale_offers().as_ref(), Some(&mine));
    check!(memory.take_scans().is_empty(), "no listing scan");
    memory.take_reads();
    check_eq!(
        ask(9, 41)?.into_resale_offers().map(|offers| offers
            .iter()
            .map(ResaleOfferView::offer)
            .collect::<Vec<_>>()),
        Some(vec![theirs.operation()]),
        "the other buyer reads only its own"
    );
    check_eq!(
        ask(8, 42)?.into_resale_offers().map(|offers| offers
            .iter()
            .map(ResaleOfferView::offer)
            .collect::<Vec<_>>()),
        Some(vec![aside.operation()]),
        "an offer on another licence is that licence's"
    );
    let steps = *memory.steps.borrow();
    let unoffered = ask(10, 41)?;
    let missing = ask(8, 999)?;
    check_eq!(unoffered.clone().into_resale_offers(), Some(Vec::new()));
    check_eq!(
        serde_json::to_string(&unoffered)?,
        serde_json::to_string(&missing)?,
        "nobody's offer to me reads as a missing licence"
    );
    check_eq!(
        ask(8, 43)?.into_resale_offers(),
        Some(Vec::new()),
        "an all-expired row lists nothing"
    );
    check_eq!(
        *memory.steps.borrow(),
        steps,
        "no live offer, no step asked"
    );
    for value in [
        serde_json::to_value(ask(8, 41)?)?,
        serde_json::to_value(ask(9, 41)?)?,
    ] {
        let text = value.to_string();
        for private in ["seller", "account", "payer", "digest", "buyer"] {
            check!(!text.contains(private), "{private} in {text}");
        }
        for id in [7, 10] {
            check!(!text.contains(&format!(":{id},")) && !text.contains(&format!(":{id}}}")));
        }
        let longest = text
            .split(|c: char| !(c.is_ascii_digit() || ('a'..='f').contains(&c)))
            .map(str::len)
            .max()
            .unwrap_or(0);
        check!(longest < 64, "a digest-length run in {text}");
    }
    // A row naming an offer to another buyer is corrupt.
    memory.buyer_offers.insert(
        (me, licence(44)?),
        command_buyer_offers(me, 44, &[theirs.operation()])?,
    );
    let source = &memory;
    check_eq!(
        answer(
            &source,
            own(8),
            || true,
            &config,
            MarketCommandRequest::resale_offers(licence(44)?)
        ),
        Err(OwnerReadError::Integrity)
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
    let ask = |here: bool, request| answer(&source, own(8), || here, &config, request);

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

/// Doc 17h §3 M8f (P3, P5): the procurement kinds answer GOD alone (the tick's caller) and
/// refuse every other caller before reading anything; `ProcurementListings` pages the current
/// listings without the counter, each with the listed revision and rating, and never a row
/// the policy no longer admits; `ProcurementOperation` answers one purchase's phase and facts,
/// `None` for a token that names none; `ProcurementOpen` and `ProcurementPurchase` answer for
/// the named agent and the named organisation only.
#[test]
fn procurement_kinds_answer_god_alone() -> TestResult {
    use crate::mind_entitlement::tests::{command_config, current_listing_fixture};
    let config = command_config(400)?;
    let older = command_config(300)?;
    let listed = token([0x40, 0, 0, 1], 1);
    let stale = token([0x41, 0, 0, 1], 1);
    let mut memory = Memory::default();
    let row = current_listing_fixture(&config, listed)?;
    let subject = row.procurement_subject();
    memory.add(row);
    memory.add(current_listing_fixture(&older, stale)?);
    let buyer = ObjId(8);
    let paid = purchase_into(&mut memory, 0xb0, listed, buyer, 90)?;
    let source = &memory;
    let ask = |actor: u32, request| answer(&source, own(actor), || false, &config, request);
    let org = mush_org::OrgId(3);
    for request in [
        MarketCommandRequest::procurement_listings(None),
        MarketCommandRequest::procurement_operation(paid),
        MarketCommandRequest::procurement_open(buyer, org),
        MarketCommandRequest::procurement_purchase(buyer, listed, org),
    ] {
        check_eq!(
            ask(8, request),
            Err(OwnerReadError::Refused),
            "a player's call"
        );
    }
    check!(memory.take_scans().is_empty() && memory.take_reads().is_empty());
    let page = ask(1, MarketCommandRequest::procurement_listings(None))?
        .into_procurement_listings()
        .ok_or("a page")?;
    check_eq!(
        page.rows().len(),
        1,
        "only the current listing, away from any counter"
    );
    let first = page.rows().first().ok_or("the listing")?;
    check_eq!(first.view().publication(), listed);
    check_eq!((first.revision(), first.rating()), subject);
    check_eq!(page.next(), None);
    let view = ask(1, MarketCommandRequest::procurement_operation(paid))?
        .into_procurement_operation()
        .ok_or("an operation answer")?
        .ok_or("the purchase")?;
    check_eq!(
        (
            view.operation(),
            view.listing(),
            view.price_minor(),
            view.org()
        ),
        (paid, listed, 400, None)
    );
    check_eq!(view.phase(), procurement::ProcurementPhase::Funded);
    check_eq!(
        ask(1, MarketCommandRequest::procurement_operation(stale))?.into_procurement_operation(),
        Some(None),
        "no such purchase"
    );
    // The buyer does not act for the organisation: its row reads refused.
    check_eq!(
        ask(1, MarketCommandRequest::procurement_open(buyer, org)),
        Err(OwnerReadError::Refused)
    );
    procurement_reads_the_organisations_row()
}

/// Doc 17h §3 M8f with M8c S1's buyer index: the House agent's quote sits on
/// the organisation's row. GOD's procurement read finds it as the agent, whose
/// grant the read asks as a read (the P2a ruling: a budget-limited grant
/// reads); while the agent does not act for the organisation the read is
/// refused, and the agent itself is refused the GOD-only kind outright.
fn procurement_reads_the_organisations_row() -> TestResult {
    use crate::mind_entitlement::tests::{
        command_config, command_org_index, command_org_quote, current_listing_fixture,
    };
    let config = command_config(400)?;
    let listed = token([0x42, 0, 0, 1], 1);
    let (org, agent) = (mush_org::OrgId(3), ObjId(50));
    let mut memory = Memory::default();
    memory.add(current_listing_fixture(&config, listed)?);
    let quote = Box::new(command_org_quote(
        token([0xc0, 0, 0, 0], 1),
        listed,
        (org, agent),
        900,
        1_200,
    )?);
    let quoted = quote.publication();
    memory.quotes.insert(quoted, *quote);
    // H1: a NEWER quote by another member on the same listing is not the
    // agent's, and its read never returns it.
    let theirs = Box::new(command_org_quote(
        token([0xc1, 0, 0, 0], 1),
        listed,
        (org, ObjId(51)),
        950,
        1_200,
    )?);
    let theirs_token = theirs.publication();
    memory.quotes.insert(theirs_token, *theirs);
    memory
        .orgs
        .insert(org, command_org_index(org, &[quoted, theirs_token])?);
    {
        let source = &memory;
        let ask = |actor: u32, request| answer(&source, own(actor), || true, &config, request);
        check_eq!(
            ask(
                1,
                MarketCommandRequest::procurement_purchase(agent, listed, org)
            ),
            Err(OwnerReadError::Refused),
            "the agent does not act for the House yet"
        );
    }
    memory.members.insert((org, agent));
    let source = &memory;
    let ask = |actor: u32, request| answer(&source, own(actor), || true, &config, request);
    let found = ask(
        1,
        MarketCommandRequest::procurement_purchase(agent, listed, org),
    )?
    .into_procurement_purchase()
    .ok_or("a procurement purchase answer")?
    .ok_or("the agent's record")?;
    check_eq!(found.quote(), Some(quoted));
    check!(!found.funded());
    check_eq!(found.price_minor(), 400);
    check_eq!(
        ask(
            50,
            MarketCommandRequest::procurement_purchase(agent, listed, org)
        ),
        Err(OwnerReadError::Refused),
        "the agent is refused the GOD-only kind"
    );
    Ok(())
}

/// Doc 17h §3 M8f round 2 (the human path of H1): on an organisation's row,
/// `/buy/pay … org=`'s quote is the actor's OWN newest, never a newer quote
/// another member (or the House's agent) made on the same listing, whose
/// Fund the provider would refuse the actor:
/// - member A quotes at 900, then B quotes the same listing at 950: A's
///   `OwnPurchase` names A's quote, B's names B's;
/// - a member with no quote of its own there has none to fund (`None`),
///   though the row holds two.
#[test]
fn org_own_purchase_funds_the_actors_own_quote() -> TestResult {
    use crate::mind_entitlement::tests::{
        command_config, command_org_index, command_org_quote, current_listing_fixture,
    };
    let config = command_config(400)?;
    let listed = token([0x43, 0, 0, 1], 1);
    let org = mush_org::OrgId(3);
    let (a, b, c) = (ObjId(51), ObjId(52), ObjId(53));
    let mut memory = Memory::default();
    memory.add(current_listing_fixture(&config, listed)?);
    let mut quote_as = |n: u8, acting: ObjId, at: i64| -> TestResult<CommitToken> {
        let quote = Box::new(command_org_quote(
            token([n, 0, 0, 0], 1),
            listed,
            (org, acting),
            at,
            1_200,
        )?);
        let publication = quote.publication();
        memory.quotes.insert(publication, *quote);
        Ok(publication)
    };
    let mine = quote_as(0xd0, a, 900)?;
    let theirs = quote_as(0xd1, b, 950)?;
    memory
        .orgs
        .insert(org, command_org_index(org, &[mine, theirs])?);
    for member in [a, b, c] {
        memory.members.insert((org, member));
    }
    let source = &memory;
    let quote_of = |actor: ObjId| -> TestResult<Option<CommitToken>> {
        Ok(answer(
            &source,
            own(actor.0),
            || true,
            &config,
            MarketCommandRequest::own_purchase_for(&code(listed), Some(org)),
        )?
        .into_own_purchase()
        .ok_or("an own-purchase answer")?
        .and_then(|view| view.quote())
        .map(|quote| quote.quote()))
    };
    check_eq!(
        quote_of(a)?,
        Some(mine),
        "A funds A's quote, not B's newer one"
    );
    check_eq!(quote_of(b)?, Some(theirs), "B funds B's");
    check_eq!(quote_of(c)?, None, "C quoted nothing to fund");
    Ok(())
}
