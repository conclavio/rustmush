//! 17g C2f (the lead's ruling R2, option (a); the C2e plan §3.1): the closed
//! admission of the four per-owner indexes inside the money transactions.
//!
//! The four families (the C2b brief §1–§4) let a command find the actor's own
//! records with a bounded read: the buyer's open entitlement quotes and
//! purchases, the holder's paid licences, and the offers naming a buyer. Each
//! **authorizes nothing**; it is written in the same transaction as the step it
//! follows. Three of those steps are money transitions whose typed checks are
//! exact: the terminal branch of `prepare` refuses any key the reservation did
//! not name ("unreserved terminal key"), and the paid-resale and pending-charter
//! transitions compare their whole mutation set with a fixed list. This module
//! is the one place those checks admit an index row, and it admits nothing but
//! the rows of one `const` table:
//!
//! - an entry is `(protocol, action index, family owner, family, rule)`; the
//!   protocols index their actions by position, as every protocol here does;
//! - the EXPECTED KEY, op and row are never a shape: `expected` derives them
//!   from the transaction's own committed record (the purchase, the licence
//!   pointer, the resale or pending operation, the seller's offer), exactly as
//!   `charter_pending::extra_terminal_key` and `paid_resale::extra_terminal_key`
//!   derive their sanction keys from the row;
//! - "Put, or Delete when empty" is decided by the row's content: a Delete of a
//!   row that would still hold a token, or a Put of an empty row, differs;
//! - a second index row, another buyer's or holder's key, a wrong licence or
//!   charter, the wrong op, or the row at an unlisted (protocol, action) is not
//!   admitted: the terminal branch refuses "unreserved terminal key", the
//!   exact sets "… exact typed union differs" and the entitlement, House and
//!   v2 validators "… index row differs";
//! - a transition that forgets its row refuses too: the exact sets with their
//!   own message, and the entitlement, House and v2 purchase terminals (whose
//!   validators filter only their own families, so only the terminal branch
//!   would otherwise see the row) with "… index row required" (`require`);
//! - those three validators check their rows exactly as well (`require`): a
//!   wrong op, a wrong row or a second row of an admitted family refuses
//!   "… index row differs" there, before the terminal branch, which admits
//!   the same rows again; a row of a family not admitted at that action (a
//!   holdings row at a Refund) refuses in the terminal branch.
//!
//! Rules the table rests on (the C2e plan §3.1):
//! 1. `roll_mind_paid_holdings` joins no protocol's own-family filter
//!    (`paid_resale::own`, `charter_pending::relevant`, the purchase validators'
//!    `protected`): four protocols write it, so in any one filter the others'
//!    grants would read as foreign writes. Each protocol requires its own
//!    derived key instead, and one audit (`audit_boot` here, and Roll's
//!    commerce boot audit in its owner) checks the whole family.
//! 2. The paid-resale and pending-charter exact sets are exact typed sets over
//!    `(key, Put | Delete)`: the seller's holdings row and an emptied offer row
//!    are Deletes.
//! 3. `roll_mind_paid_holdings` is delegated to the market as
//!    `roll_mind_personal_paid_licence` is: the market's transactions run the
//!    entitlement, House, v2 and resale grants. The renewal writers replace the
//!    pointer with the same holder principal and write no holdings row (M8c
//!    F4: an organisation-held pointer is re-stamped with the renewing actor
//!    as its `acting`; the row keeps the acting of the grant or transfer that
//!    put it, so `audit_holdings` compares the two by principal, `same_holder`).
//! 4. A Grant's or Settle's index put is unreserved capacity. The reservation
//!    names only its own targets, so the put is charged by the ordinary typed
//!    accounting like any unreserved write: at a store whose ordinary aggregate
//!    leaves no room for it the Grant refuses (and moves no money), while a
//!    Refund, whose only index write shrinks or deletes the buyer's actors row,
//!    never needs room.
//!
//! Deliberately not admitted: anything at a resale or pending TERMINAL for the
//! two offer indexes (the token leaves the row at Accept or Fund, so "an
//! indexed offer has no operation" holds across a boot after any Fund); any
//! holdings row at a revocation, recall, lapse or either renewal grant (no
//! holder changes, and a revoked licence keeps its row); the actors row at
//! Quote or Fund, which are ordinary typed writes no exact check or terminal
//! branch sees (the boot audits cover them). The v2 purchase keeps its own
//! `mind_trade::index` and a House purchase is not in the actors index (the
//! C2b brief §1 names the entitlement protocols only).
use super::*;

/// The buyer's open entitlement quotes and purchases, keyed by the buyer.
pub(super) const ACTORS: &str = "market_mind_entitlement_actors";
/// One row per held paid licence, keyed `{holder:020}/licence/{licence:016x}`.
pub(super) const HOLDINGS: &str = "roll_mind_paid_holdings";
/// The live paid-resale offers naming a buyer for one licence.
pub(super) const RESALE_OFFERS: &str = "market_mind_paid_resale_buyer_offers";
/// The live pending-charter offers naming a buyer for one charter.
pub(super) const PENDING_OFFERS: &str = "roll_mind_charter_pending_buyer_offers";
/// The C2b bound on every token set of the three bounded indexes
/// (`MAX_BUYER_QUOTES`, `MAX_BUYER_PURCHASES`, `MAX_BUYER_OFFERS`).
pub(super) const MAX_TOKENS: usize = 8;

const PURCHASE: &str = "market_mind_entitlement_purchase";
const QUOTE: &str = "market_mind_entitlement_quote";
const HOUSE_PURCHASE: &str = "market_mind_house_purchase";
const PAID_PURCHASE: &str = "market_mind_purchase";
const LICENCE: &str = "roll_mind_personal_paid_licence";
const RESALE: &str = "roll_mind_paid_resales";
const RESALE_OFFER: &str = "market_mind_paid_resale_offers";
const PENDING: &str = "roll_mind_charter_pending";
const PENDING_OFFER: &str = "roll_mind_charter_pending_offers";

