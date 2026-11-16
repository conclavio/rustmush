//! **`@econ/census`: the money census** (doc 48 cut EX.0; Q-48-17, Q-48-36).
//!
//! The matter census (`@industry/census`) counts kilograms; this one counts money. For each IC
//! day it shows where money came from, where it went, and who holds it:
//!
//! - **Sources**: the System accounts whose balance fell over the day. Only System accounts may
//!   go negative, so a fall is a mint: the account's **net mint** for the day.
//! - **Sinks**: the System accounts whose balance rose: the fees and levies they received.
//! - **Holdings by category** at the day's close: lane institutions, Houses, other
//!   organisations, players, and everything else (NPC purses, objects).
//!
//! **How a day is measured.** The census reads balances, not the journal. The economy sweep
//! (its census step) takes one reading of every account when the IC day changes. The difference
//! between two readings is that row's flows: each System account's change, and the holdings'
//! change. The journal's entries carry a wall-clock stamp only, and the IC clock is re-anchored
//! across a restart, so a post's IC day cannot be recovered afterwards. Two readings can.
//!
//! **The identity a row keeps.** Every instrument is zero-sum, so the holdings plus every
//! System balance is zero. Over a row, the System side's net mint is the holdings' change.
//! A row prints the difference as `drift`, which is 0 unless the scan stopped at its bound.
//!
//! **Bounds.** One reading scans at most [`MAX_SCAN`] accounts, only when a day turns, at the
//! sweep's 30-second pace. The book keeps [`MAX_ROWS`] rows an instrument ([`MAX_INSTRUMENTS`]
//! currency instruments at most), in one `census` document written once a row. The verb shows
//! at most [`MAX_SHOWN`] rows and [`MAX_ACCOUNT_LINES`] accounts a side a row, at 120 columns.
//!
//! **Categories.** A System account is a flow, never a holding. An organisation's category is
//! the game's word ([`MoneyCensusHook`]): a game installs the hook and names its lane
//! institutions and Houses. Without it every organisation is `org`. An object principal is a
//! `player` when it is a player object, `other` otherwise.

use std::collections::{BTreeMap, VecDeque};

use mush_core::{CivilDate, CmdError, Ctx, ModuleWrite, ObjId, ObjKind, World};
use mush_org::{OrgId, PrincipalId};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::ids::{AccountId, InstrumentId};
use crate::module::EconomyStore;
use crate::record::InstrumentKind;

#[cfg(test)]
mod tests;

/// The census document's key, in the economy namespace.
pub const CENSUS_KEY: &str = "census";
/// The document's schema.
pub const CENSUS_SCHEMA: u16 = 1;
/// IC seconds in one IC day.
pub const IC_DAY_SECS: i64 = 86_400;
/// Rows kept an instrument: a week of real time at the live 24:1 rate.
pub const MAX_ROWS: usize = 168;
/// Currency instruments the census follows at most.
pub const MAX_INSTRUMENTS: usize = 4;
/// Accounts one reading scans at most.
pub const MAX_SCAN: usize = 262_144;
/// System accounts one reading keeps at most (one more is summed, not named).
pub const MAX_SYSTEM: usize = 256;
/// Rows the verb shows by default.
pub const DEFAULT_SHOWN: usize = 7;
/// Rows the verb shows at most.
pub const MAX_SHOWN: usize = 28;
/// Accounts a side the verb names a row, then one `+N more` line.
pub const MAX_ACCOUNT_LINES: usize = 8;
/// The widest line the verb prints.
pub const MAX_COLUMNS: usize = 120;
/// The longest account label the verb prints.
const MAX_LABEL: usize = 48;
/// A row's lead word (what a story reads).
pub const DAY_LINE: &str = "econ.census_day";
/// The window's lead word.
pub const WINDOW_LINE: &str = "econ.census_window";

// ---- the categories ------------------------------------------------------------------------

/// **A holding's category** in the money census.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HoldingClass {
    /// A lane institution: an organisation the game's ports made.
    Lane,
    /// A House.
    House,
    /// Any other organisation.
    Org,
    /// A player character.
    Player,
    /// Any other object: an NPC's purse, a thing.
    Other,
}

impl HoldingClass {
    /// Every category, in print order.
    pub const ALL: [Self; 5] = [
        Self::Lane,
        Self::House,
        Self::Org,
        Self::Player,
        Self::Other,
    ];

    /// The word the verb prints.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Lane => "lane",
            Self::House => "house",
            Self::Org => "org",
            Self::Player => "player",
            Self::Other => "other",
        }
    }
}

