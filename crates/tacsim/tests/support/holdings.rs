//! **The holdings seam's test provider** (17b cut M5.1; doc 17 §15, D-UM-49;
//! the owner's OQ-2: production holdings are Standard Doctrine for every
//! character, behind a seam whose test implementation holds scripted fixture
//! minds).
//!
//! Installed through the production seam,
//! [`tacsim::match_::holdings::install_holdings`] — never a fixture
//! impersonating a production module (17b §0): production code names none of
//! this. Every character holds three minds here:
//!
//! - **Standard Doctrine** ([`HeldMind::doctrine`]), as every character does
//!   in production;
//! - **`Fixture I`** — `fixture_i.rhai`, rated Class I, scripted, presets
//!   Line and Sniper, with explicit draft fixture provenance — the
//!   fixture of every non-Doctrine lobby path (the not-held, fit and pool
//!   cases, the lost holding). Every test fielding it fields a `line_tank` at
//!   the default `fill_class` I. [`TestHoldings::drop_fixture_i`] takes it
//!   away from everybody, for the check-out's lost-holding refusal;
//! - **`Load III`** — the existing `load_iii.rhai` (its digest pinned at
//!   `tests/harness.rs` too; never copied), rated Class III — used ONLY for
//!   the over-class refusal at the choice.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use mush_core::{ObjId, World};
use smol_str::SmolStr;
use tacsim::arena::core::CoreClass;
use tacsim::arena::mind::SealEngine;
use tacsim::arena::profile_seal::MindOrigin;
use tacsim::match_::holdings::{HeldMind, Holdings, SealIdentity};

/// `Fixture I`'s source.
pub const FIXTURE_I_RHAI: &str = include_str!("fixture_i.rhai");

/// `Fixture I`'s digest, pinned.
pub const FIXTURE_I_SHA256_HEX: &str =
    "e2e7ac4d5402672883fd7fdfe73936dbff21b22c528644831ae5e666b52b1b28";

/// `Fixture I`'s name.
pub const FIXTURE_I: &str = "Fixture I";

/// `Load III`'s source — the harness's own file, not a copy.
pub const LOAD_III_RHAI: &str = include_str!("load_iii.rhai");

/// `Load III`'s digest, as `tests/harness.rs` pins it — a second copy of that
/// pin (the harness does not include `support`), and both are asserted, so a
/// change to `load_iii.rhai` moves both (17b cut M5.1's review).
pub const LOAD_III_SHA256_HEX: &str =
    "e61a8153963f0f4d17c0eb94c3dec465ac4ad8e204e136c797ec4f1f566864f3";

/// `Load III`'s name.
pub const LOAD_III: &str = "Load III";

/// The SHA-256 of `text`'s bytes.
#[must_use]
pub fn digest(text: &str) -> [u8; 32] {
    use sha2::{Digest as _, Sha256};
    Sha256::digest(text.as_bytes()).into()
}