/// `(owner, family, key bytes, record bytes, delegated to)`: the exact
/// descriptors each owner registers. A market row is a 32-byte-key family
/// except the resale offers, whose `{buyer:020}/licence/{licence:016x}` key
/// is 45 bytes.
const DESCRIPTORS: [(&str, &str, usize, usize, Option<&str>); 4] = [
    ("market", ACTORS, 32, 4096, None),
    ("roll", HOLDINGS, 192, 1024, Some("market")),
    ("market", RESALE_OFFERS, 64, 2048, None),
    ("roll", PENDING_OFFERS, 192, 2048, None),
];

/// The money protocols whose transitions the table names.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Owner {
    /// `EntitlementProtocol` (OriginalV3, DerivedV1 and their Follow kinds).
    Entitlement,
    /// `HouseProtocol` (every registered House kind).
    House,
    /// The v2 `PurchaseProtocol` (`paid.rs`, `settle_original_mind`).
    Purchase,
    /// `PaidResaleProtocol` (Offer 0, Accept 1, Fund 2, Settle 3, Refund 4,
    /// the Registry revocation 5).
    PaidResale,
    /// `CharterPendingProtocol` (Offer 0, Fund 1, Settle 2, Refund 3, the
    /// Registry revocation 4).
    CharterPending,
}

/// How a row's expected key, op and content follow from the committed record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Rule {
    /// The purchase's own buyer's actors row minus this operation: a Put, or
    /// a Delete when both of its sets would be empty.
    Release,
    /// The holder's holdings row, from the licence pointer post-image.
    Hold,
    /// The new holder's row Put and the old holder's row Deleted, from the
    /// pointer post-image and pre-image.
    Transfer,
    /// The named buyer's offer row: its retained tokens plus this offer.
    Offer,
    /// The named buyer's offer row minus the offer this step consumes: a Put,
    /// or a Delete when no token is left.
    Consume,
}

/// One closed admission.
#[derive(Clone, Copy, Debug)]
pub(super) struct Admission {
    pub(super) protocol: Owner,
    pub(super) action: usize,
    pub(super) owner: &'static str,
    pub(super) family: &'static str,
    pub(super) rule: Rule,
}

/// The whole admission table (the C2e plan §3.1, as built). Nothing outside
/// it is admitted; there is no wildcard on protocol, action, owner, family or
/// key.
pub(super) const TABLE: [Admission; 12] = [
    // The entitlement Grant and Refund release the operation from its own
    // buyer's actors row (a GOD refund edits the buyer's row, never GOD's).
    Admission {
        protocol: Owner::Entitlement,
        action: 1,
        owner: "market",
        family: ACTORS,
        rule: Rule::Release,
    },
    Admission {
        protocol: Owner::Entitlement,
        action: 2,
        owner: "market",
        family: ACTORS,
        rule: Rule::Release,
    },
    // The three grants put the holder's row beside the new pointer. The
    // House row is action 1 of every House kind (HouseV1, the two-seat
    // HouseV2 and core's Follow v3): `house::Protocol::validate_composition`
    // pins each kind's actions to `TRANSITIONS` (fund, grant, refund), so
    // action 1 is always that kind's grant, and Roll's one House writer
    // (`house_purchase::writer`) puts the holdings row for every kind.
    Admission {
        protocol: Owner::Entitlement,
        action: 1,
        owner: "roll",
        family: HOLDINGS,
        rule: Rule::Hold,
    },
    Admission {
        protocol: Owner::House,
        action: 1,
        owner: "roll",
        family: HOLDINGS,
        rule: Rule::Hold,
    },
    Admission {
        protocol: Owner::Purchase,
        action: 1,
        owner: "roll",
        family: HOLDINGS,
        rule: Rule::Hold,
    },
    // The resale's two holder changes, the immediate Accept and the
    // service Settle, move the row from seller to buyer.
    Admission {
        protocol: Owner::PaidResale,
        action: 1,
        owner: "roll",
        family: HOLDINGS,
        rule: Rule::Transfer,
    },
    Admission {
        protocol: Owner::PaidResale,
        action: 3,
        owner: "roll",
        family: HOLDINGS,
        rule: Rule::Transfer,
    },
    // The seller's Offer indexes the token under the named buyer; the
    // buyer's Accept or Fund takes it out again.
    Admission {
        protocol: Owner::PaidResale,
        action: 0,
        owner: "market",
        family: RESALE_OFFERS,
        rule: Rule::Offer,
    },
    Admission {
        protocol: Owner::PaidResale,
        action: 1,
        owner: "market",
        family: RESALE_OFFERS,
        rule: Rule::Consume,
    },
    Admission {
        protocol: Owner::PaidResale,
        action: 2,
        owner: "market",
        family: RESALE_OFFERS,
        rule: Rule::Consume,
    },
    // The pending charter's Offer and Fund, likewise.
    Admission {
        protocol: Owner::CharterPending,
        action: 0,
        owner: "roll",
        family: PENDING_OFFERS,
        rule: Rule::Offer,
    },
    Admission {
        protocol: Owner::CharterPending,
        action: 1,
        owner: "roll",
        family: PENDING_OFFERS,
        rule: Rule::Consume,
    },
];

/// One expected index mutation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Expect {
    /// A Put of exactly this row.
    Put(Value),
    /// A Delete.
    Delete,
    /// A Put of the named buyer's offer row: the fixed `head` fields, and an
    /// `offers` set that holds `token` and otherwise only tokens the row
    /// already held (the writer prunes expired offers against the world
    /// clock, which a typed check does not read), at most `MAX_TOKENS`.
    Offered {
        head: Value,
        token: String,
        retained: BTreeSet<String>,
    },
}

