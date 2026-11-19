//! **A site's opening plan** (doc 20 cut SM.6; §3.1, §3.4, §7.3; D-20-5; the SM.6 brief's lead
//! rulings 1–3, 9 and 11, decisions D3–D7, D14 and D15).
//!
//! Before anyone can mine a site, its ore becomes conserved industry quantity: one
//! `OreDeposit` per (block, family) whose pockets hold that family's ore, its reserve the
//! **gross** sum of the block's pocket tonnes of that family, and — where the block authors a
//! `worked_permille` — that share of **every** pocket moved at once into the site's spoil (the
//! commons's heaps are real). The site clerk does it on the port tick, one act a fire
//! (`crate::port::site::site_step`); this module holds what it reads:
//!
//! - [`opening_plan`] — pure: every (block, family) with
//!   ore, block letters `A`..`P` then `OreFamily::ALL`, each row's receipt, deposit id, place,
//!   vault, gross and per-pocket worked share. **One quantiser** (lead ruling 1): the tonnes
//!   are SM.1's keyed site geology (`SiteGeology`, the table the site ground cache holds and
//!   `chart::field_ground_at` renders from); there is no second table, nothing is player
//!   input and nothing is computed by the applier;
//! - [`SiteOpenings`] — the RAM slot the Site readiness
//!   line reads ("opening, N of M"); never persisted, its own slot (not `PortTickState`, which
//!   a fire takes whole).
//!
//! **Derived truth, server-side by design** (reviewer F6): [`OpeningRow`] carries the gross and
//! the pockets' worked shares, so it derives no serde at all
//! (`truth_serialization_policy.rs` pins it); a deposit's existence at its public,
//! receipt-derived id says that block holds that family's ore (the barren-ness bit, D14 — SM.7
//! and SM.10 must answer a barren and an ore-bearing (block, family) alike). No outcome, offer
//! a player sees, or player text carries a deposit id, a reserve or a cursor row.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{CanonicalId, ObjId, World};
use mush_economy::VaultId;
use mush_industry::OreDepositId;

use crate::port::def::{BLOCK_LETTERS, SiteDef};
use crate::port::map::SiteRow;
use crate::surface::address::Scheme;
use crate::surface::cursor::{CursorRow, MAX_CURSOR_ROWS, PocketCursor};
use crate::surface::geology::{OreFamily, Word, by_family};
use crate::surface::ground::{SITE_BLOCKS, SiteGeology};

/// The shipped scheme's [`block_pockets_in`] (the unit tests read it).
#[cfg(test)]
fn block_pockets(letter: char) -> Option<impl Iterator<Item = (i64, i64)>> {
    block_pockets_in(Scheme::SHIPPED, letter)
}

/// `OreDeposit.external` for every surface deposit: the provider that works it.
pub const SURFACE_PROVIDER: &str = "tacsim.surface";
/// Both survey bands of a surface deposit: uninformative (lead ruling 11), so industry's
/// `@survey` copies nothing true.
pub const SURFACE_BAND: &str = "surface";
/// A surface deposit's grade: grade is spent at the drill as tonnes (D-20-10), not here.
pub const ORE_DEPOSIT_GRADE: u8 = 0;
/// A surface deposit's accessibility: always workable, so `extraction_authorized`'s body test
/// passes until the pool is empty (§3.4).
pub const ORE_DEPOSIT_ACCESSIBILITY: u8 = 100;
/// Most deposits one block place carries: one per family (§9.2), inside industry's
/// `MAX_ORE_DEPOSITS_PER_PLACE` 8.
pub const MAX_DEPOSITS_PER_BLOCK: usize = 4;
/// Most deposits one site opens: sixteen blocks of at most four.
pub const MAX_DEPOSITS_PER_SITE: usize = 64;

/// **The family's ore good** (§7.3; goods v3, `stow.ore`).
#[must_use]
pub fn ore_good(family: OreFamily) -> &'static str {
    match family {
        OreFamily::Metals => "metal_ore",
        OreFamily::Volatiles => "ice_ore",
        OreFamily::CarbonFeedstock => "carbon_ore",
        OreFamily::StrategicElements => "strategic_ore",
    }
}

/// **A (site, block, family)'s receipt** (D5): `"{entry}/{letter lower}/{family.id()}"` — the
/// replay guard and, through `mush_industry::derive_system_deposit_id`, the deposit's id.
#[must_use]
pub fn receipt(entry: &CanonicalId, letter: char, family: OreFamily) -> String {
    format!("{entry}/{}/{}", letter.to_ascii_lowercase(), family.id())
}

/// **A block's site-local pockets** (§3.1): letter index `i` is block `(bx, by) = (i % 4,
/// 3 − i / 4)` from the south-west, as `SiteDef::shifts` reads it, so its pockets are `bx·b
/// ..bx·b+b × by·b..by·b+b`, `b` the pockets along a block's side under the site's `scheme` (16 at
/// 256: 256 pockets; 32 at 512: 1,024 — doc 32 cut S.1). `None` for a letter outside `A`..`P`.
#[must_use]
pub fn block_pockets_in(scheme: Scheme, letter: char) -> Option<impl Iterator<Item = (i64, i64)>> {
    let at = BLOCK_LETTERS.iter().position(|l| *l == letter)?;
    let at = i64::try_from(at).ok()?;
    let bx = at % SITE_BLOCKS;
    let by = SITE_BLOCKS - 1 - at / SITE_BLOCKS;
    let b = scheme.block_pockets();
    let (x0, y0) = (bx * b, by * b);
    Some((0..b).flat_map(move |dy| (0..b).map(move |dx| (x0 + dx, y0 + dy))))
}

/// **One (block, family) of a site's opening** (derived truth; no serde, reviewer F6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpeningRow {
    /// [`receipt`].
    pub receipt: String,
    /// `mush_industry::derive_system_deposit_id(&receipt)`.
    pub deposit: OreDepositId,
    /// The block letter.
    pub letter: char,
    /// The family.
    pub family: OreFamily,
    /// [`ore_good`] as a commodity id.
    pub good: CanonicalId,
    /// The block place Thing.
    pub place: ObjId,
    /// The block vault — the deposit's endpoint.
    pub vault: VaultId,
    /// Σ the block's ore pockets' tonnes of the family: the deposit's initial reserve.
    pub gross_t: u64,
    /// Σ `cursor`'s `drawn_t`: what the worked transfer moves into the spoil.
    pub worked_t: u64,
    /// One row per ore pocket whose `⌊tonnes × worked_permille / 1000⌋ ≥ 1`, pocket order.
    pub cursor: Vec<CursorRow>,
}

