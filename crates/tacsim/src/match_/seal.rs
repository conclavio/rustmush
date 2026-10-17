//! **The seal, computed** (17b cut M3.3; doc 17 §12 "Sealing", §13 "The seal";
//! D-UM-17, D-UM-45, D-UM-46).
//!
//! A sortie's seal ([`MindSeal`]) is computed world-side at the principal's
//! dispatch — in the arena at check-out, where `BOARD` then `STARTUP` is the
//! hangar's exit and nothing a seal holds can change between them — and it is
//! computed **once, by one function**, [`seal_for`], as a function of the
//! FINISHED custody: [`crate::match_::custody_unit`] builds every other field
//! and sets the seal LAST, and a rig that changes a custody after it was built
//! (the unarmed muster clears its weapon rows) calls
//! [`CustodyUnit::reseal`]. Every hand-built literal in the tree is sealed the
//! same way ([`CustodyUnit::sealed`], [`MindSeal::fixture`] — both
//! [`seal_for`] under [`SealTerms::fixture`]), so every hull in every test is
//! sealed the way production seals it (16b §0.4).
//!
//! # The order (doc 17 §13)
//!
//! 1. **The hull's loadout and stations**: the channels its stations grant —
//!    helm and sensors on every hull, fire on one that carries a weapon row.
//! 2. **The licence state**: `doctrine` (the OOC sim holds no licence before
//!    17b cut M6; D-UM-49, the owner's OQ-1).
//! 3. **The core**: its component, class, governed bit and condition, read
//!    live off the unit (`crate::arena::core::read_core`); a hull with none is
//!    Class 0, governed, and flies the ROM from the start, which its
//!    `fallback` says. **17b cut M5.2** (doc 17 §9, D-UM-41; the design
//!    note's C-24 as amended by M5.2's S1 ruling): a mind rated above a
//!    GOVERNED core's class of I or more is refused
//!    [`SealError::OverClass`] — the backstop behind the lobby's choice,
//!    which asks the same predicate first of the chassis's content-declared
//!    core (`crate::match_::lobby`'s fit, by type). This reads the core
//!    LIVE, which differs from the type's only on a tank a reset could not
//!    put back to spec (a core a `degrade_core` lowered) and in fixtures.
//!    A hull with NO core is never over-class: it flies the ROM, whatever
//!    this records as `governed` for it — and neither is a governed core
//!    whose class reads 0: Class 0 is the drive ROM's (D-UM-35), so such a
//!    hull seals Class 0 on its core and flies the ROM, as no core does
//!    (and as it did at 5f505c8). Standard Doctrine's rating I is
//!    therefore over no core, and no production seal meets the refusal.
//! 4. **The revision**: the mind the terms name ([`SealTerms::identity`]:
//!    Standard Doctrine, the registry's stock line, at the stock version, in
//!    production — the one mind production holds before 17b cut M6), its
//!    source named by its digest alone.
//! 5. **The cards** — since 17b cut M5.2 EVERY profile the terms seal
//!    ([`SealTerms::profiles`]; the Line card alone when it is empty) —
//!    each validated against the schema and the Compact floor the terms
//!    carry — refused, never clamped (D-UM-17): a card whose pair of
//!    survival sliders does not hold is [`SealError::Card`] on every hull;
//!    one whose `min_odds` is under the floor is [`SealError::Unlawful`] on a
//!    hull with a weapons station only (17b cut M5.2, the design note's
//!    C-23 and doc 17 §11's R-60: a fire section a hull cannot use is shown
//!    inert and never refuses the seal). The first profile's card is
//!    [`MindSeal::card`]; the set is [`MindSeal::profiles`], written LAST and
//!    empty — skipped — when it is the Line card alone, so a default seal is
//!    the record it always was.
//!
//! The class the mind thinks at is `min(rated, core_class, class_cap)` under
//! the terms' [`SealTerms::rated`] (Standard Doctrine's `STOCK_RATED_CLASS`)
//! and [`SealTerms::class_cap`] (17b cut M3's ruling, the plan's F4; 17b cut
//! M4.1 put the rating and the cap on the terms and the think side reads the
//! seal, `Arena::mind_class`); the engine is the terms'
//! [`SealTerms::engine_for_minds`] on every hull (17b cut M4.3), which in
//! production is [`PRODUCTION_FILL_ENGINE`] — the native stock mind since
//! the fill decision of 2026-10-09.

use crate::arena::card::{CardError, PRESETS, StockCard};
use crate::arena::conditions::{ASPECTS_ANY, FireCondition, SlotTarget, condition_is_lawful};
use crate::arena::core::{CoreClass, read_core};
use crate::arena::mind::{MindSeal, STOCK_RATED_CLASS, SealEngine, SealedProfile, SortieId};
use crate::arena::state::{Channel, CustodyUnit};
use crate::arena_task::MatchId;
use crate::config::ReflexConfig;
use crate::deliberation::order::CharacterRef;
use smol_str::SmolStr;

/// **The stock mind's identity on a seal** — its name, charter, line and the
/// licence a sim seal records — under the paths they had here: 17b cut M4.6a
/// moved the four into [`crate::arena::doctrine`], so the arena can recognise
/// Standard Doctrine by identity without reaching into the match half.
pub use crate::arena::doctrine::{SEALED_CHARTER, SEALED_LICENCE, SEALED_LINE, SEALED_MIND};

/// **A mind's identity on a seal** (17b cut M5.1; doc 17 §13, D-UM-46):
/// exactly the five identity fields [`seal_for`] writes on a [`MindSeal`] —
/// the name, the charter, the line, the revision and the source's digest.
///
/// What a holding names (`crate::match_::holdings::HeldMind::identity`), so a
/// mind chosen at the lobby is the mind a seal can name. 17b cut M5.1 built
/// it and reads it at the choice; 17b cut M5.2 puts it on the terms
/// ([`SealTerms::identity`]), which [`seal_for`] writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SealIdentity {
    /// The mind's name (`Standard Doctrine`).
    pub mind: SmolStr,
    /// Its charter (`Registry`).
    pub charter: SmolStr,
    /// Its line (`standard`).
    pub line: SmolStr,
    /// Its revision on that line.
    pub revision: u32,
    /// The SHA-256 of its source's bytes — the key the source store holds it
    /// under (`tacsim:mindsrc:<sha256>`).
    pub source_sha256: [u8; 32],
}

impl SealIdentity {
    /// **Standard Doctrine's identity** — [`SEALED_MIND`], [`SEALED_CHARTER`],
    /// [`SEALED_LINE`], [`crate::minds::STOCK_VERSION`] and the shipped
    /// source's digest ([`crate::minds::content::standard_digest`]) — the
    /// five [`seal_for`] writes on every production seal, and under
    /// [`SealTerms::fixture`].
    #[must_use]
    pub fn doctrine() -> SealIdentity {
        SealIdentity {
            mind: SmolStr::new_static(SEALED_MIND),
            charter: SmolStr::new_static(SEALED_CHARTER),
            line: SmolStr::new_static(SEALED_LINE),
            revision: crate::minds::STOCK_VERSION,
            source_sha256: crate::minds::content::standard_digest().0,
        }
    }

    /// **Whether this is Standard Doctrine** — by identity, the rule
    /// [`MindSeal::seal_ref`] derives `SealRef::doctrine` by: the name, the
    /// charter and the line are the Registry's own, whatever the revision.
    #[must_use]
    pub fn is_doctrine(&self) -> bool {
        self.mind == SEALED_MIND && self.charter == SEALED_CHARTER && self.line == SEALED_LINE
    }
}

/// **Whether `card`'s odds are lawful under `floor_permille`** (17b cut M5.1):
/// the Compact-floor probe [`seal_for`] runs — a best-hostile slot at the
/// card's `min_odds`, held to [`condition_is_lawful`] — lifted out so the
/// lobby's choice (`crate::match_::lobby`'s fit) asks the seal's own
/// question rather than a copy of it.
#[must_use]
pub fn odds_are_lawful(card: &StockCard, floor_permille: u32) -> bool {
    let probe = FireCondition {
        weapon: SmolStr::default(),
        target: SlotTarget::BestHostile,
        min_permille: card.min_odds_permille,
        aspects: ASPECTS_ANY,
        min_rounds_left: 0,
        min_range_m: 0,
        max_range_m: u32::MAX,
    };
    condition_is_lawful(&probe, floor_permille).is_ok()
}

/// **Why a hull flies the ROM from its first think**: it has no core
/// (D-UM-33).
pub const NO_CORE_FALLBACK: &str = "no core: the ROM";