impl Expect {
    fn matches(&self, mutation: &TxnMutation) -> bool {
        match (self, mutation) {
            (Self::Put(row), TxnMutation::Put { document, .. }) => {
                document.get("value") == Some(row)
            }
            (Self::Delete, TxnMutation::Delete) => true,
            (
                Self::Offered {
                    head,
                    token,
                    retained,
                },
                TxnMutation::Put { document, .. },
            ) => document
                .get("value")
                .is_some_and(|row| offered(row, head, token, retained)),
            _ => false,
        }
    }
}

fn offered(row: &Value, head: &Value, token: &str, retained: &BTreeSet<String>) -> bool {
    let (Some(row), Some(head)) = (row.as_object(), head.as_object()) else {
        return false;
    };
    if row.len() != head.len() + 1 || head.iter().any(|(name, v)| row.get(name) != Some(v)) {
        return false;
    }
    let Ok(offers) = tokens(row.get("offers").unwrap_or(&Value::Null)) else {
        return false;
    };
    offers.iter().any(|t| t == token)
        && offers
            .iter()
            .all(|t| t == token || retained.contains(t.as_str()))
}

fn field<'a>(v: &'a Value, name: &str) -> Result<&'a Value, ActionStateError> {
    v.get(name)
        .ok_or_else(|| refused("owner index field missing"))
}
/// A character id as the rows carry it: a number in `1..=u32::MAX`.
fn person(v: &Value) -> Result<u64, ActionStateError> {
    v.as_u64()
        .filter(|n| (1..=u64::from(u32::MAX)).contains(n))
        .ok_or_else(|| refused("owner index person"))
}
/// A licence or charter id as the rows carry it: a canonical nonzero decimal.
fn decimal(v: &Value, max: u64) -> Result<u64, ActionStateError> {
    let text = v.as_str().ok_or_else(|| refused("owner index decimal"))?;
    let n: u64 = text.parse().map_err(|_| refused("owner index decimal"))?;
    if n == 0 || n > max || n.to_string() != text {
        return Err(refused("owner index canonical decimal"));
    }
    Ok(n)
}
fn token(v: &Value) -> Result<CommitToken, ActionStateError> {
    serde_json::from_value(v.clone()).map_err(|_| refused("owner index token"))
}
/// A bounded token set as a `BTreeSet<CommitToken>` serializes it: an array
/// of canonical tokens in strictly ascending order, at most `MAX_TOKENS`.
fn tokens(v: &Value) -> Result<Vec<String>, ActionStateError> {
    let items = v
        .as_array()
        .ok_or_else(|| refused("owner index token set"))?;
    if items.len() > MAX_TOKENS {
        return Err(refused("owner index token bound"));
    }
    let mut out: Vec<String> = Vec::with_capacity(items.len());
    for item in items {
        let text = item
            .as_str()
            .ok_or_else(|| refused("owner index token set"))?;
        if token(item)?.to_hex() != text || out.last().is_some_and(|last| last.as_str() >= text) {
            return Err(refused("owner index token order"));
        }
        out.push(text.to_owned());
    }
    Ok(out)
}
fn key(owner: &str, family: &str, encoded: String) -> Result<Key, ActionStateError> {
    Ok(Key {
        owner: OwnerId::new(owner).map_err(|_| refused("owner index owner"))?,
        family: StateFamilyId::new(family).map_err(|_| refused("owner index family"))?,
        encoded: encoded.into(),
    })
}
fn read(view: &View<'_>, k: &Key) -> Result<Option<Value>, ActionStateError> {
    let k = k.resolve(view.registry)?;
    let document = match view.mutations.and_then(|m| m.get(&k)) {
        Some(TxnMutation::Put { document, .. }) => Some(document.clone()),
        Some(TxnMutation::Delete) => None,
        None => view
            .records
            .get(&k)
            .map(|r| {
                serde_json::from_slice(r.canonical.as_slice())
                    .map_err(|_| refused("owner index retained encoding"))
            })
            .transpose()?,
    };
    document.map(|v| field(&v, "value").cloned()).transpose()
}
fn required(view: &View<'_>, k: &Key) -> Result<Value, ActionStateError> {
    read(view, k)?.ok_or_else(|| refused("owner index committed record missing"))
}
fn registered(registry: &StateFamilyRegistry, owner: &str, family: &str) -> bool {
    match (OwnerId::new(owner), StateFamilyId::new(family)) {
        (Ok(owner), Ok(family)) => registry.entries.contains_key(&(owner, family)),
        _ => false,
    }
}

/// A holder as the rows carry it (doc 17h §3 M8c, B1): a character's bare
/// number in `1..=u32::MAX`, or an organisation's object `{"org": n,
/// "acting": m}` (the acting character nonzero, the organisation any number:
/// mush-org mints from 0, F8; nothing else). The value is kept as read, so
/// a derived row serialises exactly as Roll's `PaidHolding` does; the key
/// prefix is Roll's `Holder::key_prefix`, byte for byte.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct HolderKey {
    prefix: String,
    value: Value,
}
impl HolderKey {
    /// A character holder.
    pub(super) fn person(n: u64) -> Self {
        Self {
            prefix: format!("{n:020}"),
            value: json!(n),
        }
    }
}
fn holder(v: &Value) -> Result<HolderKey, ActionStateError> {
    if v.is_u64() {
        return Ok(HolderKey::person(person(v)?));
    }
    let object = v.as_object().ok_or_else(|| refused("owner index holder"))?;
    if object.len() != 2 {
        return Err(refused("owner index holder"));
    }
    let org = field(v, "org")?
        .as_u64()
        .ok_or_else(|| refused("owner index holder"))?;
    person(field(v, "acting")?)?;
    Ok(HolderKey {
        prefix: format!("org/{org:020}"),
        value: v.clone(),
    })
}
/// The character acting as a holder: the character itself, or the
/// organisation's `acting` character. The actor pins of the funding and
/// renewal transitions compare the invocation's actor with this.
pub(super) fn acting(v: &Value) -> Result<u64, ActionStateError> {
    if v.is_u64() {
        return person(v);
    }
    holder(v)?;
    person(field(v, "acting")?)
}
/// Whether two holders are the same principal, whoever acted (M8c F4): the
/// same character, or the same organisation under any acting character. The
/// renewal writers re-stamp an organisation-held pointer with the renewing
/// actor, so a pointer and the rows that follow it agree by principal, never
/// by `acting`.
pub(super) fn same_holder(a: &Value, b: &Value) -> Result<bool, ActionStateError> {
    Ok(holder(a)?.prefix == holder(b)?.prefix)
}
/// The holdings key (and the resale offer key: the same grammar): the
/// holder's prefix (a zero-padded decimal person, or `org/` and the
/// organisation's), so one holder's rows are contiguous.
pub(super) fn holding_key(holder: &HolderKey, licence: u64) -> String {
    format!("{}/licence/{licence:016x}", holder.prefix)
}
/// The pending-charter offer key.
pub(super) fn pending_key(buyer: u64, charter: u64) -> String {
    format!("{buyer:020}/charter/{charter:016x}")
}
/// A holdings row exactly as Roll's `PaidHolding` serializes it.
pub(super) fn holding(holder: &HolderKey, licence: u64) -> Value {
    json!({"format":1,"holder":holder.value,"licence":licence.to_string()})
}