/// **A site's opening plan** (D4, D6, D14): for each block letter `A`..`P` in order, then each
/// family in `OreFamily::ALL` order, a row when the block's ore pockets of that family hold
/// any tonnes (industry refuses a zero reserve, so a real site opens only its ore-bearing
/// rows). The worked share is per pocket, `⌊t × ‰ / 1000⌋` ("that share of every pocket",
/// §3.1), never depletion by order. At most four rows a letter and sixty-four a site, by
/// construction.
///
/// # Errors
///
/// A block place or block vault missing from `row` (named by its letter), a pocket the
/// geology does not hold, a gross past `MAX_ORE_RESERVE`, a receipt that derives no id, or a
/// cursor past [`MAX_CURSOR_ROWS`].
pub fn opening_plan(
    entry: &CanonicalId,
    site: &SiteDef,
    row: &SiteRow,
    geology: &SiteGeology,
) -> Result<Vec<OpeningRow>, String> {
    let mut plan = Vec::new();
    for letter in BLOCK_LETTERS {
        let key = letter.to_string();
        let place = row
            .blocks
            .get(&key)
            .copied()
            .ok_or_else(|| format!("{entry}: block {letter} has no place laid up"))?;
        let vault = row
            .vaults
            .get(&key)
            .copied()
            .ok_or_else(|| format!("{entry}: block {letter} has no vault laid up"))?;
        let permille = u64::from(site.block(letter).worked_permille.min(1_000));
        for family in OreFamily::ALL {
            let mut gross = 0_u64;
            let mut cursor: Vec<CursorRow> = Vec::new();
            let pockets = block_pockets_in(geology.scheme(), letter)
                .ok_or_else(|| format!("{entry}: block {letter} is off the site"))?;
            for (px, py) in pockets {
                let pocket = geology.pocket(px, py).ok_or_else(|| {
                    format!("{entry}: pocket ({px}, {py}) is off the site's geology")
                })?;
                if by_family(pocket.classes, family) == Word::Barren {
                    continue;
                }
                let tonnes = u64::from(by_family(pocket.tonnes, family));
                gross = gross
                    .checked_add(tonnes)
                    .ok_or_else(|| format!("{entry}: block {letter}'s reserve overflows"))?;
                let drawn = tonnes.saturating_mul(permille) / 1_000;
                if drawn >= 1 {
                    let number = geology
                        .scheme()
                        .pocket_number(px, py)
                        .ok_or_else(|| format!("{entry}: pocket ({px}, {py}) has no number"))?;
                    cursor.push(CursorRow {
                        pocket: number,
                        drawn_t: u32::try_from(drawn)
                            .map_err(|_| format!("{entry}: a pocket's share overflows"))?,
                        lock: None,
                    });
                }
            }
            if gross == 0 {
                continue;
            }
            if gross > mush_industry::MAX_ORE_RESERVE {
                return Err(format!(
                    "{entry}: block {letter}'s {} reserve is past industry's cap",
                    family.id()
                ));
            }
            if cursor.len() > MAX_CURSOR_ROWS {
                return Err(format!(
                    "{entry}: block {letter}'s {} cursor is past {MAX_CURSOR_ROWS} rows",
                    family.id()
                ));
            }
            cursor.sort_by_key(|row| row.pocket);
            let worked = cursor
                .iter()
                .try_fold(0_u64, |sum, row| sum.checked_add(u64::from(row.drawn_t)))
                .ok_or_else(|| format!("{entry}: block {letter}'s worked share overflows"))?;
            let receipt = receipt(entry, letter, family);
            let deposit = mush_industry::derive_system_deposit_id(&receipt)
                .map_err(|error| format!("{entry}: receipt {receipt}: {error}"))?;
            let good = CanonicalId::new(ore_good(family))
                .map_err(|_| format!("{entry}: {} is not a commodity id", ore_good(family)))?;
            plan.push(OpeningRow {
                receipt,
                deposit,
                letter,
                family,
                good,
                place: ObjId(place),
                vault: VaultId(vault),
                gross_t: gross,
                worked_t: worked,
                cursor,
            });
        }
    }
    Ok(plan)
}

/// **Where one site's opening stands** (D2; the Site readiness line).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SiteOpening {
    /// Its ground is cold: wanted, and the surface tick will warm it.
    Warming,
    /// Its ground could not be mapped (a build or a plan that failed this boot).
    Failed,
    /// `done` of `total` plan rows complete.
    Opening {
        /// Rows complete (deposit open and, when worked, the cursor written).
        done: u16,
        /// The plan's rows.
        total: u16,
    },
    /// Every row complete: `total` deposits open.
    Open {
        /// The plan's rows.
        total: u16,
    },
}

/// **The opening slot** (D2): RAM only, its own slot beside `GeologyState` — never
/// `PortTickState`, which a fire takes whole while the order and launch arms read
/// `port_ready` inside it. Bounded by the lane's sites (`states`) and their rows
/// (`attempts`, `paged`).
#[derive(Clone, Debug, Default)]
pub struct SiteOpenings {
    /// Each site's last recorded state.
    pub states: BTreeMap<CanonicalId, SiteOpening>,
    /// Unlanded submissions by receipt (open and work keyed apart: `open/…`, `work/…`).
    pub(crate) attempts: BTreeMap<String, u8>,
    /// What has paged this boot, by key.
    pub(crate) paged: BTreeSet<String>,
    /// **Doc 48 unit PL.10**: each site's or cluster's next re-roll look, IC seconds. Bounded
    /// by the lane's sites and clusters; a restart looks at once.
    pub(crate) reroll_scan: BTreeMap<CanonicalId, i64>,
}

/// **A site's opening state**; absent reads [`SiteOpening::Warming`] (a cold boot reads "still
/// being mapped" until the surface tick warms it and the port tick confirms it).
#[must_use]
pub fn opening_line(w: &World, entry: &CanonicalId) -> SiteOpening {
    w.module_state::<SiteOpenings>()
        .and_then(|slot| slot.states.get(entry).copied())
        .unwrap_or(SiteOpening::Warming)
}

// ---- doc 48 unit PL.10: the re-roll (Q-48-61) ------------------------------------------------
//
// A worked-out deposit — a surface site's (block, family) or a space cluster's (rock, family),
// its pool empty — re-rolls `[tacsim.surface].ore_reroll_ic_days` after its clerk records it
// worked out. Two acts of the clerk's `tacsim_site_work`, each in one bundle: the **mark**
// stamps the instant on the deposit's cursor ledger; the **re-roll** credits the pool with the
// deposit's opened reserve again and clears the rows. Both are durable, so a restart between
// them re-rolls on time. The opened reserve is the keyed geology's (a surface block) or the
// authored rock's (a cluster): nothing new is drawn, so the geology key, its fingerprint and
// every reading stay true, and the counter numbers the re-rolls.