/// The SHA-256 of `text`'s bytes, as the tree writes a digest.
#[must_use]
pub fn digest_hex(text: &str) -> String {
    digest(text)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Structural metadata for the pre-workshop fixed-stock fixture adapter. These
/// sources intentionally predate MIND metadata; they are not M6 save acceptance
/// fixtures. Roll still owns real immutable revisions and all retention checks.
fn fixture_envelope(class: CoreClass) -> mush_roll::Envelope {
    mush_roll::Envelope {
        schema: serde_json::json!({"sections": []}),
        presets: serde_json::json!({}),
        profiles: serde_json::json!({"Default": {}}),
        rules: serde_json::json!({}),
        declared_class: class.as_str().into(),
        mind_abi: tacsim::minds::MIND_ABI.into(),
    }
}

pub fn fixture_origin(id: u64, class: CoreClass, source: &str) -> MindOrigin {
    MindOrigin::Draft {
        character: ObjId(1),
        mind: id,
        revision: 1,
        digest: fixture_envelope(class)
            .digest(source)
            .expect("exact fixture envelope"),
    }
}

struct FixtureSource;
impl mush_roll::RevisionAdapter for FixtureSource {
    fn normalize_source(&self, source: &str) -> Result<String, mush_roll::RollError> {
        Ok(source.to_owned())
    }
    fn validate(
        &self,
        source: &str,
        envelope: &mush_roll::Envelope,
    ) -> Result<(), mush_roll::RollError> {
        let class =
            CoreClass::parse(&envelope.declared_class).ok_or(mush_roll::RollError::Missing)?;
        if ![FIXTURE_I_RHAI, LOAD_III_RHAI].contains(&source)
            || *envelope != fixture_envelope(class)
        {
            return Err(mush_roll::RollError::Missing);
        }
        tacsim::minds::RhaiRunner::new(
            &tacsim::config::ArenaConfig::default(),
            source,
            mush_mind::Digest(digest(source)),
        )
        .map(|_| ())
        .map_err(|error| mush_roll::RollError::Invalid {
            field: "fixture source".into(),
            message: error.to_string(),
        })
    }
}
impl mush_roll::SourceAdapter for FixtureSource {
    type Token = ();
    fn prepare_source(
        &self,
        source: &str,
        abi: &str,
    ) -> Result<mush_roll::PreparedSource<()>, mush_roll::RollError> {
        if abi != tacsim::minds::MIND_ABI || ![FIXTURE_I_RHAI, LOAD_III_RHAI].contains(&source) {
            return Err(mush_roll::RollError::Missing);
        }
        // The test content is already immutable in this binary. In particular,
        // do not populate Tacsim's source map: NoSources verifies checkout does.
        Ok(mush_roll::PreparedSource {
            digest: mush_roll::source_digest(source),
            operations: Vec::new(),
            token: (),
        })
    }
    fn read_source(
        &self,
        wanted: mush_core::DefinitionDigest,
    ) -> Result<&str, mush_roll::RollError> {
        [FIXTURE_I_RHAI, LOAD_III_RHAI]
            .into_iter()
            .find(|text| mush_roll::source_digest(text) == wanted)
            .ok_or(mush_roll::RollError::Missing)
    }
}

/// Seed the test adapter's three real immutable Roll revisions during fixture
/// setup, before any command. This is not a production save/metadata bypass.
/// Returned writes let a restarting fixture retain its complete setup image.
pub fn seed_fixture_revisions(w: &mut World) -> Vec<mush_core::ModuleOp> {
    let owner = w
        .module_state_mut::<mush_roll::RollStore>()
        .expect("real Roll owner");
    let mut writes = Vec::new();
    for (id, name, class, source) in [
        (1, "Fixture I", CoreClass::I, FIXTURE_I_RHAI),
        (2, "Load III", CoreClass::III, LOAD_III_RHAI),
        (3, "Fixture II", CoreClass::II, FIXTURE_I_RHAI),
    ] {
        if owner.mind(ObjId(1), name).is_err() {
            let create = owner
                .prepare_create(
                    ObjId(1),
                    name,
                    mush_roll::WorkingCopy {
                        source: source.into(),
                        envelope: fixture_envelope(class),
                    },
                )
                .expect("bounded fixture draft");
            owner
                .publish_with(create, |ops| writes.extend_from_slice(ops))
                .expect("fresh create");
            let base = owner.mind(ObjId(1), name).expect("created fixture").base();
            let save = owner
                .prepare_save(
                    ObjId(1),
                    name,
                    base,
                    "legacy test adapter",
                    None,
                    &FixtureSource,
                    &FixtureSource,
                )
                .expect("actual compiled fixture save");
            let (change, _, reference) = save.into_parts();
            assert_eq!(reference.key.mind, id);
            owner
                .publish_with(change, |ops| writes.extend_from_slice(ops))
                .expect("fresh save");
        }
        let reference = mush_roll::RevisionRef {
            key: mush_roll::RevisionKey {
                character: ObjId(1),
                mind: id,
                number: 1,
            },
            digest: fixture_envelope(class)
                .digest(source)
                .expect("fixture digest"),
        };
        let revision = owner
            .revision(ObjId(1), reference.key)
            .expect("retained exact revision");
        assert_eq!(revision.reference(), reference);
        revision
            .verify_source(&FixtureSource)
            .expect("retained source identity");
    }
    writes
}

/// **`Fixture I`, as the provider holds it.**
#[must_use]
pub fn fixture_i() -> HeldMind {
    HeldMind {
        definition: None,
        saved: None,
        identity: SealIdentity {
            mind: SmolStr::new_static(FIXTURE_I),
            origin: fixture_origin(1, CoreClass::I, FIXTURE_I_RHAI),
            source_sha256: digest(FIXTURE_I_RHAI),
        },
        rated: CoreClass::I,
        engine: SealEngine::Scripted,
        presets: vec![SmolStr::new_static("Line"), SmolStr::new_static("Sniper")],
        source: Some(Arc::from(FIXTURE_I_RHAI)),
    }
}

/// **`Load III`, as the provider holds it.**
#[must_use]
pub fn load_iii() -> HeldMind {
    HeldMind {
        definition: None,
        saved: None,
        identity: SealIdentity {
            mind: SmolStr::new_static(LOAD_III),
            origin: fixture_origin(2, CoreClass::III, LOAD_III_RHAI),
            source_sha256: digest(LOAD_III_RHAI),
        },
        rated: CoreClass::III,
        engine: SealEngine::Scripted,
        presets: vec![SmolStr::new_static("Line")],
        source: Some(Arc::from(LOAD_III_RHAI)),
    }
}

/// **The test provider**: Standard Doctrine, `Fixture I` (until dropped) and
/// `Load III`, for every character.
#[derive(Debug, Default)]
pub struct TestHoldings {
    fixture_i_dropped: AtomicBool,
}

impl TestHoldings {
    /// **Take `Fixture I` away from every character** — a holding lost
    /// between the choice and the check-out.
    pub fn drop_fixture_i(&self) {
        self.fixture_i_dropped.store(true, Ordering::SeqCst);
    }
}

impl Holdings for TestHoldings {
    fn held(&self, _w: &World, _character: ObjId) -> Vec<HeldMind> {
        let mut held = vec![HeldMind::doctrine()];
        if !self.fixture_i_dropped.load(Ordering::SeqCst) {
            held.push(fixture_i());
        }
        held.push(load_iii());
        held
    }

    fn sources(&self) -> Vec<Arc<str>> {
        vec![Arc::from(FIXTURE_I_RHAI), Arc::from(LOAD_III_RHAI)]
    }
}

/// **Install the test provider on `w`** through the production seam, and
/// hand back the provider so a test can drop a holding later.
pub fn install(w: &mut World) -> Arc<TestHoldings> {
    let _setup_writes = seed_fixture_revisions(w);
    let holdings = Arc::new(TestHoldings::default());
    tacsim::match_::holdings::install_holdings(w, holdings.clone());
    holdings
}

/// **The test provider with no sources** (17b cut M5.2; the orchestrator's
/// carried item from 17b cut M5.1): every character holds what
/// [`TestHoldings`] holds — `Fixture I`'s holding carries its source text —
/// but [`Holdings::sources`] hands out nothing, so [`install_holdings`]
/// installs no source and the check-out alone must put a chosen mind's
/// source in the store before its seal is used.
///
/// [`install_holdings`]: tacsim::match_::holdings::install_holdings
#[derive(Debug, Default)]
pub struct NoSources(TestHoldings);

impl Holdings for NoSources {
    fn held(&self, w: &World, character: ObjId) -> Vec<HeldMind> {
        self.0.held(w, character)
    }
}

/// **Install [`NoSources`] on `w`** through the production seam.
pub fn install_without_sources(w: &mut World) {
    let _setup_writes = seed_fixture_revisions(w);
    tacsim::match_::holdings::install_holdings(w, Arc::new(NoSources::default()));
}