/// The protocol and action index of the actual invocation, if it is one of
/// the table's protocols at all.
fn invocation(
    store: &ActionStateStore,
    identity: &RegisteredTxnIdentity,
) -> Option<(Owner, usize)> {
    let r = &store.registry;
    if let Some((_, i)) = r
        .entitlement_purchase
        .as_ref()
        .and_then(|p| p.action_index(identity).ok())
    {
        return Some((Owner::Entitlement, i));
    }
    if let Some((_, i)) = r
        .house_purchase
        .as_ref()
        .and_then(|p| p.action_index(identity).ok())
    {
        return Some((Owner::House, i));
    }
    if let Some(i) = r
        .original_purchase
        .as_ref()
        .and_then(|p| p.action_index(identity).ok())
    {
        return Some((Owner::Purchase, i));
    }
    if let Some(i) = r
        .paid_resale
        .as_ref()
        .and_then(|p| p.action_index(identity).ok())
    {
        return Some((Owner::PaidResale, i));
    }
    r.charter_pending
        .as_ref()
        .and_then(|p| p.action_index(identity).ok())
        .map(|i| (Owner::CharterPending, i))
}

fn admitted(protocol: Owner, action: usize, candidate: &StateKey) -> bool {
    TABLE.iter().any(|row| {
        row.protocol == protocol
            && row.action == action
            && row.owner == candidate.owner.as_str()
            && row.family == candidate.family.as_str()
    })
}

/// Every index mutation the table expects of `(protocol, action)`, each
/// derived from the transaction's own committed record. `subject` is the
/// purchase operation (Entitlement, House, Purchase), the resale or pending
/// operation (Accept, Fund, Settle), or the seller's offer (both Offers).
pub(super) fn expected(
    store: &ActionStateStore,
    mutations: &BTreeMap<StateKey, TxnMutation>,
    protocol: Owner,
    action: usize,
    subject: CommitToken,
) -> Result<BTreeMap<StateKey, Expect>, ActionStateError> {
    let before = View {
        registry: &store.registry,
        records: &store.records,
        mutations: None,
    };
    let after = View {
        registry: &store.registry,
        records: &store.records,
        mutations: Some(mutations),
    };
    let mut out = BTreeMap::new();
    for row in TABLE
        .iter()
        .filter(|row| row.protocol == protocol && row.action == action)
    {
        let rows = match (row.rule, row.protocol) {
            (Rule::Release, _) => release(&before, &after, subject)?,
            (Rule::Hold, Owner::Entitlement) => hold(&after, PURCHASE, subject)?,
            (Rule::Hold, Owner::House) => hold(&after, HOUSE_PURCHASE, subject)?,
            (Rule::Hold, Owner::Purchase) => hold(&after, PAID_PURCHASE, subject)?,
            (Rule::Transfer, _) => transfer(&before, &after, subject)?,
            (Rule::Offer, Owner::PaidResale) => resale_offer(&before, &after, subject)?,
            (Rule::Offer, Owner::CharterPending) => pending_offer(&before, &after, subject)?,
            (Rule::Consume, Owner::PaidResale) => resale_consume(&before, &after, subject)?,
            (Rule::Consume, Owner::CharterPending) => pending_consume(&before, &after, subject)?,
            _ => return Err(refused("owner index rule outside the table")),
        };
        for (k, expect) in rows {
            if k.owner.as_str() != row.owner || k.family.as_str() != row.family {
                return Err(refused("owner index row outside its family"));
            }
            if out.insert(k.resolve(&store.registry)?, expect).is_some() {
                return Err(refused("owner index derives one key twice"));
            }
        }
    }
    Ok(out)
}

/// Rule `Release`: the operation's own buyer (`quote.buyer` of the purchase
/// post-image; GOD may refund, so never the actor) loses the operation.
fn release(
    before: &View<'_>,
    after: &View<'_>,
    operation: CommitToken,
) -> Result<Vec<(Key, Expect)>, ActionStateError> {
    let purchase = required(after, &key("market", PURCHASE, operation.to_hex())?)?;
    // M8c: the market's actor index is keyed by the quote buyer's acting
    // character (a character holder, or an organisation's acting one).
    let buyer = acting(field(field(&purchase, "quote")?, "buyer")?)?;
    let k = key("market", ACTORS, buyer.to_string())?;
    let row = read(before, &k)?.ok_or_else(|| refused("owner index row absent"))?;
    let object = row
        .as_object()
        .ok_or_else(|| refused("owner index row shape"))?;
    let quotes = tokens(field(&row, "quotes")?)?;
    let mut operations = tokens(field(&row, "operations")?)?;
    let id = operation.to_hex();
    let before_len = operations.len();
    operations.retain(|t| *t != id);
    if object.len() != 4
        || field(&row, "format")? != &json!(1)
        || person(field(&row, "actor")?)? != buyer
        || operations.len() + 1 != before_len
    {
        return Err(refused("owner index row differs"));
    }
    let expect = if quotes.is_empty() && operations.is_empty() {
        Expect::Delete
    } else {
        Expect::Put(json!({"format":1,"actor":buyer,"quotes":quotes,"operations":operations}))
    };
    Ok(vec![(k, expect)])
}