/// **A worked-out deposit re-rolls after this many IC days, by default** (doc 48 Q-48-61,
/// provisional): 28 real hours at the live 24:1. `[tacsim.surface].ore_reroll_ic_days`; `0`
/// keeps the ore finite (the question's alternative).
pub const DEFAULT_ORE_REROLL_IC_DAYS: u16 = 28;
/// The longest re-roll delay a server may set, IC days.
pub const MAX_ORE_REROLL_IC_DAYS: u16 = 365;
/// How often, in IC seconds, a clerk looks over its deposits for one worked out, when none it
/// has marked falls due sooner: an IC hour (150 real seconds at 24:1).
pub const REROLL_SCAN_IC_S: i64 = 3_600;
/// A worked-out mark's word in a work row's receipt: `{deposit receipt}/out/{n}`.
pub const REROLL_MARK_WORD: &str = "out";
/// A re-roll's word in a work row's receipt: `{deposit receipt}/reroll/{n}`.
pub const REROLL_WORD: &str = "reroll";

/// **A worked-out deposit's next act** (doc 48 unit PL.10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RerollAct {
    /// Record the deposit worked out at the act's instant; re-roll `n` falls due a delay later.
    Mark {
        /// The re-roll this mark is for (`rerolls + 1`).
        n: u32,
    },
    /// Re-roll `n`, due at `due_ic`: the pool credited `reserve_t`, the rows cleared.
    Reroll {
        /// The re-roll's number (`rerolls + 1`).
        n: u32,
        /// When it fell due: the mark's instant and the delay.
        due_ic: i64,
        /// The deposit's opened reserve, credited back.
        reserve_t: u64,
    },
}

impl RerollAct {
    /// The re-roll's number.
    #[must_use]
    pub const fn n(self) -> u32 {
        match self {
            RerollAct::Mark { n } | RerollAct::Reroll { n, .. } => n,
        }
    }

    /// The act's word in its receipt.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            RerollAct::Mark { .. } => REROLL_MARK_WORD,
            RerollAct::Reroll { .. } => REROLL_WORD,
        }
    }
}

/// **One deposit's re-roll state, read**: an act due now, a wait for a marked one, or nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RerollNext {
    /// Nothing to do: ore in the pool, re-rolls off, or a cursor the audit would not pass.
    Idle,
    /// An act due now.
    Act(RerollAct),
    /// Marked worked out; its re-roll falls due at this IC instant.
    WaitUntil(i64),
}

/// **What a deposit's clerk does next** (pure; doc 48 unit PL.10): with re-rolls on
/// (`period_ic_s > 0`), an empty pool and a cursor whose audit total is the opened reserve
/// (`initial_reserve − pool == audit_drawn` at `pool == 0`) — a mark when the ledger holds none,
/// else the re-roll once its delay has run. A cursor that fails its check or the audit is never
/// touched (industry holds its block closed).
#[must_use]
pub fn reroll_next(
    cursor: Option<&PocketCursor>,
    pool_t: u64,
    reserve_t: u64,
    now_ic: i64,
    period_ic_s: i64,
) -> RerollNext {
    if period_ic_s <= 0 || pool_t != 0 || reserve_t == 0 {
        return RerollNext::Idle;
    }
    let Some(cursor) = cursor else {
        return RerollNext::Idle;
    };
    if cursor.check().is_err() || cursor.audit_drawn() != Some(reserve_t) {
        return RerollNext::Idle;
    }
    let ledger = cursor.reroll.clone().unwrap_or_default();
    let Some(n) = ledger.rerolls.checked_add(1) else {
        return RerollNext::Idle;
    };
    match ledger.worked_out_ic {
        None => RerollNext::Act(RerollAct::Mark { n }),
        Some(at) => {
            let due_ic = at.saturating_add(period_ic_s);
            if now_ic >= due_ic {
                RerollNext::Act(RerollAct::Reroll {
                    n,
                    due_ic,
                    reserve_t,
                })
            } else {
                RerollNext::WaitUntil(due_ic)
            }
        }
    }
}

/// **The cursor a mark writes**: the ledger's `worked_out_ic` set to `at_ic`. `None` when the
/// mark is not re-roll `n`'s or the deposit is already marked.
#[must_use]
pub fn marked_cursor(prior: &PocketCursor, n: u32, at_ic: i64) -> Option<PocketCursor> {
    let mut next = prior.clone();
    let mut ledger = next.reroll.take().unwrap_or_default();
    if ledger.rerolls.checked_add(1) != Some(n) || ledger.worked_out_ic.is_some() {
        return None;
    }
    ledger.worked_out_ic = Some(at_ic);
    next.reroll = Some(ledger);
    next.check().ok()?;
    Some(next)
}

/// **A cursor stamped worked out at `at_ic`** (doc 48 unit PL.10): the space draw that empties
/// its rock records the mark in its own bundle. A cursor already marked keeps its instant.
#[must_use]
pub fn stamped_worked_out(mut cursor: PocketCursor, at_ic: i64) -> PocketCursor {
    let mut ledger = cursor.reroll.take().unwrap_or_default();
    if ledger.worked_out_ic.is_none() {
        ledger.worked_out_ic = Some(at_ic);
    }
    cursor.reroll = Some(ledger);
    cursor
}

/// **The cursor a re-roll writes** (doc 48 unit PL.10): every row cleared — a locked row kept,
/// with nothing drawn, so a rig mid-run keeps its pocket — the rows' tonnes carried, `reserve_t`
/// recorded as credited, the counter at `n` and the mark spent. `None` unless the prior is
/// marked for re-roll `n` and its audit total is `reserve_t`; the result keeps
/// [`PocketCursor::drawn_total`] and leaves [`PocketCursor::audit_drawn`] at 0, which the
/// credited pool balances.
#[must_use]
pub fn rerolled_cursor(prior: &PocketCursor, n: u32, reserve_t: u64) -> Option<PocketCursor> {
    let drawn = prior.rows_drawn()?;
    let mut ledger = prior.reroll.clone().unwrap_or_default();
    if reserve_t == 0
        || ledger.rerolls.checked_add(1) != Some(n)
        || ledger.worked_out_ic.is_none()
        || prior.audit_drawn() != Some(reserve_t)
    {
        return None;
    }
    ledger.rerolls = n;
    ledger.carried_t = ledger.carried_t.checked_add(drawn)?;
    ledger.rerolled_t = ledger.rerolled_t.checked_add(reserve_t)?;
    ledger.worked_out_ic = None;
    let rows = prior
        .rows
        .iter()
        .filter(|row| row.lock.is_some())
        .map(|row| CursorRow {
            pocket: row.pocket,
            drawn_t: 0,
            lock: row.lock.clone(),
        })
        .collect();
    let next = PocketCursor {
        schema: prior.schema,
        deposit: prior.deposit,
        rows,
        reroll: Some(ledger),
    };
    next.check().ok()?;
    (next.drawn_total() == prior.drawn_total() && next.audit_drawn() == Some(0)).then_some(next)
}