/// **The game's word on its organisations** (Q-48-17's categories). A game installs it as
/// module state; `org_classes` names each lane institution and House, bounded by the game's
/// own content. An organisation it does not name is [`HoldingClass::Org`].
#[derive(Clone, Copy)]
pub struct MoneyCensusHook {
    /// The organisations the game names, and their categories.
    pub org_classes: fn(&World) -> BTreeMap<OrgId, HoldingClass>,
}

impl std::fmt::Debug for MoneyCensusHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MoneyCensusHook")
    }
}

/// **The category of an account's owner**; `None` for a System principal, which is a flow.
#[must_use]
pub fn classify(
    owner: &PrincipalId,
    orgs: &BTreeMap<OrgId, HoldingClass>,
    is_player: impl Fn(ObjId) -> bool,
) -> Option<HoldingClass> {
    match owner {
        PrincipalId::System(_) => None,
        PrincipalId::Organization(org) => Some(orgs.get(org).copied().unwrap_or(HoldingClass::Org)),
        PrincipalId::Object(id) if is_player(*id) => Some(HoldingClass::Player),
        PrincipalId::Object(_) => Some(HoldingClass::Other),
    }
}

/// **Money held, by category**, minor units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Holdings {
    /// Lane institutions.
    pub lane: i64,
    /// Houses.
    pub house: i64,
    /// Other organisations.
    pub org: i64,
    /// Players.
    pub player: i64,
    /// Everything else.
    pub other: i64,
}

impl Holdings {
    /// One category's amount.
    #[must_use]
    pub const fn get(&self, class: HoldingClass) -> i64 {
        match class {
            HoldingClass::Lane => self.lane,
            HoldingClass::House => self.house,
            HoldingClass::Org => self.org,
            HoldingClass::Player => self.player,
            HoldingClass::Other => self.other,
        }
    }

    /// Add `minor` to one category, saturating.
    pub fn add(&mut self, class: HoldingClass, minor: i64) {
        let slot = match class {
            HoldingClass::Lane => &mut self.lane,
            HoldingClass::House => &mut self.house,
            HoldingClass::Org => &mut self.org,
            HoldingClass::Player => &mut self.player,
            HoldingClass::Other => &mut self.other,
        };
        *slot = slot.saturating_add(minor);
    }

    /// Every category summed.
    #[must_use]
    pub fn total(&self) -> i128 {
        HoldingClass::ALL
            .iter()
            .map(|class| i128::from(self.get(*class)))
            .sum()
    }
}

// ---- readings, rows and the book ------------------------------------------------------------

/// **One reading of an instrument**: every System account's balance and the holdings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reading {
    /// System balances, minor units, in account order.
    pub system: BTreeMap<AccountId, i64>,
    /// The System balance of accounts past [`MAX_SYSTEM`], summed.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub system_rest: i64,
    /// The holdings.
    pub holdings: Holdings,
    /// Whether the scan stopped at [`MAX_SCAN`].
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub partial: bool,
}

#[allow(clippy::trivially_copy_pass_by_ref)] // serde's skip predicate takes a reference
const fn is_zero(v: &i64) -> bool {
    *v == 0
}

/// **Where a row opened**: the IC day and instants, and the reading.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Open {
    /// The IC day the reading fell in.
    pub day: i64,
    /// The IC second of the reading.
    pub at_ic: i64,
    /// The wall-clock second of the reading.
    pub at_real: i64,
    /// The reading.
    pub reading: Reading,
}

/// **One System account's change over a row**, minor units: negative is a mint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Flow {
    /// The account.
    pub account: AccountId,
    /// Its balance's change.
    pub delta: i64,
}

/// **One row**: the IC days between two readings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row {
    /// The first IC day the row covers.
    pub day: i64,
    /// IC days it covers: 1 when the sweep read each day's turn; more when it did not.
    pub days: i64,
    /// The IC second it opened.
    pub opened_ic: i64,
    /// The IC second it closed.
    pub closed_ic: i64,
    /// The wall-clock second it closed.
    pub closed_real: i64,
    /// Every System account that moved, in account order.
    pub flows: Vec<Flow>,
    /// The change of the System accounts past [`MAX_SYSTEM`], summed.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub flow_rest: i64,
    /// The holdings when it opened.
    pub opening: Holdings,
    /// The holdings when it closed.
    pub closing: Holdings,
    /// Whether either reading stopped at [`MAX_SCAN`].
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub partial: bool,
}