/// Rule `Hold`: the grant's licence pointer post-image names the holder.
fn hold(
    after: &View<'_>,
    purchase_family: &str,
    operation: CommitToken,
) -> Result<Vec<(Key, Expect)>, ActionStateError> {
    let purchase = required(after, &key("market", purchase_family, operation.to_hex())?)?;
    let licence = decimal(field(&purchase, "licence")?, u64::MAX)?;
    let pointer = required(
        after,
        &key("roll", LICENCE, format!("licence/{licence:016x}"))?,
    )?;
    if decimal(field(&pointer, "id")?, u64::MAX)? != licence {
        return Err(refused("owner index pointer differs"));
    }
    let holder = holder(field(&pointer, "holder")?)?;
    Ok(vec![(
        key("roll", HOLDINGS, holding_key(&holder, licence))?,
        Expect::Put(holding(&holder, licence)),
    )])
}

/// Rule `Transfer`: the resale operation's licence; the pointer's pre-image
/// names the seller, its post-image the buyer.
fn transfer(
    before: &View<'_>,
    after: &View<'_>,
    operation: CommitToken,
) -> Result<Vec<(Key, Expect)>, ActionStateError> {
    let row = required(after, &key("roll", RESALE, operation.to_hex())?)?;
    let licence = decimal(field(field(&row, "binding")?, "licence")?, u64::MAX)?;
    let pointer = key("roll", LICENCE, format!("licence/{licence:016x}"))?;
    let seller = holder(field(&required(before, &pointer)?, "holder")?)?;
    let buyer = holder(field(&required(after, &pointer)?, "holder")?)?;
    let old = key("roll", HOLDINGS, holding_key(&seller, licence))?;
    if seller == buyer || read(before, &old)? != Some(holding(&seller, licence)) {
        return Err(refused("owner index row differs"));
    }
    Ok(vec![
        (
            key("roll", HOLDINGS, holding_key(&buyer, licence))?,
            Expect::Put(holding(&buyer, licence)),
        ),
        (old, Expect::Delete),
    ])
}

/// The named buyer's offer row before this step: its token set, after its
/// fixed fields are checked against `head`; an absent row holds none.
fn offer_tokens(before: &View<'_>, k: &Key, head: &Value) -> Result<Vec<String>, ActionStateError> {
    let Some(row) = read(before, k)? else {
        return Ok(Vec::new());
    };
    let (Some(object), Some(fixed)) = (row.as_object(), head.as_object()) else {
        return Err(refused("owner index row shape"));
    };
    if object.len() != fixed.len() + 1 || fixed.iter().any(|(name, v)| object.get(name) != Some(v))
    {
        return Err(refused("owner index row differs"));
    }
    let offers = tokens(field(&row, "offers")?)?;
    if offers.is_empty() {
        return Err(refused("owner index empty row"));
    }
    Ok(offers)
}

fn resale_head(offer: &Value) -> Result<(Key, Value), ActionStateError> {
    let buyer = person(field(field(offer, "terms")?, "buyer")?)?;
    let licence = decimal(field(field(offer, "binding")?, "licence")?, u64::MAX)?;
    Ok((
        key(
            "market",
            RESALE_OFFERS,
            holding_key(&HolderKey::person(buyer), licence),
        )?,
        json!({"format":1,"buyer":buyer,"licence":licence.to_string()}),
    ))
}
fn pending_head(offer: &Value) -> Result<(Key, Value), ActionStateError> {
    let buyer = person(field(offer, "buyer")?)?;
    let charter = decimal(field(offer, "charter")?, u64::from(u32::MAX))?;
    Ok((
        key("roll", PENDING_OFFERS, pending_key(buyer, charter))?,
        json!({"format":1,"buyer":buyer,"charter":charter.to_string()}),
    ))
}

/// Rule `Offer` (resale): the seller's offer row names the buyer and licence.
fn resale_offer(
    before: &View<'_>,
    after: &View<'_>,
    offer: CommitToken,
) -> Result<Vec<(Key, Expect)>, ActionStateError> {
    let row = required(after, &key("market", RESALE_OFFER, offer.to_hex())?)?;
    let (k, head) = resale_head(&row)?;
    offered_row(before, k, head, offer)
}
/// Rule `Offer` (pending charter): the seller's offer row names the buyer
/// and charter.
fn pending_offer(
    before: &View<'_>,
    after: &View<'_>,
    offer: CommitToken,
) -> Result<Vec<(Key, Expect)>, ActionStateError> {
    let row = required(after, &key("roll", PENDING_OFFER, offer.to_hex())?)?;
    let (k, head) = pending_head(&row)?;
    offered_row(before, k, head, offer)
}
fn offered_row(
    before: &View<'_>,
    k: Key,
    head: Value,
    offer: CommitToken,
) -> Result<Vec<(Key, Expect)>, ActionStateError> {
    let retained = offer_tokens(before, &k, &head)?.into_iter().collect();
    Ok(vec![(
        k,
        Expect::Offered {
            head,
            token: offer.to_hex(),
            retained,
        },
    )])
}