/// **What a seal is taken under, beside the custody** (17b cut M3.3): the
/// match it mints the sortie in, the warrant principal, the card and the
/// Compact floor the card is validated against — and since 17b cut M5.2 the
/// mind's identity, the profiles sealed and why a substituted hull flies
/// Standard Doctrine (`fallback`).
///
/// **`rated` and `engine_for_minds` stay the terms' own and authoritative**
/// (17b cut M5.2): [`seal_for`] takes only the five identity fields from
/// [`SealTerms::identity`], never a rating or an engine, so every fixture
/// lever ([`SealTerms::rated`], a scripted muster's engine, the harness's
/// Load III heavies) stays whole and no seal digest moves by a second source
/// of either.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SealTerms {
    /// The match the sortie is minted in ([`SortieId::in_match`]).
    pub match_id: MatchId,
    /// The warrant principal: the hull's first seated pilot, else the side
    /// lead's character, else `None` (a staff match; a side with no lead).
    pub principal: Option<CharacterRef>,
    /// **The card**: the first sealed profile's — the one that flies at the
    /// release, [`MindSeal::card`]. With [`SealTerms::profiles`] empty it is
    /// the one card sealed, the Line preset over the match's doctrine on
    /// every hull nobody chose a mind for; with a set, [`seal_for`] seals the
    /// set's first card, which [`SealTerms::with_profiles`] writes here too.
    pub card: StockCard,
    /// The Compact floor, per mille (`[tacsim.minds].odds_floor_permille`).
    pub odds_floor_permille: u32,
    /// **The engine a hull's mind is sealed on** (17b cut M3.5; doc 17 §13):
    /// what [`seal_for`] writes as [`MindSeal::engine`] — for EVERY hull from
    /// 17b cut M4.3, a seated one included, so a release hands the hull to a
    /// mind on the engine its seal names: [`PRODUCTION_FILL_ENGINE`] in
    /// production (`crate::match_::start`), [`SealEngine::Native`] under the
    /// fixture terms, and [`SealEngine::Scripted`] where a fixture's rig
    /// fields a scripted mind (`tests/support`'s `Muster::scripted`), whose
    /// runner `crate::minds::install_sealed_runners` then installs from the
    /// source the seal's digest names. [`SealEngine::Reflex`] is decode-only
    /// (the M3 records).
    pub engine_for_minds: SealEngine,
    /// **The class the mind is rated at** (17b cut M4.1; doc 17 §9, D-UM-35;
    /// the M4 row's finding (9)): Standard Doctrine's
    /// [`STOCK_RATED_CLASS`] (I) on every production seal — production holds
    /// no other mind before 17b cut M6 (the owner's OQ-2) — and a fixture's
    /// own (Class III for the load mind). The seal's `class` is
    /// `min(rated, core, class_cap)`, and the think side reads it off the
    /// seal (`Arena::mind_class`) — the arena-wide `rated_class` this
    /// replaces is gone. **17b cut M5.2**: a rating above a governed core's
    /// class is refused, [`SealError::OverClass`], never capped by the core
    /// — save a governed core at Class 0, which flies the ROM, as no core
    /// does (the S1 ruling).
    pub rated: CoreClass,
    /// **The class the hull is capped at** (17b cut M4.1; doc 17 §9, D-UM-37:
    /// the lead's or the operator's cap): `[tacsim.minds].fill_class` on
    /// every production seal from 17b cut M4.3 (Class I stock, so in
    /// production it caps nothing until the cut that rates a mind above
    /// Class I — the owner's OQ-10 (b)); Class III — no cap — under the
    /// fixture terms.
    pub class_cap: CoreClass,
    /// **The mind's identity** (17b cut M5.2; [`SealIdentity`]): the five
    /// fields [`seal_for`] writes — name, charter, line, revision and the
    /// source's digest — [`SealIdentity::doctrine`] on every production seal
    /// (the same constants, so the same bytes) and under the fixture terms;
    /// a fixture mind's own through the holdings seam.
    pub identity: SealIdentity,
    /// **The profiles sealed** (17b cut M5.2; doc 17 §11, §12; the owner's
    /// OQ-3): each a preset's name (one of [`PRESETS`], case-folded) with its
    /// card over the match's doctrine, in the order chosen, the first the one
    /// that flies at the release. **Empty is the Line card alone** —
    /// [`SealTerms::card`] sealed as `Line` — which every hull nobody chose a
    /// mind for and every fixture is sealed with.
    pub profiles: Vec<(SmolStr, StockCard)>,
    /// **Why the hull flies a mind other than the one chosen for it, from the
    /// start** (17b cut M5.2; doc 17 §21's Q-UM-15 at the owner's OQ-5):
    /// [`HUMANS_FALLBACK`] when the check-out substituted Standard Doctrine
    /// for a non-Doctrine choice with humans on both sides
    /// (`crate::match_::start` off `SealChoice::substituted`), else `None` —
    /// on every production seal (production holds Standard Doctrine alone
    /// before 17b cut M6, the owner's OQ-2) and under the fixture terms.
    /// [`seal_for`] writes it as [`MindSeal::fallback`], unless the hull has
    /// no core, whose [`NO_CORE_FALLBACK`] says what actually flies (the ROM).
    pub fallback: Option<SmolStr>,
}

/// **The fallback a substituted seal records** (17b cut M5.2; Q-UM-15 at the
/// owner's OQ-5): a non-Doctrine choice sealed as Standard Doctrine because
/// humans are on both sides ([`SealTerms::fallback`]).
pub const HUMANS_FALLBACK: &str = "humans on the other side";

/// **The engine the AI fill is sealed on — the fill decision, taken
/// 2026-10-09** (the M4 row's (ii) and (iii); doc 17 §18, §22's M4):
/// [`SealEngine::Native`], Standard Doctrine on the native stock mind — the
/// fast path doc 17 §18 kept in reserve, taken. `crate::match_::start` seals
/// EVERY hull on it (17b cut M4.3), the fill's and a seated pilot's alike,
/// so a release hands any hull to a native mind too. The Rhai Standard
/// Doctrine (`crate::minds::STANDARD_RHAI` on [`SealEngine::Scripted`]) was
/// the candidate under test from 17b cut M4.2 and the production fill's
/// engine from 17b cut M4.3's flip to this decision; it stays the exact
/// reference the native mind is held equal to (Gate A and the twin, no
/// divergence), and a scripted seal still runs it — a fixture's
/// (`tests/support`'s `Muster::scripted`), or a match checked out before the
/// decision, which resumed on the runner its seal names until 17b cut M4.6's
/// format 9, which voids every older snapshot.
///
/// THE RULE (17b cut M4.2): the decision was the verdict of the minds lane's
/// fill variant (`tacsim_harness_arena_minds` in
/// `crates/tacsim/tests/harness.rs`, which seals its thirty-six Class I hulls
/// a side on this constant) over three idle-host weekly runs
/// (`scripts/harness-weekly.sh` on ALIEN18, the owner's host): `held=yes`
/// three times keeps `Scripted`; any `held=no` moves this to `Native`, with
/// the three `harness_minds_verdict: variant=fill_authored` lines in its
/// commit's body and in 17b §2. A run whose line reads `sound=no` is a lane
/// fault (a failed batch, a refused snapshot, a cold field), re-run and
/// never counted. THE OUTCOME: the three counted runs all read `held=no
/// sound=yes`, so the fill is `Native`. From the decision the fill variant
/// measures the production fill as it now is — native, no Rhai, ops nought —
/// against the same lenses; its `held=` word is a capacity reading for the
/// owner, no longer a decision input.
///
/// DECISION LOG (filled at the decision's commit; each line the run's
/// `harness_minds_verdict` line's `held=` word, then the verdict's own
/// failure words in full; the date, the HEAD and the host are the owner's
/// upload's — the tree `f19a88fb` its heading names and each ALIEN18 run's
/// own heading — rather than a report's `verdict` object):
///   run 1: `held=no` — 2026-10-09, HEAD f19a88fb, ALIEN18 (windows):
///          "per-wake p99 17.297 ms is past the 12 ms lens"
///   run 2: `held=no` — 2026-10-09, HEAD f19a88fb, ALIEN18 (windows):
///          "per-wake p99 20.750 ms is past the 12 ms lens; per-window p99
///          20.750 ms over the 48 comparable windows (the lattice's
///          substeps, inline think time inside Wake.ms) is past the 20 ms
///          lens of the authored cohort"
///   run 3: `held=no` — 2026-10-09, HEAD f19a88fb, ALIEN18 (windows):
///          "per-wake p99 17.817 ms is past the 12 ms lens"
///   each `sound=yes`; the CPU lens held on all three (p99 6.070, 6.395 and
///   5.980 ms against computed lenses of 7.995, 10.026 and 9.010 ms)
///   context, not counted: the VPS conclav.io (linux, the production host),
///   runs 1-3 the same day, `held=no` on every lens, the CPU lens included
///   (wake p99 35.341, 37.587 and 38.223 ms; CPU p99 15.455, 15.837 and
///   16.735 ms against 12.564, 12.690 and 13.326); each host's no-jitter
///   diagnostic (`TACSIM_MINDS_JITTER_PER_MILLE=0`, never a decision run):
///   ALIEN18 `held=yes` (wake p99 9.946 ms), the VPS `held=no` (wake p99
///   19.023 ms, window p99 35.961 ms); `ops_total=1763485` on all eight
///   lines — the interpreted work is the same on Windows and Linux
///   stock_ci precondition waived by the owner, 2026-10-08 (17b cut M4.5);
///   the three runs remain the owner's
pub const PRODUCTION_FILL_ENGINE: SealEngine = SealEngine::Native;

