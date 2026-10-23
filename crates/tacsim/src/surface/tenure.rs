//! **The tenure numbers** (doc 20 §9.2–§9.6 and §11's Tenure and Claims rows; cut SM.6, the
//! brief's lead ruling 6 and decision D9).
//!
//! Industry's tenure seams (`mush_industry::IndustryFamilies::renew_claim`, `lapse_claim`,
//! `take_over_claim`, `transfer_tenure`) are neutral and take every duration as an argument;
//! the numbers are this game's, and live here for `+claim/*` (`crate::port::claims`) to read.
//! **No fee constant**: the claim fees are lane data beside the licence's (cut SM.10, lead
//! ruling 3: `crate::port::def::LicenceDef::claim_stake_major` and `claim_term_major`).
//!
//! **Doc 20 cut SM.10** adds the pure rules the stake, the renewal and the contest share:
//! [`evidence_at`] (the hand-made evidence rule), [`heard_before`] and [`heard_count`] (the
//! loud soundings the site heard), [`worked_this_term`] and [`renewal_floor`] (the worked
//! floor), and [`contest_open`] (the contest window). No RNG, no world.

use mush_core::ObjId;

use crate::surface::chart::{ChartProvenance, ChartRecord, Instrument, Via};

/// A claim's term, IC seconds: the dormancy week (§9.2 "term 168 IC days"; §11's Claims row —
/// seven real days at rate 24, the charter's dormancy week, `lane:2171`).
pub const CLAIM_TERM_IC_SECS: i64 = 168 * 86_400;

/// **Use it or lose it** (§9.2; §11's Claims row): a renewal needs this many tonnes worked in
/// the term, so nobody banks ground for a cycle a week.
pub const WORKED_FLOOR_T: u64 = 50;

/// **The cooling term**, IC seconds (§9.2; §11's Tenure row): a lapsed holder, and anyone
/// acting for the same principal, may not stake the block again for one full term from the
/// lapse.
pub const COOLING_TERM_IC_SECS: i64 = 168 * 86_400;

/// **The contest window**, IC seconds (§9.4; §11's Claims row): a rival may contest within 30
/// IC days of a registration.
pub const CONTEST_WINDOW_IC_SECS: i64 = 30 * 86_400;

/// **Live claims per account** (§9.2; §11's Claims row): at most two.
pub const MAX_LIVE_CLAIMS_PER_ACCOUNT: usize = 2;

/// **The Registrar's rule** (§9.6): a papered block worked for at least this many tonnes in the
/// term has the Syndicate paper suspended at its holder's first renewal.
pub const REGISTRAR_WORKED_T: u64 = 100;

// ---- doc 20 cut SM.10: the claim numbers beside SM.6's ----

/// **The seeded claims' term**, IC seconds (doc 20 cut SM.10, OC3): a century — Occator's held
/// blocks and the Syndicate's paper never come due in phase 1, and the paper's adjudication
/// contract's deadline is the same century, so it never expires into a stale `Contested`.
pub const SEEDED_TERM_IC_SECS: i64 = 36_500 * 86_400;

/// **A claim fee contract's real-time window**, seconds (the licence's
/// `LICENCE_SETTLE_WINDOW_SECS`): an `Offered` fee the Registry has not accepted within it
/// refunds at its deadline (a refused act costs nothing, OC14).
pub const CLAIM_FEE_WINDOW_SECS: i64 = 3_600;

/// **The heard count's bound** (doc 20 cut SM.10, D12, OC12): the most `tacsim.chart` records
/// one preview or `go` reads to count the soundings heard on a block; past it the preview says
/// "at least".
pub const MAX_HEARD_SCAN_ROWS: usize = 512;

/// **A claim order's life**, IC seconds (D5): an order older than this offers nothing.
pub const CLAIM_ORDER_TTL_IC_SECS: i64 = 3_600;

/// The block's four fields' indexes on the site (`fy · 8 + fx`).
fn block_field_indexes(letter: char) -> Vec<u8> {
    let Some((bx, by)) = crate::surface::address::block_of_letter(letter) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(4);
    for dy in 0..2 {
        for dx in 0..2 {
            if let Some(index) = crate::surface::address::field_index(bx * 2 + dx, by * 2 + dy) {
                out.push(index);
            }
        }
    }
    out
}