/// Rule `Consume` (resale): the acceptance names its offer; the offer names
/// the buyer and licence.
fn resale_consume(
    before: &View<'_>,
    after: &View<'_>,
    operation: CommitToken,
) -> Result<Vec<(Key, Expect)>, ActionStateError> {
    let row = required(after, &key("roll", RESALE, operation.to_hex())?)?;
    let offer = token(field(&row, "offer")?)?;
    let seller_offer = required(before, &key("market", RESALE_OFFER, offer.to_hex())?)?;
    let (k, head) = resale_head(&seller_offer)?;
    consumed_row(before, k, head, offer)
}
/// Rule `Consume` (pending charter): the Fund names its offer.
fn pending_consume(
    before: &View<'_>,
    after: &View<'_>,
    operation: CommitToken,
) -> Result<Vec<(Key, Expect)>, ActionStateError> {
    let row = required(after, &key("roll", PENDING, operation.to_hex())?)?;
    let offer = token(field(&row, "offer")?)?;
    let seller_offer = required(before, &key("roll", PENDING_OFFER, offer.to_hex())?)?;
    let (k, head) = pending_head(&seller_offer)?;
    consumed_row(before, k, head, offer)
}
fn consumed_row(
    before: &View<'_>,
    k: Key,
    head: Value,
    offer: CommitToken,
) -> Result<Vec<(Key, Expect)>, ActionStateError> {
    let mut offers = offer_tokens(before, &k, &head)?;
    let id = offer.to_hex();
    let before_len = offers.len();
    offers.retain(|t| *t != id);
    if offers.len() + 1 != before_len {
        return Err(refused("owner index row lacks the consumed offer"));
    }
    let expect = if offers.is_empty() {
        Expect::Delete
    } else {
        let mut row = head;
        if let Some(object) = row.as_object_mut() {
            object.insert("offers".into(), json!(offers));
        }
        Expect::Put(row)
    };
    Ok(vec![(k, expect)])
}

/// The terminal branch's admission (beside the two `extra_terminal_key`s):
/// true only when the actual invocation is a table row's `(protocol,
/// action)`, `candidate` is in that row's family, and its mutation is exactly
/// the one `expected` derives from the reservation's own operation.
pub(super) fn admits_terminal_key(
    store: &ActionStateStore,
    mutations: &BTreeMap<StateKey, TxnMutation>,
    record: &Reservation,
    identity: Option<&RegisteredTxnIdentity>,
    candidate: &StateKey,
) -> Result<bool, ActionStateError> {
    let Some((protocol, action)) = identity.and_then(|i| invocation(store, i)) else {
        return Ok(false);
    };
    if !admitted(protocol, action, candidate) {
        return Ok(false);
    }
    let expected = expected(store, mutations, protocol, action, record.funding)?;
    Ok(expected
        .get(candidate)
        .zip(mutations.get(candidate))
        .is_some_and(|(expect, mutation)| expect.matches(mutation)))
}

/// The entitlement, House and v2 purchase transitions' own exact check of
/// their index rows (R2 puts it in the entitlement validator; the House and
/// v2 grants share it). These validators filter their own `protected`
/// families, so without this only the terminal branch would see an index
/// row. Of every family the table admits at `(protocol, action)`, the
/// mutations must be exactly the ones `expected` derives:
/// - an expected key with no mutation refuses "… index row required" (a
///   transition that forgot its row, or wrote another buyer's or holder's
///   key in its place);
/// - an expected key whose mutation is the wrong op or the wrong row (a Put
///   where the content implies a Delete, a Delete of a row that still holds
///   a token, a row still holding the operation), or any other key of those
///   families (a second row), refuses "… index row differs".
///
/// The terminal branch then admits exactly the same mutations again
/// (`admits_terminal_key`); a row of a family the table does not admit at
/// this action (a holdings row at a Refund) passes here and refuses there,
/// "unreserved terminal key". Every one of these transitions is a terminal
/// (the validators pin `changes.terminal`), so both checks always run; this
/// one does not rely on that.
pub(super) fn require(
    store: &ActionStateStore,
    mutations: &BTreeMap<StateKey, TxnMutation>,
    protocol: Owner,
    action: usize,
    subject: CommitToken,
) -> Result<(), ActionStateError> {
    let (missing, differs) = match protocol {
        Owner::Entitlement => (
            "entitlement index row required",
            "entitlement index row differs",
        ),
        Owner::House => ("house index row required", "house index row differs"),
        Owner::Purchase => ("purchase index row required", "purchase index row differs"),
        Owner::PaidResale => ("resale index row required", "resale index row differs"),
        Owner::CharterPending => ("pending index row required", "pending index row differs"),
    };
    let expected = expected(store, mutations, protocol, action, subject)?;
    for (k, expect) in &expected {
        match mutations.get(k) {
            None => return Err(refused(missing)),
            Some(mutation) if !expect.matches(mutation) => return Err(refused(differs)),
            Some(_) => {}
        }
    }
    if mutations
        .keys()
        .any(|k| admitted(protocol, action, k) && !expected.contains_key(k))
    {
        return Err(refused(differs));
    }
    Ok(())
}

/// The paid-resale and pending-charter exact typed set: the fixed puts are
/// Puts, and the only other mutations are exactly the index mutations the
/// table expects of `(protocol, action)`. Anything else refuses `refusal`
/// ("… exact typed union differs"), so a transition that forgets its index
/// row refuses as one that forgets any other row does.
pub(super) fn exact_set(
    store: &ActionStateStore,
    mutations: &BTreeMap<StateKey, TxnMutation>,
    puts: &[Key],
    index: (Owner, usize, CommitToken),
    refusal: &'static str,
) -> Result<(), ActionStateError> {
    let (protocol, action, subject) = index;
    let fixed = puts
        .iter()
        .map(|k| k.resolve(&store.registry))
        .collect::<Result<BTreeSet<_>, _>>()?;
    if fixed.len() != puts.len()
        || fixed.iter().any(|k| admitted(protocol, action, k))
        || fixed
            .iter()
            .any(|k| !matches!(mutations.get(k), Some(TxnMutation::Put { .. })))
        || mutations
            .keys()
            .any(|k| !fixed.contains(k) && !admitted(protocol, action, k))
    {
        return Err(refused(refusal));
    }
    let expected = expected(store, mutations, protocol, action, subject)?;
    if mutations.len() != fixed.len() + expected.len()
        || expected.iter().any(|(k, expect)| {
            !mutations
                .get(k)
                .is_some_and(|mutation| expect.matches(mutation))
        })
    {
        return Err(refused(refusal));
    }
    Ok(())
}