impl SealTerms {
    /// **The terms every fixture seals under**: match 0, no principal, the
    /// Line preset over the default doctrine, the seed floor of 100, a
    /// mind's hull sealed native, rated at the stock class and uncapped —
    /// Standard Doctrine's identity, the Line card alone (17b cut M5.2).
    #[must_use]
    pub fn fixture() -> SealTerms {
        SealTerms {
            match_id: MatchId(0),
            principal: None,
            card: crate::arena::card::line(&ReflexConfig::default()),
            odds_floor_permille: 100,
            engine_for_minds: SealEngine::Native,
            rated: STOCK_RATED_CLASS,
            class_cap: CoreClass::III,
            identity: SealIdentity::doctrine(),
            profiles: Vec::new(),
            fallback: None,
        }
    }

    /// **These terms sealing `profiles`** (17b cut M5.2): the set in order,
    /// and [`SealTerms::card`] the first's — the card that flies at the
    /// release. An empty set leaves the card as it was (the Line card alone).
    #[must_use]
    pub fn with_profiles(self, profiles: Vec<(SmolStr, StockCard)>) -> SealTerms {
        let card = match profiles.first() {
            Some((_, first)) => first.clone(),
            None => self.card.clone(),
        };
        SealTerms {
            card,
            profiles,
            ..self
        }
    }

    /// **These terms with the mind rated at `class`** (17b cut M4.1) — a
    /// rig's lever for a hull that thinks above the stock rating.
    #[must_use]
    pub fn rated(self, class: CoreClass) -> SealTerms {
        SealTerms {
            rated: class,
            ..self
        }
    }

    /// **The production fill's terms over the fixture's** (17b cut M4.3):
    /// what `crate::match_::start` seals every hull under at the stock keys
    /// — [`PRODUCTION_FILL_ENGINE`], rated I, capped at the default
    /// `[tacsim.minds].fill_class` — for the pins that field the production
    /// shape (`tests/support`'s `Muster::fill`). The fill is native since the
    /// fill decision (2026-10-09), so the SEALS these terms make are
    /// [`SealTerms::fixture`]'s byte for byte; the terms themselves differ in
    /// `class_cap` alone (`I` here, `III` there), which seals no byte —
    /// `min(I, core, I)` is `min(I, core, III)` on every shipped core, and a
    /// seal records the class, never the cap. Under the scripted fill (17b
    /// cut M4.3 to the decision) it was what made a pin the production shape,
    /// and it stays the one name for that shape whatever the constant reads.
    #[must_use]
    pub fn production_fixture() -> SealTerms {
        SealTerms {
            engine_for_minds: PRODUCTION_FILL_ENGINE,
            class_cap: crate::config::MindsConfig::default().fill_class,
            ..SealTerms::fixture()
        }
    }
}

/// **Why a seal was refused** (17b cut M3.3; D-UM-17: refuse, never clamp).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SealError {
    /// A card breaks its own schema (its pair of survival sliders) — or
    /// (17b cut M5.2) a profile is no preset, or more than four are sealed,
    /// which no lobby choice reaches (`SIM MIND` refuses both first).
    #[error("{0}")]
    Card(CardError),
    /// A card's `min_odds` is under the Compact floor, on a hull with a
    /// weapons station (17b cut M5.2; C-23).
    #[error("the card asks for odds under the Compact floor of {}%", .floor / 10)]
    Unlawful {
        /// The floor, per mille.
        floor: u32,
    },
    /// **The mind is rated above its governed core's class** (17b cut
    /// M5.2; doc 17 §9, D-UM-41; the design note's C-24 as amended by
    /// M5.2's S1 ruling): `read_core` finds a core, it is governed, its
    /// class is I or more, and the terms' rating is above it. Never on a
    /// hull with no core, nor on a governed core whose class reads 0: both
    /// seal Class 0 and fly the ROM. The backstop behind the lobby's choice,
    /// which refuses the same predicate first on the chassis's
    /// content-declared core, so a choice never reaches it; only a live core
    /// below its type's class does (a tank a reset could not put back to
    /// spec), and only under a rating above Class I — a fixture mind's, since
    /// production's one mind is rated I. Built against the fixture mind (the
    /// owner's OQ-10 (d)).
    #[error(
        "the mind is rated Class {}, over its governed core's Class {}",
        .rated.as_str(),
        .core.as_str()
    )]
    OverClass {
        /// The mind's rating.
        rated: CoreClass,
        /// The core's class as fitted.
        core: CoreClass,
    },
}

/// **Every preset's name, as a refusal lists them** — what
/// [`SealError::Card`] names for a profile that is no preset.
const PRESET_CHOICES: &str = "Line, Sentry, Sniper, Skirmisher, Hunter or Rearguard";

