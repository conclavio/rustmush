//! **The holdings seam** (17b cut M5.1; doc 17 §15, §19; D-UM-49; Q-UM-13 at
//! its default, confirmed by the owner's OQ-2 of 2026-10-10).
//!
//! Which minds a character may choose at the lobby — `SIM MIND` for the slot
//! it claimed, `SIM AI MIND` for the fill of the side it leads — is a
//! question about the character's **holdings**: in doc 17 the character's IC
//! library and Standard Doctrine (D-UM-49). Nothing IC exists before 17b cut
//! M6 (no library, no drafts, no licence), so the production answer is the
//! one mind the server ships: **every character holds Standard Doctrine and
//! its six presets**, and nothing else ([`DoctrineOnly`]).
//!
//! The answer is a seam all the same — [`Holdings`], held on the tacsim
//! module's own state slot as [`HoldingsSeam`] and installed with
//! [`DoctrineOnly`] by `tacsim::register` — because every path the lobby
//! must refuse or carry for a mind that is NOT Standard Doctrine (a mind the
//! character does not hold, a holding lost before the check-out, a scripted
//! mind's source, the over-class refusal) has to be exercised now and can
//! only be reached through a second provider. A test rig installs one with
//! [`install_holdings`]: a production seam with a test provider behind it,
//! `crate::minds::install_sealed_runners`' precedent — never a fixture
//! impersonating a production module (17b §0). Production code names no
//! fixture.
//!
//! **Keyed by the character who boards** (`crate::match_::Slot::pilot`; for
//! the fill, the side lead's own slot's pilot), never by the account: an alt
//! is the same player but not the same character, and a character's
//! holdings are the character's.
//!
//! What this module does NOT do in 17b cut M5.1: seal anything. The lobby
//! records a choice and the check-out resolves it against the seam
//! (`SimError::MindNotHeld` when a holding is gone) into
//! `crate::match_::Side::minds`; the check-out still seals Standard Doctrine
//! on Line until 17b cut M5.2 carries the choice into the seal.

use std::sync::Arc;

use mush_core::{ObjId, World};
use smol_str::SmolStr;

use crate::arena::core::CoreClass;
use crate::arena::mind::SealEngine;
pub use crate::match_::seal::SealIdentity;

/// **One mind a character holds** (17b cut M5.1): what a seal would name it
/// by, the class it is rated at, the engine it runs on, the presets a
/// principal may seal it with, and — for a scripted mind a provider carries
/// — its source text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeldMind {
    /// The five identity fields a seal writes ([`SealIdentity`]).
    pub identity: SealIdentity,
    /// The class it is rated at (D-UM-35): Standard Doctrine's
    /// [`crate::arena::mind::STOCK_RATED_CLASS`] (I).
    pub rated: CoreClass,
    /// The engine its seal names: [`crate::match_::PRODUCTION_FILL_ENGINE`]
    /// for Standard Doctrine, [`SealEngine::Scripted`] for a provider's
    /// scripted mind.
    pub engine: SealEngine,
    /// The profiles a principal may seal it with, in the order they are
    /// listed — Standard Doctrine's six presets (`crate::arena::card::PRESETS`,
    /// the owner's OQ-3). Line first.
    pub presets: Vec<SmolStr>,
    /// **Its source text**: `None` for Standard Doctrine, whose text is
    /// compiled in and stored at every boot (`store::put_mind_source`), `Some`
    /// for a scripted mind a provider carries ([`install_holdings`] installs
    /// it into the store's RAM map).
    pub source: Option<Arc<str>>,
}

impl HeldMind {
    /// **Standard Doctrine, as every character holds it** —
    /// [`SealIdentity::doctrine`], rated [`crate::arena::mind::STOCK_RATED_CLASS`],
    /// on [`crate::match_::PRODUCTION_FILL_ENGINE`], with the six presets and
    /// no source of its own.
    #[must_use]
    pub fn doctrine() -> HeldMind {
        HeldMind {
            identity: SealIdentity::doctrine(),
            rated: crate::arena::mind::STOCK_RATED_CLASS,
            engine: crate::match_::seal::PRODUCTION_FILL_ENGINE,
            presets: crate::arena::card::PRESETS
                .iter()
                .map(|name| SmolStr::new_static(name))
                .collect(),
            source: None,
        }
    }