/// **A re-roll act's work-row receipt**: `{deposit receipt}/{word}/{n}`.
#[must_use]
pub fn reroll_receipt(deposit_receipt: &str, act: RerollAct) -> String {
    format!("{deposit_receipt}/{}/{}", act.word(), act.n())
}

/// **The deposit receipt, word and number a re-roll's work-row receipt names**, or `None` for any
/// other receipt (an opening's worked share, a space draw's). The number is canonical: `1` and up,
/// no sign, no leading zero.
#[must_use]
pub fn parse_reroll_receipt(receipt: &str) -> Option<(&str, &'static str, u32)> {
    let (rest, number) = receipt.rsplit_once('/')?;
    let (base, word) = rest.rsplit_once('/')?;
    let word = [REROLL_MARK_WORD, REROLL_WORD]
        .into_iter()
        .find(|known| *known == word)?;
    let n: u32 = number.parse().ok()?;
    (n >= 1 && n.to_string() == number && !base.is_empty()).then_some((base, word, n))
}

/// **A site's deposit receipts**, in plan order (block letters `A`..`P`, then `OreFamily::ALL`):
/// the receipts every opened deposit of the site has, and some that never opened.
#[must_use]
pub fn site_receipts(entry: &CanonicalId) -> Vec<String> {
    BLOCK_LETTERS
        .into_iter()
        .flat_map(|letter| {
            OreFamily::ALL
                .into_iter()
                .map(move |family| receipt(entry, letter, family))
        })
        .collect()
}

/// **One deposit as its clerk's re-roll reads it** (doc 48 unit PL.10).
#[derive(Clone, Debug)]
pub(crate) struct RerollRead {
    /// The deposit.
    pub deposit: mush_industry::OreDeposit,
    /// Its pool, tonnes.
    pub pool_t: u64,
    /// Its cursor.
    pub cursor: Option<PocketCursor>,
}

/// **The first deposit, in `receipts` order, with a re-roll act due now** (its deposit
/// receipt, its read, its act), and the earliest instant a marked one falls due. `read`
/// answers `None` for a deposit not opened or not readable, which is passed over (fail closed).
pub(crate) fn first_due(
    receipts: &[String],
    now_ic: i64,
    period_ic_s: i64,
    mut read: impl FnMut(OreDepositId) -> Option<RerollRead>,
) -> (Option<(String, RerollRead, RerollAct)>, Option<i64>) {
    let mut wait: Option<i64> = None;
    if period_ic_s <= 0 {
        return (None, None);
    }
    for deposit_receipt in receipts {
        let Ok(id) = mush_industry::derive_system_deposit_id(deposit_receipt) else {
            continue;
        };
        let Some(found) = read(id) else {
            continue;
        };
        if found.deposit.external().map(smol_str::SmolStr::as_str) != Some(SURFACE_PROVIDER) {
            continue;
        }
        match reroll_next(
            found.cursor.as_ref(),
            found.pool_t,
            found.deposit.initial_reserve(),
            now_ic,
            period_ic_s,
        ) {
            RerollNext::Idle => {}
            RerollNext::WaitUntil(due) => {
                wait = Some(wait.map_or(due, |sooner| sooner.min(due)));
            }
            RerollNext::Act(act) => return (Some((deposit_receipt.clone(), found, act)), wait),
        }
    }
    (None, wait)
}

/// **The re-roll delay in IC seconds** the world's `[tacsim.surface]` sets (0: re-rolls off).
#[must_use]
pub fn reroll_period_ic_s(w: &World) -> i64 {
    w.module_state::<crate::store::TacsimStore>()
        .map_or(0, |store| {
            i64::from(store.config().surface.ore_reroll_ic_days).saturating_mul(86_400)
        })
}

/// **The re-roll's offered row** (`tacsim_site_work`'s fixed `rows` value): the deposit, its
/// good and vault, the clerk, the act and its number, the instant, and for a re-roll its due
/// instant and the tonnes it credits. Server-projected: the prepare re-checks every figure it
/// can against the cursor and the pool in its own transaction.
pub(crate) fn reroll_row_value(
    read: &RerollRead,
    act: RerollAct,
    clerk: ObjId,
    now_ic: i64,
) -> serde_json::Value {
    let mut value = serde_json::json!({
        "deposit": crate::surface::cursor::cursor_key_text(&read.deposit.id()),
        "commodity": read.deposit.commodity().as_str(),
        "from": read.deposit.endpoint().0.0,
        "clerk": clerk.0,
        "reroll_act": act.word(),
        "n": act.n(),
        "at_ic": now_ic,
    });
    if let (
        RerollAct::Reroll {
            due_ic, reserve_t, ..
        },
        Some(map),
    ) = (act, value.as_object_mut())
    {
        map.insert("due_ic".to_owned(), serde_json::json!(due_ic));
        map.insert("reserve_t".to_owned(), serde_json::json!(reserve_t));
    }
    value
}

/// **The re-roll row a clerk's `tacsim_site_work` offer carries** (doc 48 unit PL.10): the
/// first deposit of the clerk's site (a lane site's clerk) or cluster (a cluster clerk), in
/// order, with an act due now — the row [`reroll_step`] submits, since it walks the same
/// receipts with the same rule. `None` when nothing is due.
pub(crate) fn next_reroll_row(
    engine: &mush_core::Engine,
    pockets: &mush_core::StateFamilyHandle<OreDepositId, PocketCursor>,
    clerk: ObjId,
) -> Option<(String, serde_json::Value)> {
    let w = &engine.world;
    let receipts = match crate::port::site::clerk_site(w, clerk) {
        Some((map, _)) => site_receipts(&map.entry),
        None => crate::space::mining::cluster_receipts_of_clerk(w, clerk)?,
    };
    let industry = w
        .module_state::<mush_industry::IndustryHandles>()?
        .0
        .clone();
    let inventory = w
        .module_state::<crate::port::families::PortHandles>()?
        .inventory
        .clone()?;
    let state = engine.action_state();
    let now_ic = w.world_ic_now().at;
    let (due, _) = first_due(&receipts, now_ic, reroll_period_ic_s(w), |id| {
        let deposit = state.get(&industry.deposits, &id).ok()??.clone();
        let key = mush_inventory::pool_key(deposit.endpoint(), deposit.commodity()).ok()?;
        let pool_t = match state.get(&inventory.pools, &key).ok()? {
            Some(scalar) => u64::try_from(scalar.current()).ok()?,
            None => 0,
        };
        let cursor = state.get(pockets, &id).ok()?.cloned();
        Some(RerollRead {
            deposit,
            pool_t,
            cursor,
        })
    });
    let (deposit_receipt, read, act) = due?;
    Some((
        reroll_receipt(&deposit_receipt, act),
        reroll_row_value(&read, act, clerk, now_ic),
    ))
}

