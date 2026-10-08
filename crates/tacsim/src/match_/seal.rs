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
//!    17b cut M5; D-UM-49).
//! 3. **The core**: its component, class, governed bit and condition, read
//!    live off the unit (`crate::arena::core::read_core`); a hull with none is
//!    Class 0, governed, and flies the ROM from the start, which its
//!    `fallback` says.
//! 4. **The revision**: Standard Doctrine, the registry's stock line, at the
//!    stock version, its source named by its digest alone.
//! 5. **The card**, validated against the schema and the Compact floor the
//!    terms carry — refused, never clamped (D-UM-17): a card whose pair of
//!    survival sliders does not hold is [`SealError::Card`], one whose
//!    `min_odds` is under the floor [`SealError::Unlawful`].
//!
//! The class the mind thinks at is `min(rated, core_class, class_cap)` under
//! the terms' [`SealTerms::rated`] (Standard Doctrine's `STOCK_RATED_CLASS`)
//! and [`SealTerms::class_cap`] (17b cut M3's ruling, the plan's F4; 17b cut
//! M4.1 put the rating and the cap on the terms and the think side reads the
//! seal, `Arena::mind_class`); the engine is the reflex's for a reflex
//! hull AND a piloted one (what a release hands a piloted hull to today; 17b
//! cut M4 moves the fill), the native stock mind's for a mind's hull.

use crate::arena::card::{CardError, StockCard};
use crate::arena::conditions::{ASPECTS_ANY, FireCondition, SlotTarget, condition_is_lawful};
use crate::arena::core::{CoreClass, read_core};
use crate::arena::mind::{MindSeal, STOCK_RATED_CLASS, SealEngine, SortieId};
use crate::arena::state::{Channel, CustodyUnit};
use crate::arena_task::MatchId;
use crate::config::ReflexConfig;
use crate::deliberation::order::CharacterRef;
use smol_str::SmolStr;

/// **The stock mind's name on a seal** (doc 17 §11).
pub const SEALED_MIND: &str = "Standard Doctrine";

/// **The charter the stock mind is filed under** (doc 17 §16).
pub const SEALED_CHARTER: &str = "registry";

/// **The stock mind's line of revisions**.
pub const SEALED_LINE: &str = "standard";

/// **The licence a sim seal records** (D-UM-49: the OOC sim reads no IC
/// holding before 17b cut M5).
pub const SEALED_LICENCE: &str = "doctrine";

/// **Why a hull flies the ROM from its first think**: it has no core
/// (D-UM-33).
pub const NO_CORE_FALLBACK: &str = "no core: the ROM";

/// **What a seal is taken under, beside the custody** (17b cut M3.3): the
/// match it mints the sortie in, the warrant principal, the card and the
/// Compact floor the card is validated against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SealTerms {
    /// The match the sortie is minted in ([`SortieId::in_match`]).
    pub match_id: MatchId,
    /// The warrant principal: the hull's first seated pilot, else the side
    /// lead's character, else `None` (a staff match; a side with no lead).
    pub principal: Option<CharacterRef>,
    /// The card: the Line preset over the match's doctrine until 17b cut M5
    /// lets a principal choose one.
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
    /// [`STOCK_RATED_CLASS`] (I) everywhere in production before 17b cut M5,
    /// Class III for a fixture's load mind. The seal's `class` is
    /// `min(rated, core, class_cap)`, and the think side reads it off the
    /// seal (`Arena::mind_class`) — the arena-wide `rated_class` this
    /// replaces is gone.
    pub rated: CoreClass,
    /// **The class the hull is capped at** (17b cut M4.1; doc 17 §9, D-UM-37:
    /// the lead's or the operator's cap): `[tacsim.minds].fill_class` on
    /// every production seal from 17b cut M4.3 (Class I stock, so it caps
    /// nothing until 17b cut M5 rates minds); Class III — no cap — under the
    /// fixture terms.
    pub class_cap: CoreClass,
}