    /// The name every line spells it with (`Standard Doctrine`).
    #[must_use]
    pub fn name(&self) -> &str {
        self.identity.mind.as_str()
    }

    /// **Whether it is Standard Doctrine**, by identity
    /// ([`SealIdentity::is_doctrine`]).
    #[must_use]
    pub fn is_doctrine(&self) -> bool {
        self.identity.is_doctrine()
    }

    /// **`word` as one of its presets**, case-folded, in its canonical
    /// spelling (`sniper` is `Sniper`), or `None`.
    #[must_use]
    pub fn preset(&self, word: &str) -> Option<SmolStr> {
        let word = word.trim();
        self.presets
            .iter()
            .find(|preset| preset.eq_ignore_ascii_case(word))
            .cloned()
    }
}

/// **Whether `word` names `held`** (17b cut M5.1), case-folded: its own name,
/// and for Standard Doctrine also `standard` and `doctrine` (the first hint
/// types `SIM MIND Standard`).
#[must_use]
pub fn names(held: &HeldMind, word: &str) -> bool {
    let word = word.trim();
    if word.eq_ignore_ascii_case(held.name()) {
        return true;
    }
    held.is_doctrine()
        && (word.eq_ignore_ascii_case("standard") || word.eq_ignore_ascii_case("doctrine"))
}

/// **The mind `word` names among `held`**, case-folded ([`names`]), or `None`.
#[must_use]
pub fn resolve<'a>(held: &'a [HeldMind], word: &str) -> Option<&'a HeldMind> {
    held.iter().find(|mind| names(mind, word))
}

/// **A character's holdings** (17b cut M5.1; D-UM-49): the minds `character`
/// may choose, Standard Doctrine among them in production.
///
/// `Send + Sync`: it is held on a module state slot behind an [`Arc`].
pub trait Holdings: Send + Sync {
    /// Every mind `character` holds, in the order a list prints them.
    fn held(&self, w: &World, character: ObjId) -> Vec<HeldMind>;

    /// **Every scripted source this provider can hand out**, whichever
    /// character it answers for — what [`install_holdings`] installs into the
    /// store's RAM map so a sealed source is found by its digest. Empty for a
    /// provider that holds no scripted mind, which is [`DoctrineOnly`].
    fn sources(&self) -> Vec<Arc<str>> {
        Vec::new()
    }
}

/// **The production provider** (17b cut M5.1; OQ-2): every character holds
/// exactly Standard Doctrine and its six presets ([`HeldMind::doctrine`]),
/// and nothing else — no library, draft or licence exists before 17b cut M6.
#[derive(Clone, Copy, Debug, Default)]
pub struct DoctrineOnly;

impl Holdings for DoctrineOnly {
    fn held(&self, _w: &World, _character: ObjId) -> Vec<HeldMind> {
        vec![HeldMind::doctrine()]
    }
}

/// **The seam's state slot** (17b cut M5.1): the provider the lobby asks,
/// [`DoctrineOnly`] as `tacsim::register` installs it. Its provider is
/// private (17b cut M5.1's review): [`HoldingsSeam::production`] and
/// [`install_holdings`] are the only ways to set it, so a provider's sources
/// always reach the store's RAM map, and [`held_by`] is the only way to ask
/// it.
#[derive(Clone)]
pub struct HoldingsSeam(Arc<dyn Holdings>);

impl HoldingsSeam {
    /// The production seam: [`DoctrineOnly`].
    #[must_use]
    pub fn production() -> HoldingsSeam {
        HoldingsSeam(Arc::new(DoctrineOnly))
    }
}

impl std::fmt::Debug for HoldingsSeam {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HoldingsSeam(..)")
    }
}

/// **Install a provider in place of the production one** (17b cut M5.1) —
/// the test rigs' one route to a character holding a mind that is not
/// Standard Doctrine. Each of the provider's [`Holdings::sources`] goes into
/// the store's RAM map through `store::install_mind_source` (the RAM half of
/// `store::put_mind_source`, which needs a `Ctx` and queues a document no rig
/// can commit), so `crate::minds::source_lookup` finds a fixture's source by
/// its digest exactly as it finds a stored one. A world without the tacsim
/// module composed has no slot, and nothing happens.
pub fn install_holdings(w: &mut World, holdings: Arc<dyn Holdings>) {
    if let Some(store) = w.module_state_mut::<crate::TacsimStore>() {
        for source in holdings.sources() {
            // Already held (the same text twice, or a stored one): kept as it
            // is, which is the immutability rule.
            let _ = crate::store::install_mind_source(store, &source);
        }
    }
    if let Some(seam) = w.module_state_mut::<HoldingsSeam>() {
        *seam = HoldingsSeam(holdings);
    }
}