/// Whether a provenance entry is hand-made by one of the four reading instruments.
fn hand_made(entry: &ChartProvenance) -> bool {
    matches!(
        entry,
        ChartProvenance::Local {
            instrument: Instrument::Scan
                | Instrument::Sound
                | Instrument::QuietSound
                | Instrument::Core,
            via: Via::Hand,
            ..
        }
    )
}

/// **The hand-made evidence rule** (doc 20 §9.3; cut SM.10, lead ruling 2): the earliest
/// qualifying stamp `≥ since` (`None` since = epoch 0) in the actor's own chart row for block
/// `letter`. The candidates are every `Local { via: Hand, instrument: Scan | Sound | QuietSound
/// | Core }` entry in the provenance rings of the block's four field rows (scans) and of its
/// pocket block (soundings and cores), **plus** each of those rows' `first_hand_at` — the earliest
/// hand-made stamp ever written to the row, which survives the ring's four-entry eviction
/// (reviewer F14). **Doc 20 cut SM.11** (lead ruling 4; the SM.10 handoff) revisited SM.10's
/// second clause, which trusted `first_local_at` while its ring held no `Plan` entry: a plan's
/// stamp never sets `first_hand_at`, so a plan-made discovery never qualifies a stake; and the
/// ring's trim evicts a plan's stamp before any hand stamp (`chart`'s `stamp`), so a plan's cores
/// never push an in-epoch hand stamp out (the reviewer's F7). A `via: Plan` entry never counts,
/// and another block's rows are never read.
#[must_use]
pub fn evidence_at(row: &ChartRecord, letter: char, since: Option<i64>) -> Option<i64> {
    let inside = |at: i64| since.is_none_or(|since| at >= since);
    let mut best: Option<i64> = None;
    let mut consider = |at: i64| {
        if inside(at) && best.is_none_or(|best| at < best) {
            best = Some(at);
        }
    };
    let rings = block_field_indexes(letter)
        .into_iter()
        .filter_map(|index| row.fields.get(&index))
        .map(|field| (field.first_hand_at, field.provenance.as_slice()))
        .chain(
            row.blocks
                .get(letter.to_string().as_str())
                .map(|block| (block.first_hand_at, block.provenance.as_slice())),
        );
    for (first_hand, ring) in rings {
        for entry in ring.iter().filter(|entry| hand_made(entry)) {
            consider(entry.at());
        }
        if let Some(first_hand) = first_hand {
            consider(first_hand);
        }
    }
    best
}

/// **The loud soundings heard on block `letter`** in one chart row (doc 20 §9.4; cut SM.10):
/// the stamps of `Local { Sound, at, Hand }` entries — loud only; a quiet one is heard by nobody
/// — in the block's pocket block's ring with `from ≤ at < before` (`None` from = no lower
/// bound). The ring only: `first_local_at` names no instrument (residual R3).
#[must_use]
pub fn heard_before(row: &ChartRecord, letter: char, from: Option<i64>, before: i64) -> Vec<i64> {
    let Some(block) = row.blocks.get(letter.to_string().as_str()) else {
        return Vec::new();
    };
    block
        .provenance
        .iter()
        .filter_map(|entry| match entry {
            ChartProvenance::Local {
                instrument: Instrument::Sound,
                via: Via::Hand,
                at,
            } => Some(*at),
            ChartProvenance::Local { .. } => None,
        })
        .filter(|at| from.is_none_or(|from| *at >= from) && *at < before)
        .collect()
}

/// **The stake preview's heard count** (doc 20 cut SM.10, lead ruling 8): the distinct
/// `(principal, at)` pairs of loud soundings heard on block `letter` with `from ≤ at < before`,
/// over every row of a principal other than `actor`.
#[must_use]
pub fn heard_count(
    rows: &[ChartRecord],
    actor: ObjId,
    letter: char,
    from: Option<i64>,
    before: i64,
) -> usize {
    let mut seen: std::collections::BTreeSet<(u32, i64)> = std::collections::BTreeSet::new();
    for row in rows.iter().filter(|row| row.principal != actor) {
        for at in heard_before(row, letter, from, before) {
            seen.insert((row.principal.0, at));
        }
    }
    seen.len()
}