/// **The engine the AI fill is sealed on at 17b cut M4.3's flip** (the M4
/// row's (ii) and (iii); doc 17 §22's M4): [`SealEngine::Scripted`] — Standard
/// Doctrine interpreted — is the candidate under test; [`SealEngine::Native`]
/// is the fast path doc 17 §18 keeps in reserve. Nothing in production reads
/// it before M4.3 (`seal_for` writes a mind's engine from the terms); the
/// minds lane's fill variant (`tacsim_harness_arena_minds` in
/// `crates/tacsim/tests/harness.rs`) seals its thirty-six Class I hulls a
/// side on it, and THE DECISION IS THAT VARIANT'S VERDICT over three
/// idle-host weekly runs (`scripts/harness-weekly.sh` on the blessing host):
/// `held=yes` three times keeps `Scripted`; any `held=no` moves this to
/// `Native`, in M4.3's commit, with the three
/// `harness_minds_verdict: variant=fill_authored` lines in its body and in
/// 17b §2. A run whose line reads `sound=no` is a lane fault (a failed
/// batch, a refused snapshot, a cold field), re-run and never counted.
///
/// DECISION LOG (the committer fills these in before the flip; each line is
/// the run's `harness_minds_verdict` line's `held=` word with the date, the
/// HEAD and the host from the report's `verdict` object):
///   run 1: pending
///   run 2: pending
///   run 3: pending
pub const PRODUCTION_FILL_ENGINE: SealEngine = SealEngine::Scripted;

impl SealTerms {
    /// **The terms every fixture seals under**: match 0, no principal, the
    /// Line preset over the default doctrine, the seed floor of 100, a
    /// mind's hull sealed native, rated at the stock class and uncapped.
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
    /// shape (`tests/support`'s `Muster::fill`). Under a native fill this is
    /// [`SealTerms::fixture`] byte for byte (`min(I, core, I)` is `min(I,
    /// core, III)` on every shipped core); under a scripted one it is what
    /// makes a pin the production shape.
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
    /// The card breaks its own schema (its pair of survival sliders).
    #[error("{0}")]
    Card(CardError),
    /// The card's `min_odds` is under the Compact floor.
    #[error("the card asks for odds under the Compact floor of {}%", .floor / 10)]
    Unlawful {
        /// The floor, per mille.
        floor: u32,
    },
}