/// **What `character` holds, as the seam answers** — [`DoctrineOnly`]'s
/// answer on a world with no seam installed.
#[must_use]
pub fn held_by(w: &World, character: ObjId) -> Vec<HeldMind> {
    match w.module_state::<HoldingsSeam>() {
        Some(seam) => seam.0.held(w, character),
        None => DoctrineOnly.held(w, character),
    }
}

/// **One hull's choice, resolved at the check-out** (17b cut M5.1; the
/// design note's M5.1 item 6): the held mind, the profiles in the order they
/// were chosen (empty is Line alone), whether the slot or its side's fill
/// chose at all, and whether Q-UM-15 substituted Standard Doctrine for it
/// (always `false` until 17b cut M5.2 builds the substitution).
///
/// `crate::match_::Side::minds` carries one per fielded hull — an unchosen
/// hull's is [`SealChoice::unchosen`] — and 17b cut M5.2's seal reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SealChoice {
    /// The mind the hull will be sealed with.
    pub held: HeldMind,
    /// The profiles, canonical spellings, the first the one that flies at the
    /// release; empty is Line alone.
    pub profiles: Vec<SmolStr>,
    /// Whether the slot (or the side's fill) chose a mind at all.
    pub chose: bool,
    /// Whether Q-UM-15 substituted Standard Doctrine for the choice (17b cut
    /// M5.2; `false` here).
    pub substituted: bool,
}

impl SealChoice {
    /// **The default every unchosen hull is sealed with**: Standard Doctrine
    /// on Line, chosen by nobody (the owner's OQ-9 (i): an unchosen slot
    /// stays as built).
    #[must_use]
    pub fn unchosen() -> SealChoice {
        SealChoice {
            held: HeldMind::doctrine(),
            profiles: Vec::new(),
            chose: false,
            substituted: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **17b cut M5.1**: Standard Doctrine as every character holds it — the
    /// five identity fields `seal_for` writes, rated I, on the production
    /// engine, the six presets in doc 17's order, no source; named by its
    /// name, `standard` and `doctrine`, case-folded, and by nothing else.
    #[test]
    fn m5_doctrine_is_held_as_the_seal_names_it() {
        let held = HeldMind::doctrine();
        let identity = &held.identity;
        assert_eq!(identity.mind, crate::match_::SEALED_MIND);
        assert_eq!(identity.charter, crate::match_::SEALED_CHARTER);
        assert_eq!(identity.line, crate::match_::SEALED_LINE);
        assert_eq!(identity.revision, crate::minds::STOCK_VERSION);
        assert_eq!(
            identity.source_sha256,
            crate::minds::content::standard_digest().0
        );
        assert!(held.is_doctrine());
        assert_eq!(held.rated, CoreClass::I);
        assert_eq!(held.engine, crate::match_::PRODUCTION_FILL_ENGINE);
        assert_eq!(
            held.presets,
            [
                "Line",
                "Sentry",
                "Sniper",
                "Skirmisher",
                "Hunter",
                "Rearguard"
            ]
        );
        assert_eq!(held.source, None);
        for word in [
            "Standard Doctrine",
            "standard doctrine",
            "STANDARD",
            "doctrine",
        ] {
            assert!(names(&held, word), "{word}");
        }
        for word in ["Sentinel", "Standard Doc", "", "line"] {
            assert!(!names(&held, word), "{word}");
        }
        assert_eq!(held.preset("sNiPeR"), Some(SmolStr::new_static("Sniper")));
        assert_eq!(held.preset("Brawler"), None);
        let tw = mush_testkit::TestWorld::new();
        assert_eq!(DoctrineOnly.held(tw.world(), ObjId(7)), vec![held.clone()]);
        assert_eq!(held_by(tw.world(), ObjId(7)), vec![held]);
        assert!(DoctrineOnly.sources().is_empty());
        assert_eq!(SealChoice::unchosen().held, HeldMind::doctrine());
    }
}