/// **Worked this term** (lead ruling 4): the block's cursors' sum now less the term's basis,
/// saturating.
#[must_use]
pub fn worked_this_term(drawn_now: u64, basis: u64) -> u64 {
    drawn_now.saturating_sub(basis)
}

/// **The renewal's worked floor** (doc 20 §9.2, §9.6): the Registrar's
/// [`REGISTRAR_WORKED_T`] while the block's paper stands `Contested`, else
/// [`WORKED_FLOOR_T`].
#[must_use]
pub fn renewal_floor(paper_standing: bool) -> u64 {
    if paper_standing {
        REGISTRAR_WORKED_T
    } else {
        WORKED_FLOOR_T
    }
}

/// **The contest window** (doc 20 §9.4): open while `now ≤ registered +`
/// [`CONTEST_WINDOW_IC_SECS`].
#[must_use]
pub fn contest_open(registered_ic: i64, now_ic: i64) -> bool {
    registered_ic
        .checked_add(CONTEST_WINDOW_IC_SECS)
        .is_some_and(|shut| now_ic <= shut)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Doc 20 §9 and §11's numbers, pinned.
    #[test]
    fn sm6_the_tenure_numbers_are_pinned() {
        assert_eq!(CLAIM_TERM_IC_SECS, 14_515_200);
        assert_eq!(WORKED_FLOOR_T, 50);
        assert_eq!(COOLING_TERM_IC_SECS, 14_515_200);
        assert_eq!(CONTEST_WINDOW_IC_SECS, 2_592_000);
        assert_eq!(MAX_LIVE_CLAIMS_PER_ACCOUNT, 2);
        assert_eq!(REGISTRAR_WORKED_T, 100);
    }

    // ---- doc 20 cut SM.10 ----

    use crate::surface::chart::{FieldRow, PocketBlock};
    use std::collections::BTreeMap;

    fn local(instrument: Instrument, at: i64, via: Via) -> ChartProvenance {
        ChartProvenance::Local {
            instrument,
            at,
            via,
        }
    }

    /// Doc 20 cut SM.11: a hand-built row's `first_hand_at` is its first stamp when its ring
    /// holds a hand entry (every row these tests build was first stamped by the hand, or only by a
    /// plan) — what `chart`'s writers set.
    fn first_hand(first: i64, ring: &[ChartProvenance]) -> Option<i64> {
        ring.iter()
            .any(|entry| matches!(entry, ChartProvenance::Local { via: Via::Hand, .. }))
            .then_some(first)
    }

    fn field(first: i64, ring: Vec<ChartProvenance>) -> FieldRow {
        FieldRow {
            footing: crate::surface::reading::Footing::Regolith,
            first_local_at: first,
            last_at: ring.last().map_or(first, ChartProvenance::at),
            first_hand_at: first_hand(first, &ring),
            provenance: ring,
        }
    }

    fn block(first: i64, ring: Vec<ChartProvenance>) -> PocketBlock {
        PocketBlock {
            cells: "0".repeat(512),
            first_local_at: first,
            last_at: ring.last().map_or(first, ChartProvenance::at),
            first_hand_at: first_hand(first, &ring),
            provenance: ring,
        }
    }

    fn row(principal: u32) -> ChartRecord {
        ChartRecord {
            schema: crate::surface::chart::CHART_SCHEMA,
            principal: ObjId(principal),
            site: mush_core::CanonicalId::new("ahuna_port").unwrap_or_else(|_| panic!("id")),
            body: mush_core::CanonicalId::new("ceres").unwrap_or_else(|_| panic!("id")),
            rule: crate::surface::reading::READING_RULE_VERSION,
            fields: BTreeMap::new(),
            blocks: BTreeMap::new(),
            heard_ic: None,
            scan_stand: None,
            sound_stand: None,
        }
    }

    /// Block C's south-west field index (C is bx 2, by 3: fields (4..6, 6..8)).
    fn c_field() -> u8 {
        crate::surface::address::field_index(4, 6).unwrap_or_else(|| panic!("C-SW"))
    }

    /// Doc 20 cut SM.10's numbers, pinned.
    #[test]
    fn sm10_the_claim_numbers_are_pinned() {
        assert_eq!(SEEDED_TERM_IC_SECS, 3_153_600_000);
        assert_eq!(CLAIM_FEE_WINDOW_SECS, 3_600);
        assert_eq!(MAX_HEARD_SCAN_ROWS, 512);
        assert_eq!(CLAIM_ORDER_TTL_IC_SECS, 3_600);
    }

    /// Scan, quiet, loud and core qualify; a `Plan` entry, an earlier-epoch stamp and another
    /// block's ring do not; the earliest wins.
    #[test]
    fn sm10_evidence_reads_hand_made_rings_inside_the_epoch() {
        for instrument in [
            Instrument::Scan,
            Instrument::Sound,
            Instrument::QuietSound,
            Instrument::Core,
        ] {
            let mut one = row(1);
            one.blocks.insert(
                "C".to_owned(),
                block(500, vec![local(instrument, 500, Via::Hand)]),
            );
            assert_eq!(evidence_at(&one, 'C', None), Some(500), "{instrument:?}");
        }
        // A plan-made entry never counts, and its ring's first stamp is not trusted either.
        let mut plan = row(1);
        plan.blocks.insert(
            "C".to_owned(),
            block(400, vec![local(Instrument::Sound, 400, Via::Plan)]),
        );
        assert_eq!(evidence_at(&plan, 'C', None), None);
        // An earlier-epoch stamp does not count.
        let mut old = row(1);
        old.fields.insert(
            c_field(),
            field(100, vec![local(Instrument::Scan, 100, Via::Hand)]),
        );
        assert_eq!(evidence_at(&old, 'C', Some(200)), None);
        assert_eq!(evidence_at(&old, 'C', Some(100)), Some(100));
        // Another block's rows are never read.
        let mut other = row(1);
        other.blocks.insert(
            "D".to_owned(),
            block(300, vec![local(Instrument::Core, 300, Via::Hand)]),
        );
        assert_eq!(evidence_at(&other, 'C', None), None);
        // The earliest wins, across a field row and the pocket block.
        let mut both = row(1);
        both.fields.insert(
            c_field(),
            field(700, vec![local(Instrument::Scan, 700, Via::Hand)]),
        );
        both.blocks.insert(
            "C".to_owned(),
            block(650, vec![local(Instrument::Core, 650, Via::Hand)]),
        );
        assert_eq!(evidence_at(&both, 'C', Some(600)), Some(650));
    }

    /// Five hand scans: the ring keeps the newest four, and the first still counts by
    /// `first_local_at` (reviewer F14).
    #[test]
    fn sm10_evidence_survives_the_ring_by_first_local_at() {
        let mut five = row(1);
        let ring: Vec<ChartProvenance> = (2..=5)
            .map(|n| local(Instrument::Scan, n * 100, Via::Hand))
            .collect();
        five.fields.insert(c_field(), field(100, ring));
        assert_eq!(evidence_at(&five, 'C', None), Some(100));
        assert_eq!(evidence_at(&five, 'C', Some(150)), Some(200));
    }

    /// **Evidence reads `first_hand_at`, never a plan** (doc 20 cut SM.11; lead ruling 4, the
    /// reviewer's F7), through `chart`'s own writers: a block known only by plan readings yields
    /// none; a plan reading earlier than the hand one never wins; a pre-epoch hand stamp, an
    /// in-epoch hand stamp, then four plan cores of the block — the evidence is the in-epoch hand
    /// stamp (the Plan-first trim kept it); `heard_before` over a row whose only loud soundings are
    /// a plan's is empty.
    #[test]
    fn sm11_evidence_reads_first_hand_at_never_a_plan() {
        use crate::surface::chart::{write_core, write_sound};
        let c_pocket =
            crate::surface::address::parse_pocket_address("C117").unwrap_or_else(|| panic!("C117"));
        let reach =
            crate::surface::address::pockets_in_reach((c_pocket.0 * 4 + 2, c_pocket.1 * 4 + 2), 16);
        let stand = [c_pocket.0 * 4 + 2, c_pocket.1 * 4 + 2];
        // Plan readings only.
        let mut plan_only = row(1);
        write_sound(&mut plan_only, stand, &reach, false, 300, Via::Plan);
        write_core(&mut plan_only, c_pocket, 400, Via::Plan);
        assert_eq!(evidence_at(&plan_only, 'C', None), None);
        assert_eq!(
            heard_before(&plan_only, 'C', None, 10_000),
            Vec::<i64>::new()
        );
        // A plan reading earlier than the hand one never wins.
        let mut both = row(1);
        write_sound(&mut both, stand, &reach, false, 300, Via::Plan);
        write_core(&mut both, c_pocket, 500, Via::Hand);
        assert_eq!(evidence_at(&both, 'C', None), Some(500));
        // Pre-epoch hand, in-epoch hand, then four plan cores: the in-epoch hand stamp stands.
        let mut epoch = row(1);
        write_core(&mut epoch, c_pocket, 100, Via::Hand);
        write_core(&mut epoch, c_pocket, 600, Via::Hand);
        for at in [700, 800, 900, 1_000] {
            write_core(&mut epoch, c_pocket, at, Via::Plan);
        }
        assert_eq!(evidence_at(&epoch, 'C', Some(500)), Some(600));
        assert_eq!(evidence_at(&epoch, 'C', None), Some(100));
        assert_eq!(
            evidence_at(&epoch, 'C', Some(650)),
            None,
            "only plan stamps after"
        );
    }

    /// Only a loud hand sounding is heard; the window is `from ≤ at < before`.
    #[test]
    fn sm10_only_loud_soundings_are_heard() {
        let mut one = row(1);
        one.blocks.insert(
            "C".to_owned(),
            block(
                100,
                vec![
                    local(Instrument::QuietSound, 100, Via::Hand),
                    local(Instrument::Sound, 200, Via::Hand),
                    local(Instrument::Scan, 250, Via::Hand),
                    local(Instrument::Sound, 300, Via::Plan),
                ],
            ),
        );
        assert_eq!(heard_before(&one, 'C', None, 1_000), vec![200]);
        assert_eq!(heard_before(&one, 'C', Some(201), 1_000), Vec::<i64>::new());
        assert_eq!(heard_before(&one, 'C', Some(200), 200), Vec::<i64>::new());
        assert_eq!(heard_before(&one, 'D', None, 1_000), Vec::<i64>::new());
    }

    /// Distinct `(principal, at)` pairs, the actor's own row left out.
    #[test]
    fn sm10_heard_count_is_distinct_and_excludes_the_actor() {
        let loud = |principal: u32, stamps: &[i64]| {
            let mut one = row(principal);
            one.blocks.insert(
                "C".to_owned(),
                block(
                    stamps.first().copied().unwrap_or(0),
                    stamps
                        .iter()
                        .map(|at| local(Instrument::Sound, *at, Via::Hand))
                        .collect(),
                ),
            );
            one
        };
        let rows = vec![
            loud(1, &[100, 200]),
            loud(2, &[100]),
            loud(3, &[150]),
            loud(9, &[120]),
        ];
        assert_eq!(heard_count(&rows, ObjId(9), 'C', None, 1_000), 4);
        assert_eq!(heard_count(&rows, ObjId(9), 'C', Some(150), 1_000), 2);
        assert_eq!(heard_count(&rows, ObjId(9), 'C', None, 150), 2);
        assert_eq!(heard_count(&rows, ObjId(1), 'C', None, 1_000), 3);
    }

    /// The Registrar's floor while the paper stands; the window's edge.
    #[test]
    fn sm10_the_floor_is_the_registrars_while_paper_stands() {
        assert_eq!(renewal_floor(true), 100);
        assert_eq!(renewal_floor(false), 50);
        assert_eq!(worked_this_term(80, 30), 50);
        assert_eq!(worked_this_term(10, 30), 0);
        assert!(contest_open(1_000, 1_000 + CONTEST_WINDOW_IC_SECS));
        assert!(!contest_open(1_000, 1_001 + CONTEST_WINDOW_IC_SECS));
        assert!(!contest_open(i64::MAX, 0), "an overflowing window is shut");
    }
}