/// **The one seal computation** (17b cut M3.3; this module's head): `unit`'s
/// seal under `terms`, read off the FINISHED custody — its cells, id,
/// controller and weapon rows.
///
/// # Errors
///
/// [`SealError::OverClass`] when the mind is rated above its governed core's
/// class; [`SealError::Card`] when a card's pair does not hold (or a profile
/// is no preset, or more than four are sealed); and [`SealError::Unlawful`]
/// when an armed hull's card has its `min_odds` under the floor.
pub fn seal_for(unit: &CustodyUnit, terms: &SealTerms) -> Result<MindSeal, SealError> {
    // (1) The stations: helm and sensors on every hull, fire on an armed one.
    let armed = !unit.weapons.is_empty();
    let mut channels = vec![Channel::Helm, Channel::Sensors];
    if armed {
        channels.push(Channel::Fire);
    }
    // (2) The licence is `doctrine` (below); (3) the core, read live.
    let hardware = read_core(&unit.unit);
    // **17b cut M5.2** (C-24, as its S1 ruling amended it): over-class is a
    // governed core's class under the rating — read off the core itself, so
    // a hull with no core (which this seal records `governed`, Class 0) is
    // never over-class, and a governed core whose class reads 0 flies the
    // ROM, as no core does (`lobby::fit_on_chassis` asks the same).
    if let Some(hw) = hardware.as_ref()
        && hw.governed
        && terms.rated > hw.class
        && hw.class > CoreClass::Zero
    {
        return Err(SealError::OverClass {
            rated: terms.rated,
            core: hw.class,
        });
    }
    let (core, core_class, governed, fallback) = match hardware {
        Some(hardware) => (
            Some(hardware.component),
            hardware.class,
            hardware.governed,
            None,
        ),
        None => (
            None,
            CoreClass::Zero,
            true,
            Some(SmolStr::new_static(NO_CORE_FALLBACK)),
        ),
    };
    // **17b cut M5.2** (Q-UM-15): a substituted seal says why — unless the
    // hull has no core, whose ROM is what flies.
    let fallback = fallback.or_else(|| terms.fallback.clone());
    // (5) The cards — every profile sealed (17b cut M5.2), the Line card
    // alone when the terms name none — each against its schema on every hull
    // and (17b cut M5.2, C-23) against the floor on an armed hull only,
    // refuse-not-clamp.
    let sealed = sealed_set(terms)?;
    for &(name, card) in &sealed {
        card.pair(name).map_err(SealError::Card)?;
        // 17b cut M5.1: the probe lifted into `odds_are_lawful`, the one
        // question the lobby's choice asks too; the predicate is unchanged.
        if armed && !odds_are_lawful(card, terms.odds_floor_permille) {
            return Err(SealError::Unlawful {
                floor: terms.odds_floor_permille,
            });
        }
    }
    // `StockCard` is strings, numbers and booleans: it always serialises.
    let bytes = |card: &StockCard| serde_json::to_vec(card).unwrap_or_default();
    let digest = |bytes: &[u8]| -> [u8; 32] {
        use sha2::{Digest as _, Sha256};
        Sha256::digest(bytes).into()
    };
    // The first profile's card flies at the release (`sealed` is never
    // empty: the Line card alone at least).
    let card = sealed
        .first()
        .map(|(_, card)| bytes(card))
        .unwrap_or_default();
    let card_digest = digest(&card);
    // **17b cut M5.2** (D3, F4): the set by name with each card's digest —
    // EMPTY exactly when it is the Line card alone, so the field is skipped
    // and a default seal is the record it always was.
    let profiles = match sealed.as_slice() {
        [(name, _)] if *name == "Line" => Vec::new(),
        set => set
            .iter()
            .map(|&(name, card)| SealedProfile {
                name: SmolStr::new_static(name),
                card_digest: digest(&bytes(card)),
            })
            .collect(),
    };
    let identity = &terms.identity;
    Ok(MindSeal {
        unit: unit.id,
        sortie: SortieId::in_match(terms.match_id.0, unit.id),
        core,
        core_class,
        // 17b cut M4.1: `min(rated, core, cap)` — the terms' rating and cap,
        // where it was `min(STOCK_RATED_CLASS, core)`.
        class: terms.rated.min(core_class).min(terms.class_cap),
        governed,
        // (4) The revision — the terms' identity (17b cut M5.2): Standard
        // Doctrine's five constants in production, so the same bytes. Its
        // digest is the script's own bytes', which `tests/content.rs` pins
        // equal to `STANDARD_SHA256` — the key the source store holds it
        // under.
        mind: identity.mind.clone(),
        charter: identity.charter.clone(),
        line: identity.line.clone(),
        revision: identity.revision,
        source_sha256: identity.source_sha256,
        card,
        card_digest,
        armed: Vec::new(),
        abi: SmolStr::new_static(crate::minds::MIND_ABI),
        licence: SmolStr::new_static(SEALED_LICENCE),
        principal: terms.principal.clone(),
        channels,
        odds_floor_permille: terms.odds_floor_permille,
        fallback,
        // **17b cut M4.3**: every hull — a `Released` hands any hull to a
        // mind, so a seated hull's seal names the engine the mind that takes
        // it runs on (a fixture's reflex hull likewise, until 17b cut M4.6
        // deleted the reflex controller). `SealEngine::Reflex` is decode-only
        // from here: every live record an M3.3–M4.2 build wrote carries it
        // (the M4 row's finding (6)), and no seal computes it (`SealRef::none`
        // and `MindSeal::placeholder` still default to it, unhashed: the
        // conventions' D8).
        engine: terms.engine_for_minds,
        // 17b cut M5.2: LAST, and skipped when empty.
        profiles,
    })
}

/// **The set a seal is taken over** (17b cut M5.2): `terms.profiles` with
/// each name canonically spelled — or `[("Line", &terms.card)]` when it is
/// empty, the Line card alone.
///
/// # Errors
///
/// [`SealError::Card`] naming the presets for a profile that is none of
/// them, and a range of 1 to 4 for a set of more than four — neither
/// reachable through the lobby, which refuses both at the choice.
fn sealed_set(terms: &SealTerms) -> Result<Vec<(&'static str, &StockCard)>, SealError> {
    if terms.profiles.is_empty() {
        return Ok(vec![("Line", &terms.card)]);
    }
    if terms.profiles.len() > crate::arena::card::ProfileSet::MOST {
        return Err(SealError::Card(CardError::Range {
            slider: "profiles",
            low: 1,
            high: 4,
            step: 1,
        }));
    }
    terms
        .profiles
        .iter()
        .map(|(name, card)| {
            crate::arena::card::preset_code(name)
                .and_then(|code| PRESETS.get(usize::from(code)).copied())
                .map(|name| (name, card))
                .ok_or(SealError::Card(CardError::Choice {
                    slider: "profile",
                    choices: PRESET_CHOICES,
                }))
        })
        .collect()
}

impl MindSeal {
    /// **A fixture's seal**: [`seal_for`] under [`SealTerms::fixture`] — the
    /// one computation, never a second one. The fixture terms are lawful by
    /// construction on every hull a fixture fields (the Line preset at the
    /// seed floor, rated I).
    ///
    /// # Panics
    ///
    /// Never: the fixture card pairs and sits on the floor, and rated I is
    /// over no core (17b cut M5.2's over-class rule) — a hull with no core,
    /// and a governed core whose class cell reads 0, seal Class 0 and fly the
    /// ROM (the rule as M5.2's S1 ruling amended it), and a core of Class I
    /// or more is at or above the rating.
    #[must_use]
    pub fn fixture(unit: &CustodyUnit) -> MindSeal {
        seal_for(unit, &SealTerms::fixture()).expect("a fixture seals")
    }
}

impl CustodyUnit {
    /// **Seal the custody again, as it now stands** (17b cut M3.3): the seal
    /// is a function of the finished custody, so a rig that changes one after
    /// [`crate::match_::custody_unit`] built it (the unarmed muster clears its
    /// weapon rows) recomputes it here.
    ///
    /// # Errors
    ///
    /// [`seal_for`]'s, leaving the seal as it was.
    pub fn reseal(&mut self, terms: &SealTerms) -> Result<(), SealError> {
        self.seal = seal_for(self, terms)?;
        Ok(())
    }

    /// **This custody, sealed under the fixture terms** — a hand-built
    /// literal's last step (`CustodyUnit { …, seal: MindSeal::placeholder()
    /// }.sealed()`), and a changed one's: [`MindSeal::fixture`] over the
    /// custody as it stands.
    #[must_use]
    pub fn sealed(mut self) -> CustodyUnit {
        self.seal = MindSeal::fixture(&self);
        self
    }
}

#[cfg(test)]
mod tests {
    use mush_core::ObjId;
    use smol_str::SmolStr;

    use super::*;
    use crate::arena::state::Controller;
    use crate::arena::state::Team;
    use crate::definitions::Catalog;

    /// The shipped content, as `match_::start` reads it.
    fn catalog() -> Catalog {
        Catalog::build_fielded(&[], "tacsim").expect("the fielded content")
    }

    /// One shipped `entry` checked out as `id` under `controller` and `terms`,
    /// through the production assembly ([`crate::match_::custody_unit`]).
    fn checked_out(
        catalog: &Catalog,
        entry: &str,
        id: ObjId,
        controller: Controller,
        terms: &SealTerms,
    ) -> Result<CustodyUnit, SealError> {
        let key = mush_core::CanonicalId::new(entry).expect("a canonical id");
        let cells = crate::definitions::to_tactical_unit(
            catalog.specs.get(&key).expect("a shipped entry"),
            catalog.bindings.get(&key).expect("its binding"),
            id,
        )
        .expect("the entry instantiates");
        let posture =
            mush_sim::data::passive_rules_value(serde_json::json!({})).expect("an empty posture");
        crate::match_::custody_unit(
            catalog,
            cells,
            id,
            Team::Red,
            mush_sim::data::Pose::at(1_500, 1_500, 0, posture),
            0,
            controller,
            1,
            terms,
        )
    }

    /// Match 7, with a principal.
    fn terms() -> SealTerms {
        SealTerms {
            match_id: MatchId(7),
            principal: Some(CharacterRef::Character { id: ObjId(77) }),
            ..SealTerms::fixture()
        }
    }

