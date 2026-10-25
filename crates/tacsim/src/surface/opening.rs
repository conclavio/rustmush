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
use crate::surface::address::{POCKETS_PER_BLOCK, pocket_number};
use crate::surface::cursor::{CursorRow, MAX_CURSOR_ROWS};
use crate::surface::geology::{OreFamily, Word, by_family};
use crate::surface::ground::{SITE_BLOCKS, SiteGeology};

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

/// **A block's 256 site-local pockets** (§3.1): letter index `i` is block `(bx, by) = (i % 4,
/// 3 − i / 4)` from the south-west, as `SiteDef::shifts` reads it, so its pockets are `bx·16
/// ..bx·16+16 × by·16..by·16+16`. `None` for a letter outside `A`..`P`.
#[must_use]
pub fn block_pockets(letter: char) -> Option<impl Iterator<Item = (i64, i64)>> {
    let at = BLOCK_LETTERS.iter().position(|l| *l == letter)?;
    let at = i64::try_from(at).ok()?;
    let bx = at % SITE_BLOCKS;
    let by = SITE_BLOCKS - 1 - at / SITE_BLOCKS;
    let (x0, y0) = (bx * POCKETS_PER_BLOCK, by * POCKETS_PER_BLOCK);
    Some(
        (0..POCKETS_PER_BLOCK)
            .flat_map(move |dy| (0..POCKETS_PER_BLOCK).map(move |dx| (x0 + dx, y0 + dy))),
    )
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
            let pockets = block_pockets(letter)
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
                    let number = pocket_number(px, py)
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
}

/// **A site's opening state**; absent reads [`SiteOpening::Warming`] (a cold boot reads "still
/// being mapped" until the surface tick warms it and the port tick confirms it).
#[must_use]
pub fn opening_line(w: &World, entry: &CanonicalId) -> SiteOpening {
    w.module_state::<SiteOpenings>()
        .and_then(|slot| slot.states.get(entry).copied())
        .unwrap_or(SiteOpening::Warming)
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