impl Row {
    /// **The System side's gross mint**: every fall, summed.
    #[must_use]
    pub fn mint(&self) -> i128 {
        self.deltas().filter(|d| *d < 0).map(|d| -d).sum()
    }

    /// **The sinks' intake**: every rise, summed.
    #[must_use]
    pub fn sink(&self) -> i128 {
        self.deltas().filter(|d| *d > 0).sum()
    }

    /// **The net mint**: mint less intake, what left the System side.
    #[must_use]
    pub fn net(&self) -> i128 {
        self.mint() - self.sink()
    }

    /// **The identity's error**: the net mint less the holdings' change; 0 on a whole scan.
    #[must_use]
    pub fn drift(&self) -> i128 {
        self.net() - (self.closing.total() - self.opening.total())
    }

    fn deltas(&self) -> impl Iterator<Item = i128> + '_ {
        self.flows
            .iter()
            .map(|flow| i128::from(flow.delta))
            .chain(std::iter::once(i128::from(self.flow_rest)))
    }
}

/// **The row between `open` and a later reading**, the pure close. The live view uses it for
/// the day still open.
#[must_use]
pub fn close(open: &Open, reading: &Reading, day: i64, at_ic: i64, at_real: i64) -> Row {
    let before = &open.reading.system;
    let mut flows = Vec::new();
    let mut keys: Vec<AccountId> = before
        .keys()
        .chain(reading.system.keys())
        .copied()
        .collect();
    keys.sort_unstable();
    keys.dedup();
    for account in keys {
        let was = before.get(&account).copied().unwrap_or(0);
        let now = reading.system.get(&account).copied().unwrap_or(0);
        let delta = now.saturating_sub(was);
        if delta != 0 {
            flows.push(Flow { account, delta });
        }
    }
    Row {
        day: open.day,
        days: day.saturating_sub(open.day),
        opened_ic: open.at_ic,
        closed_ic: at_ic,
        closed_real: at_real,
        flows,
        flow_rest: reading.system_rest.saturating_sub(open.reading.system_rest),
        opening: open.reading.holdings,
        closing: reading.holdings,
        partial: open.reading.partial || reading.partial,
    }
}

/// **One instrument's census**: the open reading and the closed rows, oldest first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ledger {
    /// The reading the next row opens from.
    pub open: Option<Open>,
    /// The closed rows, at most [`MAX_ROWS`].
    pub rows: VecDeque<Row>,
}

impl Ledger {
    /// Whether a reading is due at IC day `day`: none taken yet, or the day has turned.
    #[must_use]
    pub fn due(&self, day: i64) -> bool {
        self.open.as_ref().is_none_or(|open| open.day != day)
    }

    /// **Take a reading at IC day `day`.** The first opens the ledger. A later day closes a
    /// row and opens the next from the same reading. An earlier day (the IC clock moved back)
    /// re-opens from the reading and closes nothing. Returns whether the ledger changed.
    pub fn advance(&mut self, reading: Reading, day: i64, at_ic: i64, at_real: i64) -> bool {
        let row = match &self.open {
            Some(open) if open.day == day => return false,
            Some(open) if open.day < day => Some(close(open, &reading, day, at_ic, at_real)),
            _ => None,
        };
        if let Some(row) = row {
            self.rows.push_back(row);
            while self.rows.len() > MAX_ROWS {
                self.rows.pop_front();
            }
        }
        self.open = Some(Open {
            day,
            at_ic,
            at_real,
            reading,
        });
        true
    }
}

/// **The census book**: one [`Ledger`] a currency instrument. Economy module state, and the
/// `census` document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CensusBook {
    /// [`CENSUS_SCHEMA`].
    #[serde(default)]
    pub schema: u16,
    /// The ledgers, by instrument.
    pub ledgers: BTreeMap<InstrumentId, Ledger>,
}

impl Default for CensusBook {
    fn default() -> Self {
        Self {
            schema: CENSUS_SCHEMA,
            ledgers: BTreeMap::new(),
        }
    }
}

impl CensusBook {
    /// **The book a boot reads** from the economy's documents: the `census` document, or an
    /// empty book when there is none or it does not decode (logged; the census restarts, and
    /// no money moves on it).
    #[must_use]
    pub fn load(kv: &[(SmolStr, serde_json::Value)]) -> Self {
        let Some((_, value)) = kv.iter().find(|(key, _)| key == CENSUS_KEY) else {
            return Self::default();
        };
        match serde_json::from_value::<Self>(value.clone()) {
            Ok(book) if book.schema == CENSUS_SCHEMA => book,
            Ok(book) => {
                tracing::warn!(
                    schema = book.schema,
                    "economy: the census document has another schema; the census restarts"
                );
                Self::default()
            }
            Err(error) => {
                tracing::warn!(%error, "economy: the census document does not decode; the census restarts");
                Self::default()
            }
        }
    }