    /// **17b cut M3.3** (doc 17 §13): every field of a shipped line tank's
    /// seal at check-out, on a pilot's hull and a mind's hull, and the shipped
    /// target's, which has no core.
    ///
    /// 17b cut M4.5a: the fill hull IS a mind's since 17b cut M4.3 (and
    /// `seal_for` reads no controller), so its row is the mind's; the target is
    /// fielded as the fill is, on a mind.
    #[test]
    fn m3_a_shipped_line_tank_is_sealed_field_by_field_at_check_out() {
        use crate::definitions::content::{LINE_TANK, TARGET_MK1};
        let catalog = catalog();
        let terms = terms();
        let card = serde_json::to_vec(&terms.card).expect("a card");
        let card_digest: [u8; 32] = {
            use sha2::{Digest as _, Sha256};
            Sha256::digest(&card).into()
        };
        // 17b cut M4.3: every controller seals the terms' engine.
        for (controller, engine) in [
            (Controller::Pilot, SealEngine::Native),
            (Controller::Mind, SealEngine::Native),
        ] {
            let unit = checked_out(&catalog, LINE_TANK, ObjId(4), controller, &terms)
                .expect("the line tank seals");
            assert_eq!(
                unit.seal,
                MindSeal {
                    unit: ObjId(4),
                    sortie: SortieId((7_u64 << 32) | 4),
                    core: Some(SmolStr::new_static("core")),
                    core_class: CoreClass::II,
                    class: CoreClass::I,
                    governed: true,
                    mind: SmolStr::new_static("Standard Doctrine"),
                    charter: SmolStr::new_static("registry"),
                    line: SmolStr::new_static("standard"),
                    revision: crate::minds::STOCK_VERSION,
                    source_sha256: crate::arena::digest_from_text(
                        crate::minds::content::STANDARD_SHA256
                    )
                    .expect("the pinned digest"),
                    card: card.clone(),
                    card_digest,
                    armed: Vec::new(),
                    abi: SmolStr::new_static("rhai=1.26.0;api=3;stock=2"),
                    licence: SmolStr::new_static("doctrine"),
                    principal: Some(CharacterRef::Character { id: ObjId(77) }),
                    channels: vec![Channel::Helm, Channel::Sensors, Channel::Fire],
                    odds_floor_permille: 100,
                    fallback: None,
                    engine,
                    // 17b cut M5.2: the Line card alone writes no set.
                    profiles: Vec::new(),
                },
                "{controller:?}"
            );
            // The reference the arena keeps.
            let reference = unit.seal.seal_ref();
            assert_eq!(reference.sortie, unit.seal.sortie);
            assert_eq!(reference.digest, unit.seal.digest());
            assert_eq!(
                (reference.core_class, reference.class, reference.engine),
                (CoreClass::II, CoreClass::I, engine)
            );
            assert!(reference.governed && !reference.closed);
            assert_eq!(
                reference.profiles,
                crate::arena::card::ProfileSet::LINE,
                "17b cut M5.2: the Line card alone"
            );
        }
        // The target: no core, Class 0, governed, the ROM from the start, and
        // no weapon row — so no fire channel.
        let target = checked_out(&catalog, TARGET_MK1, ObjId(9), Controller::Mind, &terms)
            .expect("the target seals");
        assert_eq!(target.seal.core, None);
        assert_eq!(target.seal.core_class, CoreClass::Zero);
        assert_eq!(target.seal.class, CoreClass::Zero);
        assert!(target.seal.governed);
        assert_eq!(target.seal.fallback.as_deref(), Some("no core: the ROM"));
        assert_eq!(target.seal.channels, vec![Channel::Helm, Channel::Sensors]);
        assert_eq!(target.seal.sortie, SortieId::in_match(7, ObjId(9)));
    }

    /// **17b cut M3.3**: the channel set reads the FINISHED custody — an
    /// armed hull's seal grants fire, and the same hull with its weapon rows
    /// cleared and sealed again (`CustodyUnit::reseal`) grants none.
    #[test]
    fn m3_the_fire_channel_follows_the_weapon_rows_through_a_reseal() {
        let catalog = catalog();
        let mut unit = checked_out(
            &catalog,
            crate::definitions::content::LINE_TANK,
            ObjId(4),
            Controller::Mind,
            &SealTerms::fixture(),
        )
        .expect("seals");
        assert!(unit.seal.channels.contains(&Channel::Fire));
        unit.weapons.clear();
        assert!(
            unit.seal.channels.contains(&Channel::Fire),
            "until it is sealed again"
        );
        unit.reseal(&SealTerms::fixture()).expect("seals again");
        assert_eq!(unit.seal.channels, vec![Channel::Helm, Channel::Sensors]);
    }

    /// **17b cut M3.3**: the digest is a function of the record — two
    /// computations agree — and every field moves it; each edited record
    /// reads back as written.
    #[test]
    fn m3_the_seal_digest_is_stable_and_moved_by_every_field() {
        type Edit = Box<dyn Fn(&mut MindSeal)>;
        let catalog = catalog();
        let unit = checked_out(
            &catalog,
            crate::definitions::content::LINE_TANK,
            ObjId(4),
            Controller::Mind,
            &terms(),
        )
        .expect("seals");
        let again = seal_for(&unit, &terms()).expect("seals");
        assert_eq!(unit.seal, again);
        assert_eq!(unit.seal.digest(), again.digest());
        let base = unit.seal.digest();
        let edits: Vec<(&str, Edit)> = vec![
            ("unit", Box::new(|s| s.unit = ObjId(5))),
            ("sortie", Box::new(|s| s.sortie = SortieId(1))),
            ("core", Box::new(|s| s.core = None)),
            ("core_class", Box::new(|s| s.core_class = CoreClass::III)),
            ("class", Box::new(|s| s.class = CoreClass::II)),
            ("governed", Box::new(|s| s.governed = false)),
            ("mind", Box::new(|s| s.mind = SmolStr::new("Sentinel"))),
            ("charter", Box::new(|s| s.charter = SmolStr::new("author"))),
            ("line", Box::new(|s| s.line = SmolStr::new("mine"))),
            ("revision", Box::new(|s| s.revision += 1)),
            ("source_sha256", Box::new(|s| s.source_sha256[0] ^= 1)),
            ("card", Box::new(|s| s.card.push(b' '))),
            ("card_digest", Box::new(|s| s.card_digest[31] ^= 1)),
            (
                "armed",
                Box::new(|s| {
                    s.armed.push(FireCondition {
                        weapon: SmolStr::new("cannon"),
                        target: SlotTarget::BestHostile,
                        min_permille: 100,
                        aspects: ASPECTS_ANY,
                        min_rounds_left: 0,
                        min_range_m: 0,
                        max_range_m: u32::MAX,
                    });
                }),
            ),
            (
                "abi",
                Box::new(|s| s.abi = SmolStr::new("rhai=1.25.0;api=1;stock=1")),
            ),
            ("licence", Box::new(|s| s.licence = SmolStr::new("core3"))),
            ("principal", Box::new(|s| s.principal = None)),
            (
                "channels",
                Box::new(|s| {
                    s.channels.pop();
                }),
            ),
            (
                "odds_floor_permille",
                Box::new(|s| s.odds_floor_permille = 110),
            ),
            (
                "fallback",
                Box::new(|s| s.fallback = Some(SmolStr::new(NO_CORE_FALLBACK))),
            ),
            ("engine", Box::new(|s| s.engine = SealEngine::Scripted)),
            // 17b cut M5.2: the sealed set, written when it is more than the
            // Line card alone.
            (
                "profiles",
                Box::new(|s| {
                    s.profiles.push(crate::arena::mind::SealedProfile {
                        name: SmolStr::new_static("Sniper"),
                        card_digest: [3; 32],
                    });
                }),
            ),
        ];
        // 17b cut M5.2: 21 → 22, `profiles` (the seal's one new field).
        assert_eq!(edits.len(), 22, "every field of the seal");
        for (field, edit) in edits {
            let mut moved = unit.seal.clone();
            edit(&mut moved);
            assert_ne!(moved.digest(), base, "{field} does not move the digest");
            // The 32-byte digests ride as 64 lowercase base-16 characters.
            let text = serde_json::to_string(&moved).expect("a seal writes");
            assert_eq!(
                serde_json::from_str::<MindSeal>(&text).ok(),
                Some(moved),
                "{field}"
            );
        }
        let wire = serde_json::to_value(&unit.seal).expect("a seal writes");
        assert_eq!(
            wire["source_sha256"],
            serde_json::json!(crate::minds::content::STANDARD_SHA256)
        );
        assert_eq!(
            wire["card_digest"].as_str().map(str::len),
            Some(64),
            "{wire}"
        );
        let mut capital = wire.clone();
        capital["source_sha256"] =
            serde_json::json!(crate::minds::content::STANDARD_SHA256.to_uppercase());
        assert!(serde_json::from_value::<MindSeal>(capital).is_err());
    }