/// One index family's owner, exact descriptor, and the owner it is delegated
/// to (the holdings family to the market; none for the other three).
pub(super) fn descriptor(
    family: &str,
) -> Result<(OwnerId, StateFamilyDescriptor, Option<&'static str>), ActionStateError> {
    let (owner, family, key_bytes, bytes, delegate) = DESCRIPTORS
        .into_iter()
        .find(|(_, name, ..)| *name == family)
        .ok_or_else(|| refused("owner index family outside the table"))?;
    let canonical = |suffix: &str| {
        CanonicalId::new(format!("{family}.{suffix}.v1")).map_err(|_| refused("owner index schema"))
    };
    let (decode, validate) = if owner == "roll" {
        ("strict_decode", "invariants")
    } else {
        ("decode", "validate")
    };
    let expected = StateFamilyDescriptor::new(
        StateFamilyId::new(family).map_err(|_| refused("owner index family"))?,
        canonical("key")?,
        canonical("value")?,
        1,
        canonical(decode)?,
        canonical(validate)?,
        canonical("canonical")?,
    )
    .and_then(|row| row.with_limits(key_bytes, bytes))
    .map_err(|_| refused("owner index descriptor"))?;
    Ok((
        OwnerId::new(owner).map_err(|_| refused("owner index owner"))?,
        expected,
        delegate,
    ))
}

/// Each protocol's composition check: the index families it writes are
/// registered with exactly their owners' descriptors, and the Roll holdings
/// family is delegated to the market as the paid-licence pointer is.
pub(super) fn validate_families(
    registry: &StateFamilyRegistry,
    families: &[&str],
) -> Result<(), ActionStateError> {
    for family in families {
        let (owner, expected, delegate) = descriptor(family)?;
        let id = expected.id().clone();
        let actual = registry
            .entries
            .get(&(owner.clone(), id.clone()))
            .ok_or_else(|| refused("owner index family unavailable"))?;
        if actual.descriptor().canonical_value() != expected.canonical_value() {
            return Err(refused("owner index descriptor differs"));
        }
        if let Some(grantee) = delegate
            && !registry.delegates_family(
                &owner,
                &id,
                &OwnerId::new(grantee).map_err(|_| refused("owner index owner"))?,
            )
        {
            return Err(refused("owner index family not delegated"));
        }
    }
    Ok(())
}

/// The clock-free boot audit of the four families (the C2b brief §1–§4),
/// each only where it is registered: an index that disagrees with its heads
/// refuses the boot. Openness that depends on the clock is never checked: an
/// abandoned quote or offer stays indexed until a later write prunes it.
pub(super) fn audit_boot(view: &View<'_>) -> Result<(), ActionStateError> {
    audit_actors(view)?;
    audit_holdings(view)?;
    audit_offers(view, Owner::PaidResale)?;
    audit_offers(view, Owner::CharterPending)
}

fn rows<'v>(view: &'v View<'_>, owner: &str, family: &str) -> impl Iterator<Item = &'v StateKey> {
    let owner = owner.to_owned();
    let family = family.to_owned();
    view.records
        .keys()
        .filter(move |k| k.owner.as_str() == owner && k.family.as_str() == family)
}
fn row_value(view: &View<'_>, k: &StateKey) -> Result<Value, ActionStateError> {
    required(
        view,
        &Key {
            owner: k.owner.clone(),
            family: k.family.clone(),
            encoded: k.encoded.clone(),
        },
    )
}

/// Actors: every indexed quote exists and names that buyer; every indexed
/// operation exists, is funded and names that buyer; every funded
/// entitlement operation is indexed under its buyer.
fn audit_actors(view: &View<'_>) -> Result<(), ActionStateError> {
    if !registered(view.registry, "market", ACTORS) {
        return Ok(());
    }
    let mut indexed = BTreeSet::new();
    for k in rows(view, "market", ACTORS) {
        let row = row_value(view, k)?;
        let actor = person(field(&row, "actor")?)?;
        let quotes = tokens(field(&row, "quotes")?)?;
        let operations = tokens(field(&row, "operations")?)?;
        if row.as_object().is_none_or(|o| o.len() != 4)
            || field(&row, "format")? != &json!(1)
            || k.encoded.as_str() != actor.to_string()
            || (quotes.is_empty() && operations.is_empty())
            || quotes.iter().any(|q| operations.contains(q))
        {
            return Err(refused("owner index actors row differs"));
        }
        for q in &quotes {
            let quote = required(view, &key("market", QUOTE, q.clone())?)?;
            // M8c: an organisation's quote is indexed under its acting
            // character.
            if acting(field(&quote, "buyer")?)? != actor {
                return Err(refused("owner index quote names another buyer"));
            }
        }
        for o in operations {
            let purchase = required(view, &key("market", PURCHASE, o.clone())?)?;
            if field(field(&purchase, "phase")?, "kind")? != "funded"
                || acting(field(field(&purchase, "quote")?, "buyer")?)? != actor
            {
                return Err(refused(
                    "owner index operation is not the buyer's open purchase",
                ));
            }
            indexed.insert(o);
        }
    }
    for k in rows(view, "market", PURCHASE) {
        let purchase = row_value(view, k)?;
        if field(field(&purchase, "phase")?, "kind")? == "funded"
            && !indexed.contains(k.encoded.as_str())
        {
            return Err(refused("owner index omits a funded purchase"));
        }
    }
    Ok(())
}