/// **A clerk's re-roll step** (doc 48 unit PL.10): over `receipts` (its site's or cluster's
/// deposits), the first act due — a mark or a re-roll — submitted as `tacsim_site_work` by the
/// clerk itself; else nothing, and the next look waits until the earliest marked deposit
/// falls due or [`REROLL_SCAN_IC_S`] passes, whichever is sooner (RAM, `SiteOpenings`; a
/// restart looks at once). Re-rolls off (`ore_reroll_ic_days = 0`) idles without reading.
pub(crate) fn reroll_step(
    ctx: &mut mush_core::Ctx<'_>,
    fire: &mut crate::port_tick::Fire,
    clerk: ObjId,
    scope: &CanonicalId,
    receipts: &[String],
) -> crate::port_tick::Step {
    use crate::port_tick::Step;
    let period = reroll_period_ic_s(ctx.w);
    if period <= 0 {
        return Step::Idle;
    }
    let now_ic = ctx.w.world_ic_now().at;
    if ctx
        .w
        .module_state::<SiteOpenings>()
        .and_then(|slot| slot.reroll_scan.get(scope))
        .is_some_and(|next| *next > now_ic)
    {
        return Step::Idle;
    }
    let Some(industry) = ctx
        .w
        .module_state::<mush_industry::IndustryHandles>()
        .map(|handles| handles.0.clone())
    else {
        return Step::Refused;
    };
    let Some((pockets, action)) = ctx
        .w
        .module_state::<crate::port::families::PortHandles>()
        .map(|handles| (handles.families.pockets.clone(), handles.site_work.clone()))
    else {
        return Step::Refused;
    };
    let (due, wait) = first_due(receipts, now_ic, period, |id| {
        let deposit = ctx.action_state(&industry.deposits, &id).ok()??.clone();
        let pool_t =
            crate::port_tick::read_pool(ctx, deposit.endpoint(), deposit.commodity()).ok()?;
        let cursor = ctx.action_state(&pockets, &id).ok()?.cloned();
        Some(RerollRead {
            deposit,
            pool_t,
            cursor,
        })
    });
    let Some((deposit_receipt, _, act)) = due else {
        let next = wait.map_or(now_ic.saturating_add(REROLL_SCAN_IC_S), |due| {
            due.min(now_ic.saturating_add(REROLL_SCAN_IC_S))
        });
        if let Some(slot) = ctx.w.module_state_mut::<SiteOpenings>() {
            slot.reroll_scan.insert(scope.clone(), next);
        }
        return Step::Idle;
    };
    let Some(action) = action else {
        return Step::Refused;
    };
    let work_receipt = reroll_receipt(&deposit_receipt, act);
    let Ok(key) = mush_core::IdempotencyKeyText::parse_canonical(format!(
        "{}{work_receipt}",
        crate::port::site::SITE_WORK_KEY_PREFIX
    )) else {
        return Step::Refused;
    };
    crate::port::site::swapped(fire.as_principal(ctx, clerk, clerk, |ctx| {
        ctx.submit_authoritative(
            action,
            mush_core::PerceivedRef::SelfActor,
            Vec::new(),
            serde_json::json!({ "receipt": work_receipt }),
            key,
        );
    }))
}