    /// **17b cut M3.3** (D-UM-17, refuse-not-clamp): the Line card under a
    /// floor of 200 is refused by name, and so is a card whose survival pair
    /// does not hold.
    #[test]
    fn m3_a_card_under_the_floor_or_off_its_pair_is_refused_by_name() {
        let catalog = catalog();
        let floored = SealTerms {
            odds_floor_permille: 200,
            ..SealTerms::fixture()
        };
        let refused = checked_out(
            &catalog,
            crate::definitions::content::LINE_TANK,
            ObjId(4),
            Controller::Mind,
            &floored,
        )
        .err();
        assert_eq!(refused, Some(SealError::Unlawful { floor: 200 }));
        assert_eq!(
            refused.map(|error| error.to_string()).as_deref(),
            Some("the card asks for odds under the Compact floor of 20%")
        );
        let mut card = crate::arena::card::line(&ReflexConfig::default());
        card.stand_min_permille = card.withdraw_permille + 50;
        let unpaired = SealTerms {
            card,
            ..SealTerms::fixture()
        };
        let refused = checked_out(
            &catalog,
            crate::definitions::content::LINE_TANK,
            ObjId(4),
            Controller::Mind,
            &unpaired,
        )
        .err();
        assert!(
            matches!(
                refused,
                Some(SealError::Card(CardError::Pair { slider: "Line", .. }))
            ),
            "{refused:?}"
        );
    }

    /// **17b cut M3.3**: [`MindSeal::fixture`] IS [`seal_for`] under
    /// [`SealTerms::fixture`] — one computation — on a fill hull, a pilot's,
    /// a mind's and a hull with no core; and a hand-built literal sealed with
    /// `CustodyUnit::sealed` carries exactly that seal.
    ///
    /// 17b cut M4.5a: the fill hull is a mind's since 17b cut M4.3, so the
    /// four kinds are a pilot's line tank, a mind's line tank (the fill), a
    /// mind's heavy (a different core, the command core) and a mind's target
    /// (no core).
    #[test]
    fn m3_the_fixture_seal_is_the_one_computation_on_four_hull_kinds() {
        use crate::definitions::content::{HEAVY_MK1, LINE_TANK, TARGET_MK1};
        let catalog = catalog();
        let hulls = [
            (LINE_TANK, Controller::Pilot),
            (LINE_TANK, Controller::Mind),
            (HEAVY_MK1, Controller::Mind),
            (TARGET_MK1, Controller::Mind),
        ];
        for (entry, controller) in hulls {
            let unit = checked_out(&catalog, entry, ObjId(4), controller, &SealTerms::fixture())
                .expect("seals");
            assert_eq!(
                Ok(MindSeal::fixture(&unit)),
                seal_for(&unit, &SealTerms::fixture()),
                "{entry} {controller:?}"
            );
            assert_eq!(unit.seal, MindSeal::fixture(&unit));
            let hand = CustodyUnit {
                seal: MindSeal::placeholder(),
                ..unit.clone()
            }
            .sealed();
            assert_eq!(hand.seal, unit.seal, "{entry} {controller:?}");
            assert_eq!(hand.seal.sortie, SortieId::in_match(0, ObjId(4)));
        }
    }
}

#[cfg(test)]
mod m5_tests {
    use mush_core::ObjId;

    use super::*;
    use crate::arena::state::{Controller, Team};
    use crate::definitions::Catalog;

    /// **The presets a profile refusal lists are `PRESETS`** (17b cut M5.2's
    /// review): [`PRESET_CHOICES`] is written out as a sentence lists them,
    /// so it is pinned to the table it copies — a preset renamed or added
    /// there fails here, not in a refusal.
    #[test]
    fn m5_the_profile_refusal_lists_every_preset() {
        let (last, rest) = PRESETS.split_last().expect("presets");
        assert_eq!(PRESET_CHOICES, format!("{} or {last}", rest.join(", ")));
    }

    /// One shipped `entry` checked out as `id` under `terms` on a mind,
    /// through the production assembly ([`crate::match_::custody_unit`]).
    fn checked_out(entry: &str, id: ObjId, terms: &SealTerms) -> Result<CustodyUnit, SealError> {
        let catalog = Catalog::build_fielded(&[], "tacsim").expect("the fielded content");
        let key = mush_core::CanonicalId::new(entry).expect("a canonical id");
        let cells = crate::definitions::to_tactical_unit(
            catalog.specs.get(&key).expect("a shipped entry"),
            catalog.bindings.get(&key).expect("its binding"),
            id,
        )
        .expect("the entry instantiates");
        let posture =
            mush_sim::data::passive_rules_value(serde_json::json!({})).expect("an empty posture");
        crate::match_::custody_unit(
            &catalog,
            cells,
            id,
            Team::Red,
            mush_sim::data::Pose::at(1_500, 1_500, 0, posture),
            0,
            Controller::Mind,
            1,
            terms,
        )
    }

    /// Default seal identities, re-blessed at M5rules for API 2 and the stock
    /// source's API header. The Line card alone is pinned under
    /// [`SealTerms::fixture`] ([`MindSeal::fixture`]) and
    /// under [`SealTerms::production_fixture`] (what `crate::match_::start`
    /// seals an unchosen hull under), on an armed line tank, the heavy (the
    /// command core) and the coreless target. A default seal's JSON carries
    /// no `profiles` key. Future identity changes require a declared re-bless.
    #[test]
    fn m5rules_default_seal_identities_are_pinned() {
        use crate::definitions::content::{HEAVY_MK1, LINE_TANK, TARGET_MK1};
        let mut got = Vec::new();
        for (entry, id) in [(LINE_TANK, 4_u32), (HEAVY_MK1, 5), (TARGET_MK1, 9)] {
            for (which, terms) in [
                ("fixture", SealTerms::fixture()),
                ("production_fixture", SealTerms::production_fixture()),
                (
                    "production_principal",
                    SealTerms {
                        match_id: MatchId(7),
                        principal: Some(CharacterRef::Character { id: ObjId(77) }),
                        ..SealTerms::production_fixture()
                    },
                ),
            ] {
                let unit = checked_out(entry, ObjId(id), &terms).expect("seals");
                if which == "fixture" {
                    assert_eq!(unit.seal, MindSeal::fixture(&unit), "{entry}");
                }
                let text = serde_json::to_string(&unit.seal).expect("a seal writes");
                assert!(!text.contains("profiles"), "{entry} {which}: {text}");
                got.push(format!(
                    "{entry} {which} {}",
                    crate::arena::snapshot::digest_text(&unit.seal.digest())
                ));
            }
        }
        // M5r API/source identities; card bytes and seal shape are unchanged.
        let pinned = [
            "tacsim.unit.line_tank fixture \
             f43574cdf7d9db2d442579090ce24724ca0c5b89b220be2854e1b3b02ef2cb0e",
            "tacsim.unit.line_tank production_fixture \
             f43574cdf7d9db2d442579090ce24724ca0c5b89b220be2854e1b3b02ef2cb0e",
            "tacsim.unit.line_tank production_principal \
             c65378815d221daed6d5a92798442a5f167b20d84d3a0d5a088afcb3f2f83daf",
            "tacsim.unit.heavy_mk1 fixture \
             f4e5727194862e9d8d3bd8a7f8a7024705dbf805abfcdbacb5c437748ae238d4",
            "tacsim.unit.heavy_mk1 production_fixture \
             f4e5727194862e9d8d3bd8a7f8a7024705dbf805abfcdbacb5c437748ae238d4",
            "tacsim.unit.heavy_mk1 production_principal \
             3a98946b5cc79330f92203359d62786378cfeb6c842bd771396d1aa0be8b8645",
            "tacsim.unit.target_mk1 fixture \
             86a58d015d834ed5cb8400422050b2c70ed4993882c52904ab2d80c905736f03",
            "tacsim.unit.target_mk1 production_fixture \
             86a58d015d834ed5cb8400422050b2c70ed4993882c52904ab2d80c905736f03",
            "tacsim.unit.target_mk1 production_principal \
             0e8b6a9d941d96df94fe1c1ec33274f70c678958a26492de429e1cc9454f2eda",
        ];
        assert_eq!(got, pinned, "a default seal's digest moved: a STOP");
    }

    /// SHA-256 over a card's canonical bytes, as a seal digests it.
    fn card_digest(card: &StockCard) -> [u8; 32] {
        use sha2::{Digest as _, Sha256};
        Sha256::digest(serde_json::to_vec(card).expect("a card")).into()
    }

    /// A preset over the default doctrine.
    fn preset(name: &str) -> StockCard {
        crate::arena::card::preset(name, &ReflexConfig::default()).expect("a preset")
    }