/// Holdings: exactly one row per paid-licence pointer, under the pointer's
/// current holder, and no row without its pointer.
fn audit_holdings(view: &View<'_>) -> Result<(), ActionStateError> {
    if !registered(view.registry, "roll", HOLDINGS) {
        return Ok(());
    }
    let mut expected = BTreeMap::new();
    for k in rows(view, "roll", LICENCE) {
        let pointer = row_value(view, k)?;
        let licence = decimal(field(&pointer, "id")?, u64::MAX)?;
        let holder = holder(field(&pointer, "holder")?)?;
        expected.insert(holding_key(&holder, licence), holding(&holder, licence));
    }
    for k in rows(view, "roll", HOLDINGS) {
        let row = row_value(view, k)?;
        // M8c F4: the row agrees with its pointer by principal: a renewal
        // re-stamps an organisation-held pointer's `acting` and writes no
        // row, so the row keeps the acting of the grant or transfer that
        // put it.
        let fits = match expected.remove(k.encoded.as_str()) {
            Some(derived) => {
                derived.get("format") == row.get("format")
                    && derived.get("licence") == row.get("licence")
                    && row.as_object().is_some_and(|o| o.len() == 3)
                    && same_holder(field(&derived, "holder")?, field(&row, "holder")?)?
            }
            None => false,
        };
        if !fits {
            return Err(refused("owner index holding has no current pointer"));
        }
    }
    if !expected.is_empty() {
        return Err(refused("owner index omits a held licence"));
    }
    Ok(())
}

/// Offers: each indexed offer exists, names that buyer and licence (or
/// charter), and has no operation. No "every open offer is indexed": whether
/// an offer is still open depends on the clock. A lost row is not harmless,
/// though: the Consume rule needs the row to hold the offer, so that offer's
/// Accept or Fund refuses ("owner index row lacks the consumed offer") until
/// it lapses; no money moves.
fn audit_offers(view: &View<'_>, protocol: Owner) -> Result<(), ActionStateError> {
    let (owner, family, offer_owner, offer_family, operations_owner, operations) = match protocol {
        Owner::PaidResale => (
            "market",
            RESALE_OFFERS,
            "market",
            RESALE_OFFER,
            "roll",
            RESALE,
        ),
        _ => (
            "roll",
            PENDING_OFFERS,
            "roll",
            PENDING_OFFER,
            "roll",
            PENDING,
        ),
    };
    if !registered(view.registry, owner, family) {
        return Ok(());
    }
    let mut used = BTreeSet::new();
    for k in rows(view, operations_owner, operations) {
        used.insert(token(field(&row_value(view, k)?, "offer")?)?);
    }
    for k in rows(view, owner, family) {
        let row = row_value(view, k)?;
        let offers = tokens(field(&row, "offers")?)?;
        if offers.is_empty() {
            return Err(refused("owner index empty row"));
        }
        let mut head = None;
        for o in offers {
            let id = token(&json!(o))?;
            let offer = required(view, &key(offer_owner, offer_family, o)?)?;
            let (expected, fixed) = match protocol {
                Owner::PaidResale => resale_head(&offer)?,
                _ => pending_head(&offer)?,
            };
            if expected.encoded.as_str() != k.encoded.as_str()
                || head.get_or_insert_with(|| fixed.clone()) != &fixed
                || used.contains(&id)
            {
                return Err(refused("owner index offer differs"));
            }
        }
        let fixed = head.ok_or_else(|| refused("owner index empty row"))?;
        let mut whole = fixed;
        if let Some(object) = whole.as_object_mut() {
            object.insert("offers".into(), field(&row, "offers")?.clone());
        }
        if whole != row {
            return Err(refused("owner index offer row differs"));
        }
    }
    Ok(())
}

#[cfg(test)]
pub(super) mod tests;

#[cfg(test)]
mod holder_tests {
    use super::*;
    /// M8c F11: the holder grammar of the index rows: a character's bare
    /// number, an organisation's `{org, acting}` object (organisation 0
    /// included, F8) and nothing else (a third field, a zero acting
    /// character, a string, a zero person); the acting pin; the derived
    /// key and row; sameness by principal.
    #[test]
    fn holder_rows_are_a_person_or_an_organisation_object() {
        let person = holder(&json!(7)).unwrap();
        assert_eq!(person, HolderKey::person(7));
        assert_eq!(
            holding_key(&person, 9),
            format!("{:020}/licence/{:016x}", 7, 9)
        );
        assert_eq!(
            holding(&person, 9),
            json!({"format":1,"holder":7,"licence":"9"})
        );
        let org = holder(&json!({"org":3,"acting":7})).unwrap();
        assert_eq!(org.prefix, format!("org/{:020}", 3));
        assert_eq!(org.value, json!({"org":3,"acting":7}));
        assert_eq!(
            holding_key(&org, 9),
            format!("org/{:020}/licence/{:016x}", 3, 9)
        );
        assert_eq!(
            holding(&org, 9),
            json!({"format":1,"holder":{"org":3,"acting":7},"licence":"9"})
        );
        assert_eq!(
            holder(&json!({"org":0,"acting":7})).unwrap().prefix,
            format!("org/{:020}", 0)
        );
        for bad in [
            json!(0),
            json!(u64::from(u32::MAX) + 1),
            json!({"org":3,"acting":0}),
            json!({"org":3,"acting":7,"x":1}),
            json!({"org":3}),
            json!({"acting":7}),
            json!({"org":"3","acting":7}),
            json!("7"),
            json!(-1),
            json!(null),
        ] {
            assert!(holder(&bad).is_err(), "{bad}");
        }
        assert_eq!(acting(&json!(7)).unwrap(), 7);
        assert_eq!(acting(&json!({"org":3,"acting":7})).unwrap(), 7);
        assert!(acting(&json!({"org":3,"acting":0})).is_err());
        assert!(same_holder(&json!({"org":3,"acting":7}), &json!({"org":3,"acting":8})).unwrap());
        assert!(!same_holder(&json!({"org":3,"acting":7}), &json!({"org":4,"acting":7})).unwrap());
        assert!(!same_holder(&json!({"org":3,"acting":7}), &json!(7)).unwrap());
        assert!(same_holder(&json!(7), &json!(7)).unwrap());
        assert!(!same_holder(&json!(7), &json!(8)).unwrap());
        assert!(same_holder(&json!(7), &json!({"org":3})).is_err());
    }
}