/// **The one seal computation** (17b cut M3.3; this module's head): `unit`'s
/// seal under `terms`, read off the FINISHED custody — its cells, id,
/// controller and weapon rows.
///
/// # Errors
///
/// [`SealError::Card`] when the card's pair does not hold, and
/// [`SealError::Unlawful`] when its `min_odds` is under the floor.
pub fn seal_for(unit: &CustodyUnit, terms: &SealTerms) -> Result<MindSeal, SealError> {
    // (1) The stations: helm and sensors on every hull, fire on an armed one.
    let mut channels = vec![Channel::Helm, Channel::Sensors];
    if !unit.weapons.is_empty() {
        channels.push(Channel::Fire);
    }
    // (2) The licence is `doctrine` (below); (3) the core, read live.
    let (core, core_class, governed, fallback) = match read_core(&unit.unit) {
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
    // (5) The card, against its schema and the floor, refuse-not-clamp.
    terms.card.pair("Line").map_err(SealError::Card)?;
    let probe = FireCondition {
        weapon: SmolStr::default(),
        target: SlotTarget::BestHostile,
        min_permille: terms.card.min_odds_permille,
        aspects: ASPECTS_ANY,
        min_rounds_left: 0,
        min_range_m: 0,
        max_range_m: u32::MAX,
    };
    if condition_is_lawful(&probe, terms.odds_floor_permille).is_err() {
        return Err(SealError::Unlawful {
            floor: terms.odds_floor_permille,
        });
    }
    // `StockCard` is strings, numbers and booleans: it always serialises.
    let card = serde_json::to_vec(&terms.card).unwrap_or_default();
    let card_digest = {
        use sha2::{Digest as _, Sha256};
        Sha256::digest(&card).into()
    };
    Ok(MindSeal {
        unit: unit.id,
        sortie: SortieId::in_match(terms.match_id.0, unit.id),
        core,
        core_class,
        // 17b cut M4.1: `min(rated, core, cap)` — the terms' rating and cap,
        // where it was `min(STOCK_RATED_CLASS, core)`.
        class: terms.rated.min(core_class).min(terms.class_cap),
        governed,
        // (4) The revision.
        mind: SmolStr::new_static(SEALED_MIND),
        charter: SmolStr::new_static(SEALED_CHARTER),
        line: SmolStr::new_static(SEALED_LINE),
        revision: crate::minds::STOCK_VERSION,
        // The digest of the script's own bytes, which `tests/content.rs` pins
        // equal to `STANDARD_SHA256` — the key the source store holds it under.
        source_sha256: crate::minds::content::standard_digest().0,
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
        // it runs on; a fixture's reflex hull likewise (it is sealed, never
        // run on a mind). `SealEngine::Reflex` is decode-only from here: every
        // M3 record in the centre's history carries it (the M4 row's finding
        // (6)), and nothing writes it.
        engine: terms.engine_for_minds,
    })
}

impl MindSeal {
    /// **A fixture's seal**: [`seal_for`] under [`SealTerms::fixture`] — the
    /// one computation, never a second one. The fixture terms are lawful by
    /// construction (the Line preset at the seed floor), so it never refuses.
    ///
    /// # Panics
    ///
    /// Never on any unit: the fixture card pairs and sits on the floor.
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
                    abi: SmolStr::new_static("rhai=1.26.0;api=1;stock=1"),
                    licence: SmolStr::new_static("doctrine"),
                    principal: Some(CharacterRef::Character { id: ObjId(77) }),
                    channels: vec![Channel::Helm, Channel::Sensors, Channel::Fire],
                    odds_floor_permille: 100,
                    fallback: None,
                    engine,
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
        ];
        assert_eq!(edits.len(), 21, "every field of the seal");
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
mod m4_tests {
    use mush_core::ObjId;

    use super::*;
    use crate::arena::state::{Controller, Team};
    use crate::arena::test_support::{line_tank, pose, scout, tactical_unit};

    /// **17b cut M4.1: the seal's class is `min(rated, core, cap)`** (the M4
    /// row's finding (9); doc 17 §9, D-UM-35, D-UM-37): on a Class II line
    /// core, rated I and uncapped seals I (the stock terms); rated III seals
    /// II (the core caps it); rated III capped at I seals I; rated I capped
    /// at Zero seals Zero; a hull with no core seals Zero whatever the terms
    /// say, its `fallback` named. The fixture terms rate at
    /// [`STOCK_RATED_CLASS`] and cap at III.
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
            (CoreClass::III, CoreClass::III, CoreClass::II),
            (CoreClass::III, CoreClass::I, CoreClass::I),
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
        let bare = crate::arena::state::CustodyUnit {
            unit: tactical_unit(ObjId(9)),
            controller: Controller::Mind,
            ..scout(ObjId(9), Team::Blue, pose(6_500, 4_500))
        };
        let seal = seal_for(&bare, &SealTerms::fixture().rated(CoreClass::III)).expect("seals");
        assert_eq!(seal.class, CoreClass::Zero, "no core: the ROM");
        assert_eq!(seal.fallback.as_deref(), Some(NO_CORE_FALLBACK));
    }

    /// **17b cut M4.2**: the fill's candidate engine is one a mind can be
    /// sealed on — `native` or `scripted`, never `reflex`, which is kept
    /// decode-only for the M3 records (the M4 row's finding (6)).
    #[test]
    fn m4_2_the_production_fill_engine_is_native_or_scripted() {
        assert!(matches!(
            PRODUCTION_FILL_ENGINE,
            SealEngine::Native | SealEngine::Scripted
        ));
        assert_ne!(PRODUCTION_FILL_ENGINE, SealEngine::Reflex);
    }
}