    /// **17b cut M5.2: a sealed profile is named with its card's digest, and
    /// every card is validated** (doc 17 §11, §12; the owner's OQ-3; D3):
    /// Sniper then Line, typed in any case, seal canonically spelled, each
    /// with its card's digest, written LAST; the first's card is the seal's
    /// `card` (and `SealTerms::with_profiles` puts it on the terms) and the
    /// reference holds the codes `[2, 0]`. The Line card alone, named, writes
    /// no set — the default seal byte for byte. EVERY card is held to its
    /// pair, and on an armed hull to the floor: a second card off its pair is
    /// refused naming its preset, a second under the floor refuses the seal.
    /// A name that is no preset and a fifth profile are refused as cards
    /// (unreachable through the lobby). And the identity is the terms': a
    /// fixture mind's five fields sealed as given, its reference not
    /// doctrine, its rating and engine the terms' own.
    #[test]
    fn m5_a_sealed_profile_is_named_with_its_card_digest_and_every_card_is_validated() {
        use crate::arena::card::ProfileSet;
        use crate::arena::mind::SealedProfile;
        use crate::definitions::content::LINE_TANK;
        let unit = checked_out(LINE_TANK, ObjId(4), &SealTerms::fixture()).expect("seals");
        assert!(
            unit.seal.profiles.is_empty(),
            "a default seal writes no set"
        );
        let terms = SealTerms::fixture().with_profiles(vec![
            (SmolStr::new("sniper"), preset("Sniper")),
            (SmolStr::new("LINE"), preset("Line")),
        ]);
        assert_eq!(
            terms.card,
            preset("Sniper"),
            "the first's card on the terms"
        );
        let seal = seal_for(&unit, &terms).expect("seals");
        assert_eq!(
            seal.profiles,
            vec![
                SealedProfile {
                    name: SmolStr::new_static("Sniper"),
                    card_digest: card_digest(&preset("Sniper")),
                },
                SealedProfile {
                    name: SmolStr::new_static("Line"),
                    card_digest: card_digest(&preset("Line")),
                },
            ]
        );
        assert_eq!(
            seal.card,
            serde_json::to_vec(&preset("Sniper")).expect("a card")
        );
        assert_eq!(seal.card_digest, card_digest(&preset("Sniper")));
        assert_eq!(
            seal.seal_ref().profiles,
            ProfileSet {
                len: 2,
                codes: [2, 0, 0, 0],
            }
        );
        assert_ne!(seal.digest(), unit.seal.digest(), "the set is sealed");
        let text = serde_json::to_string(&seal).expect("a seal writes");
        let at = |key: &str| text.find(&format!("\"{key}\":")).expect(key);
        assert!(at("profiles") > at("engine"), "written last: {text}");
        assert_eq!(
            serde_json::from_str::<MindSeal>(&text).ok(),
            Some(seal.clone())
        );
        // The Line card alone, named: no set, the default seal.
        let line_alone =
            SealTerms::fixture().with_profiles(vec![(SmolStr::new("Line"), preset("Line"))]);
        assert_eq!(seal_for(&unit, &line_alone), Ok(unit.seal.clone()));
        assert_eq!(unit.seal.seal_ref().profiles, ProfileSet::LINE);

        // Every card validated: a second off its pair, named by its preset.
        let mut unpaired = preset("Rearguard");
        unpaired.stand_min_permille = unpaired.withdraw_permille + 50;
        let refused = seal_for(
            &unit,
            &SealTerms::fixture().with_profiles(vec![
                (SmolStr::new("Sniper"), preset("Sniper")),
                (SmolStr::new("Rearguard"), unpaired),
            ]),
        );
        assert!(
            matches!(
                refused,
                Err(SealError::Card(CardError::Pair {
                    slider: "Rearguard",
                    ..
                }))
            ),
            "{refused:?}"
        );
        // A second under the floor (Hunter's 30 under 40), on this armed hull;
        // Sniper's 65 alone is lawful there.
        let floored = |profiles: Vec<(SmolStr, StockCard)>| SealTerms {
            odds_floor_permille: 400,
            ..SealTerms::fixture().with_profiles(profiles)
        };
        assert_eq!(
            seal_for(
                &unit,
                &floored(vec![
                    (SmolStr::new("Sniper"), preset("Sniper")),
                    (SmolStr::new("Hunter"), preset("Hunter")),
                ])
            ),
            Err(SealError::Unlawful { floor: 400 })
        );
        assert!(
            seal_for(
                &unit,
                &floored(vec![(SmolStr::new("Sniper"), preset("Sniper"))])
            )
            .is_ok()
        );
        // No preset by that name; a fifth profile.
        assert_eq!(
            seal_for(
                &unit,
                &SealTerms::fixture()
                    .with_profiles(vec![(SmolStr::new("Brawler"), preset("Line"))])
            ),
            Err(SealError::Card(CardError::Choice {
                slider: "profile",
                choices: "Line, Sentry, Sniper, Skirmisher, Hunter or Rearguard",
            }))
        );
        let five = ["Line", "Sentry", "Sniper", "Hunter", "Rearguard"]
            .into_iter()
            .map(|name| (SmolStr::new(name), preset(name)))
            .collect();
        assert!(matches!(
            seal_for(&unit, &SealTerms::fixture().with_profiles(five)),
            Err(SealError::Card(CardError::Range {
                slider: "profiles",
                ..
            }))
        ));

        // The identity is the terms' — its rating and engine are not.
        let identity = SealIdentity {
            mind: SmolStr::new_static("Fixture I"),
            charter: SmolStr::new_static("Fixture"),
            line: SmolStr::new_static("fixture_i"),
            revision: 1,
            source_sha256: [9; 32],
        };
        let terms = SealTerms {
            identity: identity.clone(),
            engine_for_minds: SealEngine::Scripted,
            ..SealTerms::fixture()
        };
        let seal = seal_for(&unit, &terms).expect("seals");
        assert_eq!(
            SealIdentity {
                mind: seal.mind.clone(),
                charter: seal.charter.clone(),
                line: seal.line.clone(),
                revision: seal.revision,
                source_sha256: seal.source_sha256,
            },
            identity
        );
        assert!(!seal.seal_ref().doctrine, "not Standard Doctrine");
        assert_eq!(
            (seal.class, seal.engine),
            (CoreClass::I, SealEngine::Scripted)
        );
    }

    /// **17b cut M5.2: an over-class seal is refused by name** (doc 17 §9,
    /// D-UM-41; the design note's C-24; the owner's OQ-10 (d)): rated III on
    /// the line tank's governed Class II core is `SealError::OverClass`,
    /// whatever the cap, and `StartError::MindOverClass`'s two forms (built
    /// here; the mapping from the seal through `match_::start`, on the
    /// fixture mind `Load III`, is `tests/match_lifecycle.rs`'
    /// `m5_an_over_class_mind_is_refused_at_the_check_out_by_hull`) name the
    /// hull in the call-off's short form, never the mind; rated II there
    /// seals II. A coreless hull
    /// (`target_mk1`) rated III seals Class 0 on the ROM — never over-class,
    /// though its seal records it `governed`. **The S1 ruling** (C-24
    /// amended: "a governed core at Class 0 flies the ROM, as no core
    /// does"): the line tank with its governed core lowered to Class 0 seals
    /// Class 0 on that core under rating I (production's) and rating III
    /// alike, never `OverClass`. The heavy's Class III command
    /// core takes rated III (sealed III), capped at I sealing I.
    #[test]
    fn m5_an_over_class_seal_is_refused_by_name() {
        use crate::definitions::content::{HEAVY_MK1, LINE_TANK, TARGET_MK1};
        let rated = |class: CoreClass, cap: CoreClass| SealTerms {
            class_cap: cap,
            ..SealTerms::fixture().rated(class)
        };
        for cap in [CoreClass::III, CoreClass::I] {
            let refused = checked_out(LINE_TANK, ObjId(4), &rated(CoreClass::III, cap));
            assert_eq!(
                refused.as_ref().err(),
                Some(&SealError::OverClass {
                    rated: CoreClass::III,
                    core: CoreClass::II,
                }),
                "capped {cap:?}"
            );
            assert_eq!(
                refused.err().map(|error| error.to_string()).as_deref(),
                Some("the mind is rated Class III, over its governed core's Class II")
            );
        }
        let lawful = checked_out(LINE_TANK, ObjId(4), &rated(CoreClass::II, CoreClass::III))
            .expect("rated II on II seals");
        assert_eq!(lawful.seal.class, CoreClass::II);
        // The check-out's refusal: the hull named, the classes in the long
        // form only, the mind never.
        let tw = mush_testkit::TestWorld::new();
        let start = crate::match_::StartError::MindOverClass {
            unit: ObjId(4),
            callsign: "RA".to_owned(),
            rated: CoreClass::III,
            core: CoreClass::II,
        };
        assert_eq!(
            start.to_string(),
            "RA's mind is rated Class III, over its core's Class II; the match cannot start."
        );
        assert_eq!(
            start.call_off_words(tw.world(), &[]),
            "RA's mind is over its core's class"
        );
        // No core: never over-class.
        let target = checked_out(TARGET_MK1, ObjId(9), &rated(CoreClass::III, CoreClass::III))
            .expect("a coreless hull seals whatever its rating");
        assert_eq!(
            (target.seal.core, target.seal.class, target.seal.governed),
            (None, CoreClass::Zero, true)
        );
        assert_eq!(target.seal.fallback.as_deref(), Some(NO_CORE_FALLBACK));
        // A governed core at Class 0 (a `degrade_core`'s floor): never
        // over-class — it seals Class 0 on that core and flies the ROM, as no
        // core does (17b cut M5.2's S1 ruling), whatever the rating.
        let mut nought =
            checked_out(LINE_TANK, ObjId(4), &SealTerms::fixture()).expect("the line tank seals");
        let addr = mush_tactics::CellAddr::Component {
            component: mush_tactics::ComponentId::new("core").expect("a component"),
            cell: mush_tactics::CellId::new(crate::definitions::unit::CORE_CLASS_CELL)
                .expect("a cell"),
        };
        let plan = mush_tactics::UnitMutationPlan::new(
            vec![mush_tactics::UnitMutationOp::SetCell { addr, value: 0 }],
            None,
            None,
        )
        .expect("a plan");
        nought.unit = mush_tactics::apply(&nought.unit, &plan, 0)
            .expect("the plan applies")
            .0;
        assert_eq!(
            read_core(&nought.unit).map(|hw| (hw.class, hw.governed)),
            Some((CoreClass::Zero, true)),
            "a governed core at Class 0, still a core"
        );
        for rating in [CoreClass::I, CoreClass::III] {
            let seal = seal_for(&nought, &rated(rating, CoreClass::III))
                .expect("a governed Class 0 core is never over-class");
            assert_eq!(
                (
                    seal.core.as_deref(),
                    seal.core_class,
                    seal.class,
                    seal.governed
                ),
                (Some("core"), CoreClass::Zero, CoreClass::Zero, true),
                "rated {rating:?} on a governed Class 0 core seals Class 0"
            );
        }
        // The command core takes Class III.
        let heavy = checked_out(HEAVY_MK1, ObjId(5), &rated(CoreClass::III, CoreClass::III))
            .expect("the heavy seals rated III");
        assert_eq!(
            (heavy.seal.core_class, heavy.seal.class),
            (CoreClass::III, CoreClass::III)
        );
        let capped = checked_out(HEAVY_MK1, ObjId(5), &rated(CoreClass::III, CoreClass::I))
            .expect("the heavy seals rated III capped at I");
        assert_eq!(capped.seal.class, CoreClass::I);
    }

