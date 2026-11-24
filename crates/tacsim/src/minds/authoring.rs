//! Validation at the character library's immutable-save boundary.
//!
//! Working text may be invalid. A revision must have a canonical, valid card
//! envelope and compile in the same bounded sandbox used at execution. Admission
//! against a particular core, chassis, Compact floor and operator policy happens
//! at selection and sealing, because those are not properties of the source.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Mutex, OnceLock, PoisonError};

use mush_core::DefinitionDigest;
use mush_mind::{Digest, EngineLimits, Sandbox};
use mush_roll::{Envelope, RevisionAdapter, RollError};
use sha2::{Digest as _, Sha256};

use crate::arena::core::CoreClass;

/// Maximum cached retained-ABI validation verdicts (both valid and rejected) in RAM.
const MAX_RETAINED_CACHE: usize = 256;
const MAX_CACHED_ERR_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct RetainedCacheKey {
    revision: DefinitionDigest,
    source: DefinitionDigest,
    abi: &'static str,
    executable: bool,
}

type CachedVerdict = Result<(), (String, String)>;

#[derive(Default)]
struct RetainedCache {
    entries: BTreeMap<RetainedCacheKey, CachedVerdict>,
    order: VecDeque<RetainedCacheKey>,
}

impl RetainedCache {
    fn get(&mut self, key: &RetainedCacheKey) -> Option<CachedVerdict> {
        let hit = self.entries.get(key)?.clone();
        self.order.retain(|old| old != key);
        self.order.push_back(*key);
        Some(hit)
    }

    fn insert(&mut self, key: RetainedCacheKey, value: CachedVerdict) {
        if self.entries.contains_key(&key) {
            self.order.retain(|old| old != &key);
            self.order.push_back(key);
            self.entries.insert(key, value);
            return;
        }
        while self.entries.len() >= MAX_RETAINED_CACHE {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            } else {
                self.entries.clear();
            }
        }
        self.order.push_back(key);
        self.entries.insert(key, value);
    }
}

fn retained_cache() -> &'static Mutex<RetainedCache> {
    static CACHE: OnceLock<Mutex<RetainedCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(RetainedCache::default()))
}

/// Process-wide validation sandbox, built once and kept separate from
/// [`super::standard_sandbox`] so save/validation traffic never evicts hot
/// active match runners while eliminating repeated `Sandbox::new` construction.
fn validation_sandbox() -> &'static Sandbox {
    static SANDBOX: OnceLock<Sandbox> = OnceLock::new();
    SANDBOX.get_or_init(|| Sandbox::new(&EngineLimits::default(), super::register_api))
}

fn truncate_cached(mut text: String) -> String {
    if text.len() > MAX_CACHED_ERR_BYTES {
        let mut cut = MAX_CACHED_ERR_BYTES;
        while cut > 0 && !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
    }
    text
}

fn to_cached(result: &Result<(), RollError>) -> CachedVerdict {
    match result {
        Ok(()) => Ok(()),
        Err(RollError::Invalid { field, message }) => Err((
            truncate_cached(field.clone()),
            truncate_cached(message.clone()),
        )),
        Err(error) => Err(("mind_abi".to_owned(), truncate_cached(error.to_string()))),
    }
}

/// Game semantics supplied to the workflow owner. Validation uses a dedicated
/// process-wide sandbox (separate from `standard_sandbox`), so unsuccessful saves
/// cannot grow or evict the execution cache and never recompile `STANDARD_RHAI`.
#[derive(Clone, Copy, Debug, Default)]
pub struct RevisionRules;

impl RevisionAdapter for RevisionRules {
    fn normalize_source(&self, source: &str) -> Result<String, RollError> {
        super::normalize::normalize_source(source)
    }

    fn validate(&self, source: &str, envelope: &Envelope) -> Result<(), RollError> {
        validate_source(source, envelope, true, true)
    }
}

/// Historical source audit only. This adapter is never the authoring save
/// boundary and grants no certificate or execution authority. A row saved under any
/// ABI since the Registry opened (`super::RETAINED_ABIS`) retains its original
/// parser/metadata validity; current rows still receive full current validation.
/// Re-conformance separately compiles for current use.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RetainedRevisionRules;
impl RevisionAdapter for RetainedRevisionRules {
    fn normalize_source(&self, source: &str) -> Result<String, RollError> {
        super::normalize::normalize_source(source)
    }
    fn validate(&self, source: &str, envelope: &Envelope) -> Result<(), RollError> {
        if envelope.mind_abi == super::MIND_ABI {
            RevisionRules.validate(source, envelope)
        } else {
            let revision_digest = envelope.digest(source)?;
            let source_digest =
                DefinitionDigest::from_bytes(Sha256::digest(source.as_bytes()).into());
            validate_retained_cached(revision_digest, source_digest, source, envelope, false)
        }
    }
}