/// **`tacsim_site_work`'s re-roll rows, prepared** (doc 48 unit PL.10): a mark or a re-roll of
/// one deposit, in one bundle. Each figure the offer fixed is re-checked here against the
/// deposit's own cursor and pool, read in this transaction: the receipt names the deposit and
/// its number is the ledger's next; the pool is empty. A mark stamps its instant; a re-roll that
/// has fallen due credits the pool the cursor's audit total (which, at an empty pool, is the
/// opened reserve) and writes [`rerolled_cursor`]. Any disagreement refuses, staging nothing.
///
/// # Errors
///
/// An `ActionError` naming the first check that failed.
pub(crate) fn prepare_reroll(
    pockets: &mush_core::StateFamilyHandle<OreDepositId, PocketCursor>,
    inventory: &mush_inventory::InventoryFamilies,
    outcome: &mush_core::DefinitionRef,
    invocation: &mush_core::ActionInvocation,
    txn: &mut mush_core::ActionTxn<'_>,
    receipt: &str,
    entry: &serde_json::Map<String, serde_json::Value>,
) -> Result<mush_core::AuthoritativeActionOutput, mush_core::ActionError> {
    use mush_core::ActionErrorCode;
    use serde_json::Value;
    let error = crate::port_tick::port_error;
    let invalid = || {
        error(
            ActionErrorCode::InvalidRequest,
            "the re-roll row does not read",
        )
    };
    let number = |key: &str| entry.get(key).and_then(Value::as_u64).ok_or_else(invalid);
    let signed = |key: &str| entry.get(key).and_then(Value::as_i64).ok_or_else(invalid);
    let (deposit_receipt, word, n) = parse_reroll_receipt(receipt).ok_or_else(invalid)?;
    if entry.get("reroll_act").and_then(Value::as_str) != Some(word) || number("n")? != u64::from(n)
    {
        return Err(invalid());
    }
    let actor = invocation.actor();
    if u32::try_from(number("clerk")?).ok() != Some(actor.0) {
        return Err(error(
            ActionErrorCode::Unauthorized,
            "that site row is another clerk's",
        ));
    }
    let deposit = entry
        .get("deposit")
        .and_then(Value::as_str)
        .and_then(crate::surface::cursor::cursor_key_parse)
        .ok_or_else(invalid)?;
    if mush_industry::derive_system_deposit_id(deposit_receipt).ok() != Some(deposit) {
        return Err(error(
            ActionErrorCode::Integrity,
            "the site row does not name its own deposit",
        ));
    }
    let commodity = entry
        .get("commodity")
        .and_then(Value::as_str)
        .and_then(|good| CanonicalId::new(good).ok())
        .ok_or_else(invalid)?;
    let from = mush_inventory::EndpointRef(VaultId(number("from")?));
    let at_ic = signed("at_ic")?;
    let prior = txn
        .get(pockets, &deposit)
        .map_err(|_| {
            error(
                ActionErrorCode::Integrity,
                "the pocket cursors are unavailable",
            )
        })?
        .cloned()
        .ok_or_else(|| error(ActionErrorCode::Conflict, "that deposit was never worked"))?;
    let pool = inventory
        .pool(txn, from, &commodity)
        .map_err(|_| {
            error(
                ActionErrorCode::Integrity,
                "the deposit's pool does not read",
            )
        })?
        .map_or(0, |pool| pool.quantity);
    if pool != 0 {
        return Err(error(
            ActionErrorCode::Conflict,
            "that deposit is not worked out",
        ));
    }
    let next = if word == REROLL_MARK_WORD {
        marked_cursor(&prior, n, at_ic)
            .ok_or_else(|| error(ActionErrorCode::Conflict, "that deposit is already marked"))?
    } else {
        let due_ic = signed("due_ic")?;
        let reserve_t = number("reserve_t")?;
        let marked = prior
            .reroll
            .as_ref()
            .and_then(|ledger| ledger.worked_out_ic);
        if !marked.is_some_and(|at| due_ic > at) || at_ic < due_ic {
            return Err(error(
                ActionErrorCode::Conflict,
                "that deposit's re-roll is not due",
            ));
        }
        let next = rerolled_cursor(&prior, n, reserve_t).ok_or_else(|| {
            error(
                ActionErrorCode::Conflict,
                "that deposit's re-roll does not match its cursor",
            )
        })?;
        inventory
            .credit(txn, from, &commodity, reserve_t)
            .map_err(|_| {
                error(
                    ActionErrorCode::ProviderRejected,
                    "the deposit's re-roll could not be credited",
                )
            })?;
        next
    };
    txn.replace(pockets, &deposit, next).map_err(|_| {
        error(
            ActionErrorCode::Integrity,
            "the pocket cursor did not stage",
        )
    })?;
    let outcome = mush_core::RulesValue::new(
        outcome.clone(),
        serde_json::json!({
            "actor": actor.0,
            "deposit": crate::surface::cursor::cursor_key_text(&deposit),
            "receipt": receipt,
        }),
    )
    .map_err(|_| {
        error(
            ActionErrorCode::BoundsExceeded,
            "the site work's outcome exceeds canonical bounds",
        )
    })?;
    Ok(mush_core::AuthoritativeActionOutput::new(outcome)
        .with_eligible_witnesses(vec![invocation.principal().clone()]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ore_good` names the four goods-v3 rows under `stow.ore`, each of its own family.
    #[test]
    fn sm6_ore_good_is_the_goods_v3_ore_row() {
        let table = crate::definitions::goods::goods_table()
            .unwrap_or_else(|error| panic!("the goods table loads: {error}"));
        for family in OreFamily::ALL {
            let id = CanonicalId::new(ore_good(family)).unwrap_or_else(|_| panic!("id"));
            let good = table
                .good(&id)
                .unwrap_or_else(|| panic!("{} is a goods row", ore_good(family)));
            assert_eq!(good.stowage.as_str(), "stow.ore");
            assert_eq!(
                good.family.as_ref().map(CanonicalId::as_str),
                Some(family.id())
            );
        }
    }

    /// Every receipt is one industry accepts, and the longest lane one fits.
    #[test]
    fn sm6_the_receipt_is_an_industry_receipt() {
        let entry = CanonicalId::new("occator_port").unwrap_or_else(|_| panic!("id"));
        for letter in BLOCK_LETTERS {
            for family in OreFamily::ALL {
                let text = receipt(&entry, letter, family);
                assert!(mush_industry::is_receipt(&text), "{text}");
            }
        }
        assert_eq!(
            receipt(&entry, 'C', OreFamily::Metals),
            "occator_port/c/metals"
        );
    }

    /// 256 distinct pockets a letter, 4,096 over A–P, and each in its own letter.
    #[test]
    fn sm6_block_pockets_cover_the_site_once() {
        let mut all = BTreeSet::new();
        for letter in BLOCK_LETTERS {
            let pockets: BTreeSet<(i64, i64)> = block_pockets(letter)
                .unwrap_or_else(|| panic!("{letter}"))
                .collect();
            assert_eq!(pockets.len(), 256, "{letter}");
            for (px, py) in &pockets {
                assert_eq!(
                    crate::surface::address::block_letter(*px, *py),
                    Some(letter)
                );
            }
            all.extend(pockets);
        }
        assert_eq!(all.len(), 4_096);
        assert!(block_pockets('Q').is_none());
    }

    fn cid(id: &str) -> CanonicalId {
        CanonicalId::new(id).unwrap_or_else(|_| panic!("{id}"))
    }

    /// A site whose blocks take the default terms but for `worked` (letter, ‰).
    fn site(worked: &[(&str, u16)]) -> SiteDef {
        SiteDef {
            name: "Here".to_owned(),
            issuer: cid("freeport_registry"),
            zone: crate::port::def::Zone::Core,
            tenure: crate::port::def::SiteTenureDef {
                default: crate::port::def::Tenure::Open,
            },
            blocks: worked
                .iter()
                .map(|(letter, permille)| {
                    (
                        (*letter).to_owned(),
                        crate::port::def::BlockDef {
                            worked_permille: *permille,
                            ..crate::port::def::BlockDef::default()
                        },
                    )
                })
                .collect(),
            papers: std::collections::BTreeMap::new(),
            yard: None,
        }
    }

    /// A fully laid-up row: block `L`'s place is `100 + i`, its vault `500 + i`.
    fn laid() -> SiteRow {
        let mut row = SiteRow::default();
        for (i, letter) in BLOCK_LETTERS.into_iter().enumerate() {
            let i = u32::try_from(i).unwrap_or_else(|_| panic!("small"));
            row.blocks.insert(letter.to_string(), 100 + i);
            row.vaults.insert(letter.to_string(), 500 + u64::from(i));
        }
        row
    }

    /// The block's own pocket number, written out from §3.1's address rule (`n = (15 − py %
    /// 16)·16 + px % 16 + 1`) rather than read back from `pocket_number`.
    fn number(px: i64, py: i64) -> u16 {
        u16::try_from((15 - py % 16) * 16 + px % 16 + 1).unwrap_or_else(|_| panic!("a number"))
    }

    /// One plan row as the floor test compares it: letter, family, gross, worked, cursor.
    type Summary = (char, OreFamily, u64, u64, Vec<(u16, u32)>);

    /// **The worked share floors per pocket, through `opening_plan` itself** (D6; review T2):
    /// on a synthetic table, block A at 400 ‰ holds a 3 t, a 24 t and a 2 t metals pocket (and a
    /// barren slot with tonnes, never counted) and a 24 t volatiles pocket sharing the 2 t
    /// pocket; block B at 0 ‰ holds one 24 t metals pocket; every other (block, family) is
    /// barren. The plan is exactly three rows in letter-then-family order; each gross is Σ the
    /// ore pockets' tonnes; each cursor is one row per pocket whose `⌊t × ‰ / 1000⌋ ≥ 1` (3 t →
    /// 1, 24 t → 9, 2 t → none) in pocket order; 0 ‰ gives no cursor rows and 0 worked; each
    /// row's receipt, deposit id, place, vault and good are the site's.
    #[test]
    fn sm6_the_worked_share_floors_per_pocket() {
        use OreFamily::{Metals, Volatiles};
        // Block A is (bx, by) = (0, 3): pockets x 0..16, y 48..64. Block B is (1, 3).
        let geology = SiteGeology::for_test(&[
            ((2, 63), Metals, Word::Fair, 3),
            ((5, 48), Metals, Word::Rich, 24),
            ((0, 60), Metals, Word::Lean, 2),
            ((0, 60), Volatiles, Word::Rich, 24),
            ((3, 63), Metals, Word::Barren, 5),
            ((20, 50), Metals, Word::Rich, 24),
        ]);
        let entry = cid("ahuna_port");
        let plan = opening_plan(&entry, &site(&[("A", 400), ("B", 0)]), &laid(), &geology)
            .unwrap_or_else(|error| panic!("the plan: {error}"));
        let summary: Vec<Summary> = plan
            .iter()
            .map(|row| {
                (
                    row.letter,
                    row.family,
                    row.gross_t,
                    row.worked_t,
                    row.cursor.iter().map(|c| (c.pocket, c.drawn_t)).collect(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (
                    'A',
                    Metals,
                    3 + 24 + 2,
                    1 + 9,
                    vec![(number(2, 63), 1), (number(5, 48), 9)]
                ),
                ('A', Volatiles, 24, 9, vec![(number(0, 60), 9)]),
                ('B', Metals, 24, 0, Vec::new()),
            ]
        );
        assert_eq!((number(2, 63), number(5, 48), number(0, 60)), (3, 246, 49));
        assert!(
            plan.iter()
                .flat_map(|row| &row.cursor)
                .all(|c| c.lock.is_none())
        );
        for row in &plan {
            let at = u32::try_from(
                BLOCK_LETTERS
                    .iter()
                    .position(|l| *l == row.letter)
                    .unwrap_or(99),
            )
            .unwrap_or_else(|_| panic!("small"));
            assert_eq!(row.receipt, receipt(&entry, row.letter, row.family));
            assert_eq!(
                Ok(row.deposit),
                mush_industry::derive_system_deposit_id(&row.receipt)
            );
            assert_eq!(row.place, ObjId(100 + at));
            assert_eq!(row.vault, VaultId(500 + u64::from(at)));
            assert_eq!(row.good.as_str(), ore_good(row.family));
        }
    }

    /// **`opening_plan`'s refusals** (review T8): a block with no place or no vault laid up
    /// refuses naming its letter, and a cursor past [`MAX_CURSOR_ROWS`] refuses naming the
    /// block and family — defensive: the shipped geology holds at most 64 ore pockets a block a
    /// family by construction (§3.2, §11; pinned in `tests/geology.rs`), so it never binds on a
    /// real site.
    #[test]
    fn sm6_the_plan_refuses_a_missing_row_and_an_overfull_cursor() {
        let entry = cid("kerwan_port");
        let geology = SiteGeology::for_test(&[((20, 50), OreFamily::Metals, Word::Rich, 24)]);
        let mut row = laid();
        row.vaults.remove("C");
        assert_eq!(
            opening_plan(&entry, &site(&[]), &row, &geology),
            Err("kerwan_port: block C has no vault laid up".to_owned())
        );
        let mut row = laid();
        row.blocks.remove("A");
        assert_eq!(
            opening_plan(&entry, &site(&[]), &row, &geology),
            Err("kerwan_port: block A has no place laid up".to_owned())
        );
        // 65 rich metals pockets in block B at 1000 ‰: 65 cursor rows.
        let dense: Vec<((i64, i64), OreFamily, Word, u8)> = (0..65_i64)
            .map(|n| {
                (
                    (16 + n % 16, 48 + n / 16),
                    OreFamily::Metals,
                    Word::Rich,
                    24,
                )
            })
            .collect();
        let geology = SiteGeology::for_test(&dense);
        assert_eq!(
            opening_plan(&entry, &site(&[("B", 1_000)]), &laid(), &geology),
            Err(format!(
                "kerwan_port: block B's metals cursor is past {MAX_CURSOR_ROWS} rows"
            ))
        );
        // Exactly 64 fits, and the whole share moves.
        let geology = SiteGeology::for_test(&dense[..64]);
        let plan = opening_plan(&entry, &site(&[("B", 1_000)]), &laid(), &geology)
            .unwrap_or_else(|error| panic!("64 rows fit: {error}"));
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].cursor.len(), MAX_CURSOR_ROWS);
        assert_eq!(plan[0].worked_t, 64 * 24);
        assert_eq!(plan[0].gross_t, 64 * 24);
    }

    // ---- doc 48 unit PL.10: the re-roll ----

    fn drawn(pocket: u16, drawn_t: u32) -> CursorRow {
        CursorRow {
            pocket,
            drawn_t,
            lock: None,
        }
    }

    fn lock(tv: u32) -> crate::surface::cursor::PocketLock {
        crate::surface::cursor::PocketLock {
            tv: ObjId(tv),
            ordinal: 1,
        }
    }

    /// A worked-out cursor of a 30 t deposit: 24 t and 6 t drawn, and a locked row drawn to
    /// nothing more (a rig whose last core came up empty).
    fn worked_out() -> PocketCursor {
        PocketCursor {
            schema: crate::surface::cursor::POCKET_CURSOR_SCHEMA,
            deposit: OreDepositId(41),
            rows: vec![
                drawn(3, 24),
                drawn(9, 6),
                CursorRow {
                    pocket: 12,
                    drawn_t: 0,
                    lock: Some(lock(7)),
                },
            ],
            reroll: None,
        }
    }

    const DAY: i64 = 86_400;
    const PERIOD: i64 = 28 * DAY;

    /// **The rule** (pure): off, ore in the pool, no cursor, or an audit total that is not the
    /// reserve is idle; an unmarked worked-out deposit is marked; a marked one waits out its
    /// delay to the second and is then re-rolled with its reserve.
    #[test]
    fn pl10_a_worked_out_deposit_is_marked_then_rerolled_when_due() {
        let cursor = worked_out();
        let at = 1_000_000;
        assert_eq!(reroll_next(Some(&cursor), 0, 30, at, 0), RerollNext::Idle);
        assert_eq!(
            reroll_next(Some(&cursor), 1, 30, at, PERIOD),
            RerollNext::Idle
        );
        assert_eq!(reroll_next(None, 0, 30, at, PERIOD), RerollNext::Idle);
        assert_eq!(
            reroll_next(Some(&cursor), 0, 31, at, PERIOD),
            RerollNext::Idle
        );
        assert_eq!(
            reroll_next(Some(&cursor), 0, 30, at, PERIOD),
            RerollNext::Act(RerollAct::Mark { n: 1 })
        );
        let marked = marked_cursor(&cursor, 1, at).expect("the mark");
        assert_eq!(
            marked.reroll.as_ref().and_then(|l| l.worked_out_ic),
            Some(at)
        );
        assert_eq!(marked_cursor(&marked, 1, at), None, "marked once");
        // Doc 48e, production G33: 0 means never. A deposit stamped worked out (a site's dig
        // stamps it whatever the period) re-rolls nothing while re-rolls are off, however long.
        assert_eq!(
            reroll_next(Some(&marked), 0, 30, at + 365 * DAY, 0),
            RerollNext::Idle
        );
        assert_eq!(marked_cursor(&cursor, 2, at), None, "the next number only");
        let due = at + PERIOD;
        assert_eq!(
            reroll_next(Some(&marked), 0, 30, due - 1, PERIOD),
            RerollNext::WaitUntil(due)
        );
        assert_eq!(
            reroll_next(Some(&marked), 0, 30, due, PERIOD),
            RerollNext::Act(RerollAct::Reroll {
                n: 1,
                due_ic: due,
                reserve_t: 30
            })
        );
    }

    /// **The re-rolled cursor**: the rows cleared but the locked one (kept, nothing drawn), the
    /// rows' tonnes carried and the reserve credited, the mark spent; the total never falls and
    /// the audit total is 0. A second cycle — drawn out again, marked, re-rolled — counts 2 and
    /// carries both. A re-roll of an unmarked cursor, of the wrong number or of another reserve
    /// is refused.
    #[test]
    fn pl10_a_reroll_clears_the_rows_and_keeps_the_totals() {
        let cursor = worked_out();
        assert_eq!(rerolled_cursor(&cursor, 1, 30), None, "unmarked");
        let marked = marked_cursor(&cursor, 1, 5).expect("the mark");
        assert_eq!(rerolled_cursor(&marked, 2, 30), None, "the wrong number");
        assert_eq!(rerolled_cursor(&marked, 1, 29), None, "another reserve");
        let once = rerolled_cursor(&marked, 1, 30).expect("the re-roll");
        assert_eq!(
            once.rows,
            vec![CursorRow {
                pocket: 12,
                drawn_t: 0,
                lock: Some(lock(7)),
            }]
        );
        assert_eq!(
            once.reroll,
            Some(crate::surface::cursor::RerollLedger {
                rerolls: 1,
                carried_t: 30,
                rerolled_t: 30,
                worked_out_ic: None,
            })
        );
        assert_eq!(once.drawn_total(), Some(30));
        assert_eq!(once.audit_drawn(), Some(0));
        assert_eq!(once.rows_drawn(), Some(0));
        // Drawn out again (the lock released), marked and re-rolled.
        let mut again = once.clone();
        again.rows = vec![drawn(3, 24), drawn(9, 6)];
        assert_eq!(again.audit_drawn(), Some(30));
        assert_eq!(
            reroll_next(Some(&again), 0, 30, 9, PERIOD),
            RerollNext::Act(RerollAct::Mark { n: 2 })
        );
        let twice = rerolled_cursor(&marked_cursor(&again, 2, 9).expect("mark 2"), 2, 30)
            .expect("re-roll 2");
        assert!(twice.rows.is_empty());
        assert_eq!(twice.drawn_total(), Some(60));
        assert_eq!(twice.audit_drawn(), Some(0));
        assert_eq!(twice.reroll.as_ref().map(|l| l.rerolls), Some(2));
        assert!(!twice.is_empty(), "a ledger with no row is kept");
        twice.check().expect("the re-rolled cursor checks");
    }

    /// **A rig's cursor edits keep the ledger**: after a re-roll, a lock taken and released with
    /// nothing drawn leaves the cursor (rows empty, ledger kept) rather than deleting it, and a
    /// draw adds to the rows only.
    #[test]
    fn pl10_the_rig_cursor_edits_keep_a_rerolled_ledger() {
        use crate::surface::mine::{cursor_after, cursor_locked, cursor_released};
        let marked = marked_cursor(&worked_out(), 1, 5).expect("the mark");
        let mut once = rerolled_cursor(&marked, 1, 30).expect("the re-roll");
        once.rows.clear();
        let ours = lock(8);
        let locked = cursor_locked(Some(&once), once.deposit, 4, ours.clone()).expect("lock");
        let released = cursor_released(&locked, 4, &ours).expect("kept, not deleted");
        assert!(released.rows.is_empty());
        assert_eq!(released.reroll, once.reroll);
        let closed = cursor_after(Some(&locked), once.deposit, 4, 0, &ours, false)
            .expect("in bounds")
            .expect("kept, not deleted");
        assert_eq!(closed.reroll, once.reroll);
        let dug = cursor_after(Some(&locked), once.deposit, 4, 3, &ours, false)
            .expect("in bounds")
            .expect("drawn");
        assert_eq!(dug.rows_drawn(), Some(3));
        assert_eq!(dug.drawn_total(), Some(33));
        assert_eq!(dug.audit_drawn(), Some(3));
    }

    /// **The receipts**: a re-roll's receipt round-trips; an opening's worked share, a space
    /// draw, a bad word, `0` or a padded number is no re-roll's.
    #[test]
    fn pl10_the_reroll_receipts_round_trip_and_refuse_others() {
        let base = receipt(&cid("ahuna_port"), 'C', OreFamily::Metals);
        let mark = reroll_receipt(&base, RerollAct::Mark { n: 1 });
        assert_eq!(mark, "ahuna_port/c/metals/out/1");
        assert_eq!(
            parse_reroll_receipt(&mark),
            Some((base.as_str(), REROLL_MARK_WORD, 1))
        );
        let again = reroll_receipt(
            &base,
            RerollAct::Reroll {
                n: 12,
                due_ic: 0,
                reserve_t: 1,
            },
        );
        assert_eq!(
            parse_reroll_receipt(&again),
            Some((base.as_str(), REROLL_WORD, 12))
        );
        for other in [
            base.as_str(),
            "pallas_inclined_belt/a/metals/draw/3",
            "ahuna_port/c/metals/out/0",
            "ahuna_port/c/metals/out/01",
            "ahuna_port/c/metals/out/+1",
            "ahuna_port/c/metals/roll/1",
            "/out/1",
        ] {
            assert_eq!(parse_reroll_receipt(other), None, "{other}");
        }
        assert!(mush_industry::is_receipt(&again));
        let receipts = site_receipts(&cid("ahuna_port"));
        assert_eq!(receipts.len(), MAX_DEPOSITS_PER_SITE);
        assert_eq!(receipts[0], "ahuna_port/a/metals");
        assert_eq!(receipts[63], "ahuna_port/p/strategic_elements");
    }

    /// The caps: a letter carries at most one row a family, a site at most 64.
    #[test]
    fn sm6_the_plan_caps_hold_by_construction() {
        assert_eq!(MAX_DEPOSITS_PER_BLOCK, OreFamily::ALL.len());
        assert_eq!(
            MAX_DEPOSITS_PER_SITE,
            BLOCK_LETTERS.len() * OreFamily::ALL.len()
        );
        const { assert!(MAX_DEPOSITS_PER_BLOCK <= mush_industry::MAX_ORE_DEPOSITS_PER_PLACE) };
    }
}