    /// **17b cut M5.2: an unarmed hull is never refused for the floor** (the
    /// design note's C-23; doc 17 §11's R-60: a fire section a hull cannot
    /// use is shown inert and never refuses the seal). Under an operator
    /// floor of 20%, the armed line tank's Line card (10%) is refused as it
    /// always was; the same tank with its weapon rows cleared, sealed again,
    /// is sealed — it was refused before this cut — and so is the unarmed
    /// target under a floor of 90%. The pair still runs on every hull: an
    /// unarmed hull's card off its pair is refused.
    #[test]
    fn m5_an_unarmed_hull_is_never_refused_for_the_floor() {
        use crate::definitions::content::{LINE_TANK, TARGET_MK1};
        let floored = |floor: u32| SealTerms {
            odds_floor_permille: floor,
            ..SealTerms::fixture()
        };
        assert_eq!(
            checked_out(LINE_TANK, ObjId(4), &floored(200)).err(),
            Some(SealError::Unlawful { floor: 200 }),
            "armed: refused, as before"
        );
        let mut unarmed = checked_out(LINE_TANK, ObjId(4), &SealTerms::fixture()).expect("seals");
        unarmed.weapons.clear();
        unarmed
            .reseal(&floored(200))
            .expect("unarmed: the floor is inert");
        assert_eq!(unarmed.seal.channels, vec![Channel::Helm, Channel::Sensors]);
        assert_eq!(unarmed.seal.odds_floor_permille, 200);
        let target = checked_out(TARGET_MK1, ObjId(9), &floored(900));
        assert!(target.is_ok(), "{target:?}");
        let mut card = preset("Line");
        card.stand_min_permille = card.withdraw_permille + 50;
        assert!(matches!(
            unarmed.reseal(&SealTerms {
                card,
                ..floored(200)
            }),
            Err(SealError::Card(CardError::Pair { slider: "Line", .. }))
        ));
    }
}

#[cfg(test)]
mod m4_tests {
    use mush_core::ObjId;

    use super::*;
    use crate::arena::state::{Controller, Team};
    use crate::arena::test_support::{line_tank, pose, scout, tactical_unit};

    /// **17b cut M4.1: the seal's class is `min(rated, core, cap)`** (the M4
    /// row's finding (9); doc 17 §9, D-UM-35, D-UM-37): on a Class II line
    /// core, rated I and uncapped seals I (the stock terms); rated I capped
    /// at Zero seals Zero; rated II capped at II seals II; a hull with no
    /// core seals Zero whatever the terms say, its `fallback` named — rated
    /// III included, never over-class (C-24). The fixture terms rate at
    /// [`STOCK_RATED_CLASS`] and cap at III.
    ///
    /// **Re-pinned at 17b cut M5.2**: over-class is refused at the seal since
    /// 17b cut M5.2 (doc 17 §9, D-UM-41) — the rows rated III on the governed
    /// Class II core, `(III, III)` (sealed II, "the core caps it") and
    /// `(III, I)` (sealed I), were lawful here and are
    /// `SealError::OverClass { rated: III, core: II }` now, whatever the cap.
    #[test]
    fn m4_the_seal_s_class_is_the_least_of_the_rating_the_core_and_the_cap() {
        let fixture = SealTerms::fixture();
        assert_eq!(fixture.rated, STOCK_RATED_CLASS);
        assert_eq!(fixture.class_cap, CoreClass::III);
        let mut red = line_tank(ObjId(4), Team::Red, pose(1_500, 4_500));
        red.controller = Controller::Mind;
        assert_eq!(
            read_core(&red.unit).map(|core| core.class),
            Some(CoreClass::II),
            "the line tank carries the line core"
        );
        for (rated, cap, class) in [
            (CoreClass::I, CoreClass::III, CoreClass::I),
            (CoreClass::I, CoreClass::Zero, CoreClass::Zero),
            (CoreClass::II, CoreClass::II, CoreClass::II),
        ] {
            let terms = SealTerms {
                class_cap: cap,
                ..SealTerms::fixture().rated(rated)
            };
            let seal = seal_for(&red, &terms).expect("the terms seal");
            assert_eq!(seal.class, class, "rated {rated:?} capped {cap:?}");
            assert_eq!(seal.core_class, CoreClass::II, "the core as fitted");
        }
        // 17b cut M5.2: rated III over the governed Class II core is refused,
        // whatever the cap — the two rows that sealed II and I before it.
        for cap in [CoreClass::III, CoreClass::I] {
            let terms = SealTerms {
                class_cap: cap,
                ..SealTerms::fixture().rated(CoreClass::III)
            };
            assert_eq!(
                seal_for(&red, &terms),
                Err(SealError::OverClass {
                    rated: CoreClass::III,
                    core: CoreClass::II,
                }),
                "rated III capped {cap:?}"
            );
        }
        let bare = crate::arena::state::CustodyUnit {
            unit: tactical_unit(ObjId(9)),
            controller: Controller::Mind,
            ..scout(ObjId(9), Team::Blue, pose(6_500, 4_500))
        };
        let seal = seal_for(&bare, &SealTerms::fixture().rated(CoreClass::III)).expect("seals");
        assert_eq!(seal.class, CoreClass::Zero, "no core: the ROM");
        assert_eq!(seal.fallback.as_deref(), Some(NO_CORE_FALLBACK));
    }

    /// **17b cut M4.2**: the fill's engine is one a mind can be sealed on —
    /// `native` or `scripted`, never `reflex`, which is kept decode-only for
    /// the M3 records (the M4 row's finding (6)). The fill decision
    /// (2026-10-09) took `native`.
    #[test]
    fn m4_2_the_production_fill_engine_is_native_or_scripted() {
        assert!(matches!(
            PRODUCTION_FILL_ENGINE,
            SealEngine::Native | SealEngine::Scripted
        ));
        assert_ne!(PRODUCTION_FILL_ENGINE, SealEngine::Reflex);
    }
}