/// Pure re-conformance validation of an immutable retained filed envelope.
/// This is not source/filing authority. The closed Registry reader and typed
/// producer independently prove the retained filed subject. Author saves keep
/// their exact current-ABI requirement; source and old metadata are unchanged.
pub(crate) fn validate_retained_registry(
    source: &str,
    envelope: &Envelope,
) -> Result<(), RollError> {
    let revision_digest = envelope.digest(source)?;
    let source_digest = DefinitionDigest::from_bytes(Sha256::digest(source.as_bytes()).into());
    validate_retained_cached(revision_digest, source_digest, source, envelope, true)
}

/// Cached retained-ABI validation keyed by `(revision_digest, source_digest, MIND_ABI, executable)`.
/// Callers with an already-verified `Revision` pass its exact digests directly, eliminating
/// synchronous world-thread recompilation on repeated `SIM MIND filed:<id>` and `@mind/source`
/// reads for both valid and incompatible retained minds.
pub(crate) fn validate_retained_cached(
    revision_digest: DefinitionDigest,
    source_digest: DefinitionDigest,
    source: &str,
    envelope: &Envelope,
    executable: bool,
) -> Result<(), RollError> {
    let key = RetainedCacheKey {
        revision: revision_digest,
        source: source_digest,
        abi: super::MIND_ABI,
        executable,
    };
    if let Some(cached) = retained_cache()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&key)
    {
        return cached.map_err(|(field, message)| RollError::Invalid { field, message });
    }
    let result = validate_source(source, envelope, false, executable);
    let verdict = to_cached(&result);
    retained_cache()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(key, verdict);
    result
}

fn validate_source(
    source: &str,
    envelope: &Envelope,
    current_save: bool,
    executable: bool,
) -> Result<(), RollError> {
    envelope.validate()?;
    if source.len() > mush_roll::MAX_SOURCE_BYTES {
        return Err(invalid("source", "maximum 32 KiB"));
    }
    if current_save && envelope.mind_abi != super::MIND_ABI {
        return Err(invalid("mind_abi", "not the current executable ABI"));
    }
    let class = CoreClass::parse(&envelope.declared_class)
        .ok_or_else(|| invalid("declared_class", "expected I, II or III"))?;
    if class == CoreClass::Zero {
        return Err(invalid(
            "declared_class",
            "class 0 is the Registry drive ROM; authored scripts require I, II or III",
        ));
    }
    crate::arena::schema::validate_envelope(
        &envelope.schema,
        &envelope.presets,
        &envelope.profiles,
        &envelope.rules,
        class,
    )
    .map_err(|error| invalid("envelope", error.to_string()))?;
    if current_save {
        super::metadata::validate(source, envelope)?;
    } else {
        super::metadata::validate_retained_registry_checked(source, envelope, !executable)?;
    }
    if !executable {
        return Ok(());
    }
    let digest = Digest(Sha256::digest(source.as_bytes()).into());
    validation_sandbox()
        .compile(source, digest, super::MIND_ABI)
        .map_err(|error| invalid("source", error.to_string()))?;
    Ok(())
}

