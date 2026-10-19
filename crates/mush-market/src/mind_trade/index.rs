//! Typed working-set bounds for unfunded offers. These indexes never authorize
//! money or a licence; a managed purchase must consume its quote in the same
//! transaction that creates the complete funded obligation.
use super::records::{MindListing, MindQuote, Row};
use super::{Families, error};
use mush_core::{ActionTxn, CommitToken, ModError, ModuleCtx, ObjId, Registrar, StateFamilyHandle};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

// Same numeric ceilings as the existing Market working sets, separate typed
// counters. No legacy listing/quote allocation or capacity is consumed.
pub const MAX_LISTINGS: usize = 1024;
pub const MAX_QUOTES: usize = 2048;
pub const MAX_SELLER_LISTINGS: usize = 64;
pub const MAX_BUYER_QUOTES: usize = 8;
pub const RETIRE_BATCH: usize = 16;
pub const QUERY_PAGE: usize = 16;

#[derive(Clone)]
pub(super) struct Indices {
    pub counts: StateFamilyHandle<(), Counts>,
    pub actors: StateFamilyHandle<ObjId, ActorOffers>,
    pub expiry: StateFamilyHandle<ExpiryKey, Expiry>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Counts {
    pub listings: u32,
    pub quotes: u32,
}
impl Row for Counts {
    const FAMILY: &'static str = "market_mind_offer_counts";
    const MAX_BYTES: usize = 1024;
    fn validate(&self) -> Result<(), String> {
        if self.listings as usize > MAX_LISTINGS || self.quotes as usize > MAX_QUOTES {
            return Err("original offer capacity exceeded".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ActorOffers {
    pub actor: ObjId,
    pub listings: BTreeSet<CommitToken>,
    pub quotes: BTreeSet<CommitToken>,
}
impl ActorOffers {
    fn empty(actor: ObjId) -> Self {
        Self {
            actor,
            listings: BTreeSet::new(),
            quotes: BTreeSet::new(),
        }
    }
    fn is_empty(&self) -> bool {
        self.listings.is_empty() && self.quotes.is_empty()
    }
}
impl Row for ActorOffers {
    const FAMILY: &'static str = "market_mind_actor_offers";
    const MAX_BYTES: usize = 4096;
    fn validate(&self) -> Result<(), String> {
        if self.actor.0 == 0
            || self.listings.len() > MAX_SELLER_LISTINGS
            || self.quotes.len() > MAX_BUYER_QUOTES
            || !self.listings.is_disjoint(&self.quotes)
        {
            return Err("invalid original offer actor index".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ExpiryKey {
    pub at: i64,
    pub quote: CommitToken,
}
impl ExpiryKey {
    pub fn encoded(&self) -> String {
        // Signed UTC order, without a fallible subtraction at i64::MIN.
        format!(
            "{:016x}/{}",
            (self.at as u64) ^ (1u64 << 63),
            self.quote.to_hex()
        )
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Expiry {
    pub quote: CommitToken,
    pub buyer: ObjId,
    pub at: i64,
}
impl Expiry {
    pub fn key(&self) -> ExpiryKey {
        ExpiryKey {
            at: self.at,
            quote: self.quote,
        }
    }
    fn of(quote: &MindQuote) -> Self {
        Self {
            quote: quote.publication,
            buyer: quote.buyer,
            at: quote.expires_at_utc,
        }
    }
}
impl Row for Expiry {
    const FAMILY: &'static str = "market_mind_quote_expiry";
    const MAX_BYTES: usize = 1024;
    fn validate(&self) -> Result<(), String> {
        if self.buyer.0 == 0 {
            return Err("invalid original quote expiry owner".into());
        }
        Ok(())
    }
}

fn family<K: Send + Sync + 'static, R: Row>(
    reg: &mut Registrar<'_>,
    key: impl Fn(&K) -> Result<String, String> + Send + Sync + 'static,
) -> Result<StateFamilyHandle<K, R>, ModError> {
    use mush_core::mechanics::CanonicalId;
    use mush_core::{StateFamilyDescriptor, StateFamilyId};
    let id = |suffix: &str| CanonicalId::new(format!("{}.{}.v1", R::FAMILY, suffix)).map_err(error);
    let descriptor = StateFamilyDescriptor::new(
        StateFamilyId::new(R::FAMILY).map_err(error)?,
        id("key")?,
        id("value")?,
        1,
        id("decode")?,
        id("validate")?,
        id("canonical")?,
    )
    .and_then(|d| d.with_limits(64, R::MAX_BYTES))
    .map_err(error)?;
    reg.state_family(
        descriptor,
        key,
        super::records::decode_value::<R>,
        R::validate,
        |value: &R| serde_json::to_value(value).map_err(|e| e.to_string()),
    )
}
pub(super) fn register(reg: &mut Registrar<'_>) -> Result<Indices, ModError> {
    Ok(Indices {
        counts: family(reg, |_: &()| Ok("capacity".into()))?,
        actors: family(reg, |actor: &ObjId| Ok(actor.0.to_string()))?,
        expiry: family(reg, |key: &ExpiryKey| Ok(key.encoded()))?,
    })
}

fn counts(txn: &mut ActionTxn<'_>, f: &Families) -> Result<(bool, Counts), String> {
    let row = txn
        .get(&f.indices.counts, &())
        .map_err(|e| e.to_string())?
        .cloned();
    let exists = row.is_some();
    let row = row.unwrap_or_default();
    row.validate()?;
    Ok((exists, row))
}
fn actor(
    txn: &mut ActionTxn<'_>,
    f: &Families,
    actor: ObjId,
) -> Result<(bool, ActorOffers), String> {
    let row = txn
        .get(&f.indices.actors, &actor)
        .map_err(|e| e.to_string())?
        .cloned();
    let exists = row.is_some();
    let row = row.unwrap_or_else(|| ActorOffers::empty(actor));
    row.validate()?;
    if row.actor != actor {
        return Err("crossed original actor index".into());
    }
    Ok((exists, row))
}
fn put_counts(
    txn: &mut ActionTxn<'_>,
    f: &Families,
    exists: bool,
    row: Counts,
) -> Result<(), String> {
    row.validate()?;
    if exists {
        txn.replace(&f.indices.counts, &(), row)
    } else {
        txn.insert(&f.indices.counts, &(), row)
    }
    .map_err(|e| e.to_string())
}
fn put_actor(
    txn: &mut ActionTxn<'_>,
    f: &Families,
    exists: bool,
    row: ActorOffers,
) -> Result<(), String> {
    row.validate()?;
    if row.is_empty() {
        if exists {
            txn.remove(&f.indices.actors, &row.actor)
                .map_err(|e| e.to_string())?;
        }
        return Ok(());
    }
    let key = row.actor;
    if exists {
        txn.replace(&f.indices.actors, &key, row)
    } else {
        txn.insert(&f.indices.actors, &key, row)
    }
    .map_err(|e| e.to_string())
}
pub(super) fn publish_listing(
    txn: &mut ActionTxn<'_>,
    f: &Families,
    row: MindListing,
) -> Result<(), String> {
    row.validate()?;
    let (present, mut count) = counts(txn, f)?;
    let (actor_present, mut owner) = actor(txn, f, row.subject.seller)?;
    if count.listings as usize >= MAX_LISTINGS
        || owner.listings.len() >= MAX_SELLER_LISTINGS
        || (!present && !owner.is_empty())
        || !owner.listings.insert(row.publication)
    {
        return Err("original listing capacity unavailable".into());
    }
    count.listings = count
        .listings
        .checked_add(1)
        .ok_or("listing capacity overflow")?;
    let key = row.publication;
    txn.insert(&f.listings, &key, row)
        .map_err(|e| e.to_string())?;
    put_counts(txn, f, present, count)?;
    put_actor(txn, f, actor_present, owner)
}
pub(super) fn publish_quote(
    txn: &mut ActionTxn<'_>,
    f: &Families,
    row: MindQuote,
) -> Result<(), String> {
    row.validate()?;
    let (present, mut count) = counts(txn, f)?;
    let (actor_present, mut owner) = actor(txn, f, row.buyer)?;
    if count.quotes as usize >= MAX_QUOTES
        || owner.quotes.len() >= MAX_BUYER_QUOTES
        || (!present && !owner.is_empty())
        || !owner.quotes.insert(row.publication)
    {
        return Err("original quote capacity unavailable".into());
    }
    count.quotes = count
        .quotes
        .checked_add(1)
        .ok_or("quote capacity overflow")?;
    let expiry = Expiry::of(&row);
    let key = row.publication;
    txn.insert(&f.quotes, &key, row)
        .map_err(|e| e.to_string())?;
    txn.insert(&f.indices.expiry, &expiry.key(), expiry)
        .map_err(|e| e.to_string())?;
    put_counts(txn, f, present, count)?;
    put_actor(txn, f, actor_present, owner)
}
pub(super) fn retire_listing(
    txn: &mut ActionTxn<'_>,
    f: &Families,
    key: CommitToken,
    seller: ObjId,
) -> Result<(), String> {
    let row = txn
        .get(&f.listings, &key)
        .map_err(|e| e.to_string())?
        .cloned()
        .ok_or("missing listing")?;
    row.validate()?;
    if row.publication != key || row.subject.seller != seller {
        return Err("listing unavailable".into());
    }
    let (present, mut count) = counts(txn, f)?;
    let (actor_present, mut owner) = actor(txn, f, seller)?;
    if !present || !actor_present || !owner.listings.remove(&key) {
        return Err("missing listing capacity".into());
    }
    count.listings = count
        .listings
        .checked_sub(1)
        .ok_or("listing capacity underflow")?;
    txn.remove(&f.listings, &key).map_err(|e| e.to_string())?;
    put_counts(txn, f, true, count)?;
    put_actor(txn, f, true, owner)
}

/// Retire only these unfunded quotes. `buyer=None` is the fixed expiry action,
/// which has already required GOD and now independently checks every deadline.
/// One post-image per actor/count key; repeated same-key txn mutations refuse.
pub(super) fn retire_quotes(
    txn: &mut ActionTxn<'_>,
    f: &Families,
    keys: &[CommitToken],
    buyer: Option<ObjId>,
    now: i64,
) -> Result<(), String> {
    if keys.is_empty()
        || keys.len() > RETIRE_BATCH
        || keys.iter().copied().collect::<BTreeSet<_>>().len() != keys.len()
    {
        return Err("invalid original quote retirement batch".into());
    }
    let (present, mut count) = counts(txn, f)?;
    if !present {
        return Err("missing quote capacity".into());
    }
    let mut actors = BTreeMap::<ObjId, ActorOffers>::new();
    let mut rows = Vec::with_capacity(keys.len());
    for key in keys {
        let row = txn
            .get(&f.quotes, key)
            .map_err(|e| e.to_string())?
            .cloned()
            .ok_or("missing quote")?;
        row.validate()?;
        if row.publication != *key
            || buyer.is_some_and(|id| id != row.buyer)
            || (buyer.is_none() && now < row.expires_at_utc)
        {
            return Err("quote retirement unavailable".into());
        }
        let expected = Expiry::of(&row);
        if txn
            .get(&f.indices.expiry, &expected.key())
            .map_err(|e| e.to_string())?
            != Some(&expected)
        {
            return Err("missing quote expiry index".into());
        }
        if let std::collections::btree_map::Entry::Vacant(entry) = actors.entry(row.buyer) {
            let (exists, owner) = actor(txn, f, row.buyer)?;
            if !exists {
                return Err("missing buyer index".into());
            }
            entry.insert(owner);
        }
        if !actors
            .get_mut(&row.buyer)
            .is_some_and(|a| a.quotes.remove(key))
        {
            return Err("missing quote in buyer index".into());
        }
        count.quotes = count
            .quotes
            .checked_sub(1)
            .ok_or("quote capacity underflow")?;
        rows.push(expected);
    }
    for row in rows {
        txn.remove(&f.quotes, &row.quote)
            .map_err(|e| e.to_string())?;
        txn.remove(&f.indices.expiry, &row.key())
            .map_err(|e| e.to_string())?;
    }
    put_counts(txn, f, true, count)?;
    for (_, owner) in actors {
        put_actor(txn, f, true, owner)?;
    }
    Ok(())
}

fn scan<K: Send + Sync + 'static, R: Row>(
    ctx: &ModuleCtx<'_>,
    family: &StateFamilyHandle<K, R>,
    limit: usize,
) -> Result<Vec<(String, R)>, ModError> {
    let mut all = Vec::new();
    let mut after = None;
    loop {
        let page = ctx
            .scan_action_state(family, after.as_deref(), 32)
            .map_err(error)?;
        for entry in page.entries() {
            if all.len() >= limit {
                return Err(ModError::msg("original offer recovery bound exceeded"));
            }
            entry.value().validate().map_err(error)?;
            all.push((entry.encoded_key().to_owned(), entry.value().clone()));
        }
        let Some(next) = page.next_after() else {
            break;
        };
        after = Some(next.to_string());
    }
    Ok(all)
}
pub(super) fn audit(ctx: &ModuleCtx<'_>, f: &Families) -> Result<(), ModError> {
    let listings = scan(ctx, &f.listings, MAX_LISTINGS)?;
    let quotes = scan(ctx, &f.quotes, MAX_QUOTES)?;
    let mut actors = BTreeMap::<ObjId, ActorOffers>::new();
    let mut expiry = BTreeMap::new();
    for (key, row) in &listings {
        if *key != row.publication.to_hex() {
            return Err(ModError::msg("listing key differs"));
        }
        actors
            .entry(row.subject.seller)
            .or_insert_with(|| ActorOffers::empty(row.subject.seller))
            .listings
            .insert(row.publication);
    }
    for (key, row) in &quotes {
        if *key != row.publication.to_hex() {
            return Err(ModError::msg("quote key differs"));
        }
        actors
            .entry(row.buyer)
            .or_insert_with(|| ActorOffers::empty(row.buyer))
            .quotes
            .insert(row.publication);
        let value = Expiry::of(row);
        expiry.insert(value.key().encoded(), value);
    }
    for value in actors.values() {
        value.validate().map_err(error)?;
    }
    let actual_actors = scan(ctx, &f.indices.actors, MAX_LISTINGS + MAX_QUOTES)?;
    if actual_actors.len() != actors.len() {
        return Err(ModError::msg("original actor index incomplete"));
    }
    for (key, value) in actual_actors {
        if key != value.actor.0.to_string() || actors.get(&value.actor) != Some(&value) {
            return Err(ModError::msg("original actor index differs"));
        }
    }
    let actual_expiry: BTreeMap<_, _> = scan(ctx, &f.indices.expiry, MAX_QUOTES)?
        .into_iter()
        .collect();
    if actual_expiry != expiry {
        return Err(ModError::msg("original quote expiry index differs"));
    }
    let expected = Counts {
        listings: listings.len() as u32,
        quotes: quotes.len() as u32,
    };
    let actual_counts = scan(ctx, &f.indices.counts, 1)?;
    match actual_counts.first() {
        Some((key, value)) if key == "capacity" && value == &expected => Ok(()),
        None if expected == Counts::default() => Ok(()),
        _ => Err(ModError::msg("original offer counts differ")),
    }
}