    /// The document.
    #[must_use]
    pub fn to_doc(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

/// The IC day an IC second falls in.
#[must_use]
pub const fn ic_day(ic: i64) -> i64 {
    ic.div_euclid(IC_DAY_SECS)
}

// ---- the world reads ------------------------------------------------------------------------

/// The currency instruments the census follows, in id order, at most [`MAX_INSTRUMENTS`].
#[must_use]
pub fn currencies(store: &EconomyStore) -> Vec<InstrumentId> {
    store
        .snapshot()
        .instruments()
        .filter(|def| def.kind == InstrumentKind::Currency)
        .map(|def| def.id.clone())
        .take(MAX_INSTRUMENTS)
        .collect()
}

/// The game's organisation categories, empty without its hook.
#[must_use]
pub fn org_classes(w: &World) -> BTreeMap<OrgId, HoldingClass> {
    w.module_state::<MoneyCensusHook>()
        .map(|hook| (hook.org_classes)(w))
        .unwrap_or_default()
}

/// **One reading of every instrument in `instruments`**, in one scan of at most [`MAX_SCAN`]
/// accounts.
#[must_use]
pub fn read(
    w: &World,
    store: &EconomyStore,
    instruments: &[InstrumentId],
    orgs: &BTreeMap<OrgId, HoldingClass>,
) -> BTreeMap<InstrumentId, Reading> {
    let mut out: BTreeMap<InstrumentId, Reading> = instruments
        .iter()
        .map(|id| (id.clone(), Reading::default()))
        .collect();
    let is_player = |id: ObjId| w.get(id).is_some_and(|o| o.kind == ObjKind::Player);
    let mut scanned = 0usize;
    for account in store.snapshot().accounts() {
        let Some(reading) = out.get_mut(&account.instrument) else {
            continue;
        };
        if scanned >= MAX_SCAN {
            reading.partial = true;
            continue;
        }
        scanned = scanned.saturating_add(1);
        let minor = account.posted_balance.minor();
        match classify(&account.owner, orgs, is_player) {
            Some(class) => reading.holdings.add(class, minor),
            None if reading.system.len() < MAX_SYSTEM => {
                reading.system.insert(account.id, minor);
            }
            None => reading.system_rest = reading.system_rest.saturating_add(minor),
        }
    }
    out
}

/// **The sweep's census step** (doc 48 cut EX.0): when an instrument's IC day has turned (or
/// it has no reading yet), read every currency account once and advance its ledger, then
/// write the `census` document. A day that has not turned costs one comparison an instrument.
pub(crate) fn sample(ctx: &mut Ctx<'_>) {
    let at_ic = ctx.w.game_now_unix();
    let at_real = ctx.w.now();
    let day = ic_day(at_ic);
    let readings = {
        let (Some(store), Some(book)) = (
            ctx.w.module_state::<EconomyStore>(),
            ctx.w.module_state::<CensusBook>(),
        ) else {
            return;
        };
        let instruments: Vec<InstrumentId> = currencies(store)
            .into_iter()
            .filter(|id| book.ledgers.get(id).is_none_or(|ledger| ledger.due(day)))
            .collect();
        if instruments.is_empty() {
            return;
        }
        read(ctx.w, store, &instruments, &org_classes(ctx.w))
    };
    let doc = {
        let Some(book) = ctx.w.module_state_mut::<CensusBook>() else {
            return;
        };
        let mut changed = false;
        for (instrument, reading) in readings {
            changed |= book
                .ledgers
                .entry(instrument)
                .or_default()
                .advance(reading, day, at_ic, at_real);
        }
        if !changed {
            return;
        }
        book.to_doc()
    };
    crate::api::push_op(
        ctx.out,
        ModuleWrite::JsonSet {
            key: SmolStr::new_static(CENSUS_KEY),
            value: doc,
        },
    );
}

// ---- the view -------------------------------------------------------------------------------

/// The IC date of an IC day, `YYYY-MM-DD`.
#[must_use]
pub fn date(day: i64) -> String {
    let civil = CivilDate::from_ic_seconds(day.saturating_mul(IC_DAY_SECS));
    format!("{}-{:02}-{:02}", civil.year, civil.month, civil.day)
}

/// One account's printed label: `#12 sys:tacsim.treasury`, cut to 48 characters.
#[must_use]
pub fn label(store: &EconomyStore, account: AccountId) -> String {
    let name = store
        .snapshot()
        .account(account)
        .map(|acct| match &acct.owner {
            PrincipalId::System(id) if acct.name.as_str() == id.as_str() => {
                format!("sys:{}", id.as_str())
            }
            PrincipalId::System(id) => format!("sys:{} {}", id.as_str(), acct.name),
            _ => acct.name.to_string(),
        })
        .unwrap_or_default();
    let text = format!("#{} {name}", account.0);
    text.chars().take(MAX_LABEL).collect()
}

/// **Fit a `key=value` line to [`MAX_COLUMNS`]**: words carry on over indented lines.
#[must_use]
pub fn fit(line: &str) -> Vec<String> {
    if line.chars().count() <= MAX_COLUMNS {
        return vec![line.to_owned()];
    }
    let mut out = Vec::new();
    let mut current = String::new();
    for word in line.split(' ') {
        let width = current.chars().count();
        if !current.is_empty()
            && width.saturating_add(1).saturating_add(word.chars().count()) > MAX_COLUMNS
        {
            out.push(std::mem::replace(&mut current, "    ".to_owned()));
        }
        if !current.is_empty() && !current.ends_with(' ') {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.trim().is_empty() {
        out.push(current);
    }
    out
}

/// The label of the System accounts past [`MAX_SYSTEM`], summed.
pub const REST_LABEL: &str = "the other System accounts";

/// One side's amounts: an account, or `None` for the accounts past [`MAX_SYSTEM`].
type Side = Vec<(Option<AccountId>, i128)>;

/// One side's account lines of a row: the largest first, at most [`MAX_ACCOUNT_LINES`].
fn side_lines(
    out: &mut Vec<String>,
    word: &str,
    key: &str,
    flows: &[(Option<AccountId>, i128)],
    labels: &dyn Fn(AccountId) -> String,
) {
    let mut sorted: Vec<&(Option<AccountId>, i128)> = flows.iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (account, amount) in sorted.iter().take(MAX_ACCOUNT_LINES) {
        let name = account.map_or_else(|| REST_LABEL.to_owned(), labels);
        out.extend(fit(&format!("  {word} {name} {key}={amount}")));
    }
    if sorted.len() > MAX_ACCOUNT_LINES {
        let rest: i128 = sorted.iter().skip(MAX_ACCOUNT_LINES).map(|(_, a)| *a).sum();
        out.push(format!(
            "  {word} +{} more {key}={rest}",
            sorted.len().saturating_sub(MAX_ACCOUNT_LINES)
        ));
    }
}

/// A row's sources and sinks, as positive amounts.
fn sides(flows: impl Iterator<Item = (Option<AccountId>, i128)>) -> (Side, Side) {
    let mut sources = Vec::new();
    let mut sinks = Vec::new();
    for (account, delta) in flows {
        if delta < 0 {
            sources.push((account, -delta));
        } else if delta > 0 {
            sinks.push((account, delta));
        }
    }
    (sources, sinks)
}

/// **A row's lines**: the day line, the holdings line, then its sources and sinks.
#[must_use]
pub fn row_lines(row: &Row, open: bool, labels: &dyn Fn(AccountId) -> String) -> Vec<String> {
    let mut out = Vec::new();
    let mut head = format!(
        "{DAY_LINE} {} days={} mint={} sink={} net={} drift={}",
        date(row.day),
        row.days,
        row.mint(),
        row.sink(),
        row.net(),
        row.drift()
    );
    if row.partial {
        head.push_str(" partial");
    }
    if open {
        head.push_str(" open");
    }
    out.extend(fit(&head));
    let mut held = String::from("  held");
    for class in HoldingClass::ALL {
        held.push_str(&format!(" {}={}", class.word(), row.closing.get(class)));
    }
    held.push_str(&format!(" total={}", row.closing.total()));
    out.extend(fit(&held));
    let (sources, sinks) = sides(
        row.flows
            .iter()
            .map(|flow| (Some(flow.account), i128::from(flow.delta)))
            .chain(std::iter::once((None, i128::from(row.flow_rest)))),
    );
    side_lines(&mut out, "source", "mint", &sources, labels);
    side_lines(&mut out, "sink", "intake", &sinks, labels);
    out
}

/// **The window's lines**: the rows' totals, and each System account's net over them, the
/// faucets' net mint first.
#[must_use]
pub fn window_lines(rows: &[&Row], labels: &dyn Fn(AccountId) -> String) -> Vec<String> {
    let (Some(first), Some(last)) = (rows.first(), rows.last()) else {
        return Vec::new();
    };
    let mut by_account: BTreeMap<AccountId, i128> = BTreeMap::new();
    let mut rest = 0i128;
    let (mut mint, mut sink, mut days) = (0i128, 0i128, 0i64);
    for row in rows {
        for flow in &row.flows {
            *by_account.entry(flow.account).or_default() += i128::from(flow.delta);
        }
        rest += i128::from(row.flow_rest);
        mint += row.mint();
        sink += row.sink();
        days = days.saturating_add(row.days);
    }
    let mut out = fit(&format!(
        "{WINDOW_LINE} {}..{} days={days} mint={mint} sink={sink} net={}",
        date(first.day),
        date(last.day.saturating_add(last.days.max(1)).saturating_sub(1)),
        mint - sink
    ));
    let (faucets, sinks) = sides(
        by_account
            .into_iter()
            .map(|(account, net)| (Some(account), net))
            .chain(std::iter::once((None, rest))),
    );
    side_lines(&mut out, "faucet", "mint", &faucets, labels);
    side_lines(&mut out, "sink", "intake", &sinks, labels);
    out
}

/// **The verb's lines for one instrument**: the last `shown` rows and the open day, then the
/// window.
#[must_use]
pub fn instrument_lines(
    instrument: &InstrumentId,
    ledger: &Ledger,
    live: Option<&Row>,
    shown: usize,
    labels: &dyn Fn(AccountId) -> String,
) -> Vec<String> {
    let shown = shown.clamp(1, MAX_SHOWN);
    let skip = ledger.rows.len().saturating_sub(shown);
    let rows: Vec<&Row> = ledger.rows.iter().skip(skip).collect();
    let mut out = fit(&format!(
        "econ.census instrument={} rows={} shown={} kept={MAX_ROWS}",
        instrument.as_str(),
        ledger.rows.len(),
        rows.len()
    ));
    for row in &rows {
        out.extend(row_lines(row, false, labels));
    }
    if let Some(row) = live {
        out.extend(row_lines(row, true, labels));
    }
    out.extend(window_lines(&rows, labels));
    out
}

/// The verb's header line.
pub const HEADER: &str = "Money census, minor units: per IC day, the System accounts' mint and \
                          intake, and holdings by category.";

/// **`@econ/census [<days>]`** (staff; the caller checks): the last `days` rows (default 7,
/// at most 28) of every currency instrument, the day still open, and the window's totals.
///
/// # Errors
///
/// A bad count, or no economy.
pub(crate) fn census_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @econ/census [<days>]   (days is 1-28, default 7)";
    let raw = ctx.args.lhs().trim().to_owned();
    let shown = if raw.is_empty() {
        DEFAULT_SHOWN
    } else {
        raw.parse::<usize>()
            .ok()
            .filter(|n| (1..=MAX_SHOWN).contains(n))
            .ok_or_else(|| CmdError::msg(USAGE))?
    };
    let lines = {
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or_else(|| CmdError::msg("No economy."))?;
        let empty = CensusBook::default();
        let book = ctx.w.module_state::<CensusBook>().unwrap_or(&empty);
        let at_ic = ctx.w.game_now_unix();
        let day = ic_day(at_ic);
        let instruments = currencies(store);
        let readings = read(ctx.w, store, &instruments, &org_classes(ctx.w));
        let labels = |account: AccountId| label(store, account);
        let mut lines = vec![HEADER.to_owned()];
        for instrument in &instruments {
            let ledger = book.ledgers.get(instrument).cloned().unwrap_or_default();
            let live = match (&ledger.open, readings.get(instrument)) {
                (Some(open), Some(reading)) => Some(close(open, reading, day, at_ic, ctx.w.now())),
                _ => None,
            };
            if ledger.open.is_none() {
                lines.push(format!(
                    "econ.census instrument={} has no reading yet: the next sweep takes one.",
                    instrument.as_str()
                ));
            }
            lines.extend(instrument_lines(
                instrument,
                &ledger,
                live.as_ref(),
                shown,
                &labels,
            ));
        }
        if instruments.is_empty() {
            lines.push("No currency instrument is registered.".to_owned());
        }
        lines
    };
    for line in lines {
        crate::commands::info(ctx, line);
    }
    Ok(())
}