fn invalid(field: &str, message: impl Into<String>) -> RollError {
    RollError::Invalid {
        field: field.to_owned(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn envelope() -> Envelope {
        Envelope {
            schema: json!({"sections": []}),
            presets: json!({}),
            profiles: json!({"Default": {}}),
            rules: json!({}),
            declared_class: "I".to_owned(),
            mind_abi: crate::minds::MIND_ABI.to_owned(),
        }
    }

    #[test]
    fn saving_requires_the_real_think_signature_and_sandbox() {
        let rules = RevisionRules;
        let envelope = envelope();
        assert!(rules.validate("fn think(view, card) {}", &envelope).is_ok());
        for source in [
            "fn think(view) {}",
            "fn think(view, card) { missing_name; }",
            "fn think(view, card) { eval(\"42\"); }",
            "fn think(view, card) { import \"outside\"; }",
        ] {
            assert!(rules.validate(source, &envelope).is_err(), "{source}");
        }
    }

    #[test]
    fn save_does_not_execute_top_level_statements() {
        assert!(
            RevisionRules
                .validate("while true {} fn think(view, card) {}", &envelope())
                .is_ok()
        );
    }

    #[test]
    fn saving_refuses_unknown_abi_rom_claim_and_invalid_profile_values() {
        let source = "fn think(view, card) {}";
        let mut metadata = envelope();
        metadata.mind_abi = "old".to_owned();
        assert!(RevisionRules.validate(source, &metadata).is_err());
        metadata = envelope();
        metadata.declared_class = "0".to_owned();
        assert!(RevisionRules.validate(source, &metadata).is_err());
        metadata = envelope();
        metadata.profiles = json!({"Default": {"undeclared": 1}});
        assert!(RevisionRules.validate(source, &metadata).is_err());
    }

    #[test]
    fn retained_audit_never_becomes_fresh_save_or_current_execution_authority() {
        let source = "const MIND = #{name: \"Retained\", api: 4, class: \"discretion\", role: \"combat\", core: \"I\"}; fn think(v,c) {}";
        let mut old = envelope();
        old.mind_abi = "rhai=1.26.0;api=4;stock=2".into();
        assert_ne!(old.mind_abi, super::super::MIND_ABI);
        let before = serde_json::to_vec(&old).unwrap();
        assert!(RetainedRevisionRules.validate(source, &old).is_ok());
        assert!(RevisionRules.validate(source, &old).is_err());
        assert_eq!(serde_json::to_vec(&old).unwrap(), before);
        assert_eq!(
            RetainedRevisionRules.normalize_source(source).unwrap(),
            RevisionRules.normalize_source(source).unwrap()
        );
        assert!(
            RetainedRevisionRules
                .validate("fn think(v) {}", &old)
                .is_err()
        );
        let wrong_api = source.replace("api: 4", "api: 3");
        assert!(RetainedRevisionRules.validate(&wrong_api, &old).is_err());
        old.mind_abi = "rhai=1.26.0;api=3;stock=2".into();
        assert!(RetainedRevisionRules.validate(source, &old).is_err());
        assert!(
            RetainedRevisionRules
                .validate("fn think(v,c) { missing_name; }", &envelope())
                .is_err()
        );
    }

    /// **Doc 45 unit AP.3: no ABI since the Registry opened is refused for ever.** A
    /// revision saved under each of them, the one this ABI replaced among them, is still an
    /// archive the cold audit reads and a subject the Registry may rate again, and is never
    /// a fresh save. Its header's `api` is the one its envelope names and no other.
    #[test]
    fn every_abi_since_the_registry_opened_is_retained_and_none_is_a_fresh_save() {
        assert!(
            super::super::RETAINED_ABIS
                .iter()
                .any(|(abi, api)| *abi == "rhai=1.26.0;api=8;stock=3" && *api == 8),
            "the ABI that API 9 replaced"
        );
        assert!(
            super::super::RETAINED_ABIS
                .iter()
                .any(|(abi, api)| *abi == "rhai=1.26.0;api=9;stock=3" && *api == 9),
            "the ABI that API 10 replaced (doc 49 unit CC.9)"
        );
        for (abi, api) in super::super::RETAINED_ABIS {
            let source = format!(
                "const MIND = #{{name: \"Retained\", api: {api}, class: \"discretion\", role: \"combat\", core: \"I\"}}; fn think(v,c) {{ this.marker = 73; }}"
            );
            let mut old = envelope();
            old.mind_abi = abi.to_owned();
            let before = serde_json::to_vec(&old).unwrap();
            assert!(
                RetainedRevisionRules.validate(&source, &old).is_ok(),
                "{abi}"
            );
            assert!(validate_retained_registry(&source, &old).is_ok(), "{abi}");
            assert!(RevisionRules.validate(&source, &old).is_err(), "{abi}");
            assert_eq!(serde_json::to_vec(&old).unwrap(), before, "{abi}");
            // The header must name the API its own envelope does.
            let other = source.replace(&format!("api: {api}"), &format!("api: {}", api + 1));
            assert!(
                RetainedRevisionRules.validate(&other, &old).is_err(),
                "{abi}"
            );
            // Retained is not a licence to run what no longer compiles.
            assert!(
                validate_retained_registry("fn think(v,c) { missing_name; }", &old).is_err(),
                "{abi}"
            );
        }
        // An ABI that never was is still refused.
        let mut never = envelope();
        never.mind_abi = "rhai=1.26.0;api=8;stock=2".into();
        assert!(
            RetainedRevisionRules
                .validate("fn think(v,c) {}", &never)
                .is_err()
        );
    }

    /// **Doc 45 unit AP.3: a script that reads the posture saves under API 9**, and its
    /// header may not claim the API before the getter existed.
    #[test]
    fn a_script_that_reads_the_posture_saves_under_the_current_api() {
        let source = format!(
            "const MIND = #{{name: \"Posture\", api: {}, class: \"discretion\", role: \"combat\", core: \"I\"}}; fn think(v,c) {{ this.posture = v.posture; }}",
            super::super::API_VERSION
        );
        assert!(RevisionRules.validate(&source, &envelope()).is_ok());
        assert!(
            RevisionRules
                .validate(
                    &source.replace(&format!("api: {}", super::super::API_VERSION), "api: 8"),
                    &envelope()
                )
                .is_err()
        );
    }

    #[test]
    fn retained_api5_keeps_its_metadata_without_granting_current_save_authority() {
        let source = "const MIND = #{name: \"Retained\", api: 5, class: \"discretion\", role: \"combat\", core: \"I\"}; fn think(v,c) {}";
        let mut old = envelope();
        old.mind_abi = "rhai=1.26.0;api=5;stock=2".into();
        let before = serde_json::to_vec(&old).unwrap();
        assert!(RetainedRevisionRules.validate(source, &old).is_ok());
        assert!(validate_retained_registry(source, &old).is_ok());
        assert!(RevisionRules.validate(source, &old).is_err());
        assert_eq!(serde_json::to_vec(&old).unwrap(), before);
        assert!(
            RetainedRevisionRules
                .validate(&source.replace("api: 5", "api: 6"), &old)
                .is_err()
        );
    }

    #[test]
    fn p2_tsc_19_retained_validation_caches_both_valid_and_rejected_with_bounded_lru() {
        let mut old = envelope();
        old.mind_abi = "rhai=1.26.0;api=5;stock=2".into();
        let ok_src = "const MIND = #{name: \"CacheHit\", api: 5, class: \"discretion\", role: \"combat\", core: \"I\"}; fn think(v, c) { this.x = 19; }";
        let bad_src = "const MIND = #{name: \"CacheBad\", api: 5, class: \"discretion\", role: \"combat\", core: \"I\"}; fn think(v, c) { missing_p2_tsc_19_ident; }";

        // Prime entries (executable = true and executable = false) for valid and rejected minds.
        assert!(validate_retained_registry(ok_src, &old).is_ok());
        assert!(RetainedRevisionRules.validate(ok_src, &old).is_ok());
        assert!(validate_retained_registry(bad_src, &old).is_err());

        let ok_rev = old.digest(ok_src).unwrap();
        let ok_src_dig = DefinitionDigest::from_bytes(Sha256::digest(ok_src.as_bytes()).into());
        let bad_rev = old.digest(bad_src).unwrap();
        let bad_src_dig = DefinitionDigest::from_bytes(Sha256::digest(bad_src.as_bytes()).into());
        {
            let mut cache = retained_cache()
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            assert!(
                matches!(
                    cache.get(&RetainedCacheKey {
                        revision: ok_rev,
                        source: ok_src_dig,
                        abi: super::super::MIND_ABI,
                        executable: true,
                    }),
                    Some(Ok(()))
                ),
                "executable valid retained mind must be cached"
            );
            assert!(
                matches!(
                    cache.get(&RetainedCacheKey {
                        revision: ok_rev,
                        source: ok_src_dig,
                        abi: super::super::MIND_ABI,
                        executable: false,
                    }),
                    Some(Ok(()))
                ),
                "archival valid retained mind must be cached"
            );
            assert!(
                matches!(
                    cache.get(&RetainedCacheKey {
                        revision: bad_rev,
                        source: bad_src_dig,
                        abi: super::super::MIND_ABI,
                        executable: true,
                    }),
                    Some(Err(_))
                ),
                "incompatible retained mind must be negatively cached"
            );
        }
        // Repeated calls with the exact digests return the cached verdict without recompiling.
        for _ in 0..8 {
            assert!(validate_retained_cached(ok_rev, ok_src_dig, ok_src, &old, true).is_ok());
            assert!(validate_retained_cached(ok_rev, ok_src_dig, ok_src, &old, false).is_ok());
            assert!(validate_retained_cached(bad_rev, bad_src_dig, bad_src, &old, true).is_err());
        }

        // Verify LRU capacity stays strictly bounded at MAX_RETAINED_CACHE.
        let mut lru = RetainedCache::default();
        for i in 0..(MAX_RETAINED_CACHE + 32) {
            let mut b = [0u8; 32];
            b[..8].copy_from_slice(&(i as u64).to_le_bytes());
            let d = DefinitionDigest::from_bytes(b);
            lru.insert(
                RetainedCacheKey {
                    revision: d,
                    source: d,
                    abi: super::super::MIND_ABI,
                    executable: true,
                },
                Ok(()),
            );
            assert!(lru.entries.len() <= MAX_RETAINED_CACHE);
            assert!(lru.order.len() <= MAX_RETAINED_CACHE);
        }
    }
}
