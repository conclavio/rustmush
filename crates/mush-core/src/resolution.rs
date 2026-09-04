//! Deterministic, domain-separated authoritative randomness.
//!
//! Providers receive a short-lived [`ResolutionRng`], never root key bytes.
//! Each named stream has an independent counter, so adding a cosmetic draw
//! cannot shift hit-location or damage results. The resulting transcript is
//! stored in a [`ResolutionRecord`]; historical replay applies the recorded
//! outcome rather than running formulas again.

use std::collections::BTreeMap;
use std::fmt;

use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::json;
use sha2::Sha256;
use thiserror::Error;

use crate::action::{CanonicalHash, CanonicalHashError, CommitToken, ScopedIdempotencyKey};
use crate::mechanics::{CanonicalId, DefinitionRef, RulesValue};

const RESOLUTION_KDF_DOMAIN: &[u8] = b"rustmush.resolution.kdf.v1\0";
const RESOLUTION_DRAW_DOMAIN: &[u8] = b"rustmush.resolution.draw.v1\0";

/// Maximum root-key versions retained during rotation.
pub const MAX_RESOLUTION_SECRET_VERSIONS: usize = 4;
/// Maximum independent labels used by one resolution.
pub const MAX_RESOLUTION_STREAMS: usize = 64;
/// Maximum logical draws recorded by one resolution.
pub const MAX_RESOLUTION_DRAWS: usize = 1_024;
/// Maximum exact definitions named by one resolution record.
pub const MAX_RESOLUTION_DEFINITIONS: usize = 64;
/// Defensive bound on rejection samples for one unbiased bounded draw.
pub const MAX_RNG_REJECTION_ATTEMPTS: u32 = 128;

/// Exact deterministic algorithm/KDF contract used by a resolution record.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RngAlgorithmVersion {
    HmacSha256CounterV1,
}

/// One root-secret version loaded from durable secret storage. Key bytes are
/// deliberately neither serializable nor printable.
#[derive(Clone)]
pub struct ResolutionSecret {
    version: u32,
    key: [u8; 32],
}

impl fmt::Debug for ResolutionSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolutionSecret")
            .field("version", &self.version)
            .field("key", &"[REDACTED]")
            .finish()
    }
}

impl ResolutionSecret {
    pub fn new(version: u32, key: [u8; 32]) -> Result<Self, ResolutionSecretError> {
        if version == 0 {
            return Err(ResolutionSecretError::ZeroVersion);
        }
        if key == [0; 32] {
            return Err(ResolutionSecretError::ZeroKey);
        }
        Ok(Self { version, key })
    }

    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }
}

/// Current plus retained KDF secrets. Rotation must retain any version named
/// by unresolved accepted work; callers select that exact version on resume.
#[derive(Clone)]
pub struct ResolutionKeyRing {
    current: ResolutionSecret,
    retained: BTreeMap<u32, ResolutionSecret>,
}

impl fmt::Debug for ResolutionKeyRing {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolutionKeyRing")
            .field("current_version", &self.current.version)
            .field("retained_versions", &self.retained.keys())
            .finish()
    }
}

impl ResolutionKeyRing {
    pub fn new(
        current: ResolutionSecret,
        retained: impl IntoIterator<Item = ResolutionSecret>,
    ) -> Result<Self, ResolutionSecretError> {
        let mut retained_by_version = BTreeMap::new();
        for secret in retained {
            if secret.version >= current.version {
                return Err(ResolutionSecretError::NonMonotonicVersion);
            }
            if retained_by_version.insert(secret.version, secret).is_some() {
                return Err(ResolutionSecretError::DuplicateVersion);
            }
        }
        if retained_by_version.len().saturating_add(1) > MAX_RESOLUTION_SECRET_VERSIONS {
            return Err(ResolutionSecretError::VersionLimit {
                max: MAX_RESOLUTION_SECRET_VERSIONS,
            });
        }
        Ok(Self {
            current,
            retained: retained_by_version,
        })
    }

    #[must_use]
    pub const fn current_version(&self) -> u32 {
        self.current.version
    }

    /// Start new work under the current durable root-key version.
    pub fn start(
        &self,
        idempotency: &ScopedIdempotencyKey,
        domain: &CanonicalId,
        inputs_hash: CanonicalHash,
    ) -> Result<ResolutionRng, ResolutionRngError> {
        ResolutionRng::from_secret(&self.current, idempotency, domain, inputs_hash)
    }

    /// Resume accepted work under the exact secret version it recorded.
    pub fn resume(
        &self,
        secret_version: u32,
        idempotency: &ScopedIdempotencyKey,
        domain: &CanonicalId,
        inputs_hash: CanonicalHash,
    ) -> Result<ResolutionRng, ResolutionRngError> {
        let secret = if self.current.version == secret_version {
            &self.current
        } else {
            self.retained
                .get(&secret_version)
                .ok_or(ResolutionRngError::UnknownSecretVersion {
                    version: secret_version,
                })?
        };
        ResolutionRng::from_secret(secret, idempotency, domain, inputs_hash)
    }
}

/// Invalid durable resolution key ring.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ResolutionSecretError {
    #[error("resolution secret version zero is reserved")]
    ZeroVersion,
    #[error("an all-zero resolution secret is not accepted")]
    ZeroKey,
    #[error("retained resolution-secret versions must precede the current version")]
    NonMonotonicVersion,
    #[error("resolution secret version appears more than once")]
    DuplicateVersion,
    #[error("resolution key ring exceeds its hard limit of {max} versions")]
    VersionLimit { max: usize },
}

/// Shape and result of one logical deterministic draw.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecordedDrawValue {
    RawU32 { value: u32 },
    RangeU64 { upper_exclusive: u64, value: u64 },
}

/// One labeled draw retained in chronological call order.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedDraw {
    label: CanonicalId,
    ordinal: u32,
    draw: RecordedDrawValue,
}

impl RecordedDraw {
    #[must_use]
    pub const fn label(&self) -> &CanonicalId {
        &self.label
    }

    #[must_use]
    pub const fn ordinal(&self) -> u32 {
        self.ordinal
    }

    #[must_use]
    pub const fn draw(&self) -> RecordedDrawValue {
        self.draw
    }
}

/// Non-cloneable resolution-scoped deterministic generator.
pub struct ResolutionRng {
    algorithm: RngAlgorithmVersion,
    secret_version: u32,
    seed: [u8; 32],
    inputs_hash: CanonicalHash,
    next_by_label: BTreeMap<CanonicalId, u32>,
    draws: Vec<RecordedDraw>,
}

impl fmt::Debug for ResolutionRng {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolutionRng")
            .field("algorithm", &self.algorithm)
            .field("secret_version", &self.secret_version)
            .field("seed", &"[REDACTED]")
            .field("inputs_hash", &self.inputs_hash)
            .field("streams", &self.next_by_label.len())
            .field("draws", &self.draws.len())
            .finish()
    }
}

impl ResolutionRng {
    fn from_secret(
        secret: &ResolutionSecret,
        idempotency: &ScopedIdempotencyKey,
        domain: &CanonicalId,
        inputs_hash: CanonicalHash,
    ) -> Result<Self, ResolutionRngError> {
        let context = CanonicalHash::for_canonical_value(
            "resolution_seed_context.v1",
            1,
            &json!({
                "idempotency": idempotency,
                "domain": domain,
                "inputs_hash": inputs_hash,
            }),
            64 * 1024,
        )?;
        let mut mac = Hmac::<Sha256>::new_from_slice(&secret.key)
            .expect("HMAC-SHA256 accepts a fixed 32-byte key");
        mac.update(RESOLUTION_KDF_DOMAIN);
        mac.update(&secret.version.to_be_bytes());
        mac.update(context.as_bytes());
        let seed: [u8; 32] = mac.finalize().into_bytes().into();
        Ok(Self {
            algorithm: RngAlgorithmVersion::HmacSha256CounterV1,
            secret_version: secret.version,
            seed,
            inputs_hash,
            next_by_label: BTreeMap::new(),
            draws: Vec::new(),
        })
    }

    #[must_use]
    pub const fn algorithm(&self) -> RngAlgorithmVersion {
        self.algorithm
    }

    #[must_use]
    pub const fn secret_version(&self) -> u32 {
        self.secret_version
    }

    #[must_use]
    pub const fn inputs_hash(&self) -> CanonicalHash {
        self.inputs_hash
    }

    #[must_use]
    pub fn draws(&self) -> &[RecordedDraw] {
        &self.draws
    }

    /// Draw one raw word from a named independent stream.
    pub fn draw_u32(&mut self, label: &CanonicalId) -> Result<u32, ResolutionRngError> {
        let ordinal = self.reserve_draw(label)?;
        let value = u32::from_be_bytes(
            self.block(label, ordinal, 0)[..4]
                .try_into()
                .expect("four-byte slice"),
        );
        self.draws.push(RecordedDraw {
            label: label.clone(),
            ordinal,
            draw: RecordedDrawValue::RawU32 { value },
        });
        Ok(value)
    }

    /// Draw uniformly from `0..upper_exclusive` using bounded rejection
    /// sampling. One successful result consumes one logical draw ordinal.
    pub fn draw_bounded_u64(
        &mut self,
        label: &CanonicalId,
        upper_exclusive: u64,
    ) -> Result<u64, ResolutionRngError> {
        if upper_exclusive == 0 {
            return Err(ResolutionRngError::EmptyRange);
        }
        let ordinal = self.reserve_draw(label)?;
        let threshold = upper_exclusive.wrapping_neg() % upper_exclusive;
        let mut selected = None;
        for attempt in 0..MAX_RNG_REJECTION_ATTEMPTS {
            let raw = u64::from_be_bytes(
                self.block(label, ordinal, attempt)[..8]
                    .try_into()
                    .expect("eight-byte slice"),
            );
            if raw >= threshold {
                selected = Some(raw % upper_exclusive);
                break;
            }
        }
        let value = selected.ok_or(ResolutionRngError::RejectionLimit {
            max: MAX_RNG_REJECTION_ATTEMPTS,
        })?;
        self.draws.push(RecordedDraw {
            label: label.clone(),
            ordinal,
            draw: RecordedDrawValue::RangeU64 {
                upper_exclusive,
                value,
            },
        });
        Ok(value)
    }

    fn reserve_draw(&mut self, label: &CanonicalId) -> Result<u32, ResolutionRngError> {
        if self.draws.len() >= MAX_RESOLUTION_DRAWS {
            return Err(ResolutionRngError::DrawLimit {
                max: MAX_RESOLUTION_DRAWS,
            });
        }
        if !self.next_by_label.contains_key(label)
            && self.next_by_label.len() >= MAX_RESOLUTION_STREAMS
        {
            return Err(ResolutionRngError::StreamLimit {
                max: MAX_RESOLUTION_STREAMS,
            });
        }
        let ordinal = *self.next_by_label.get(label).unwrap_or(&0);
        let next = ordinal
            .checked_add(1)
            .ok_or(ResolutionRngError::OrdinalExhausted)?;
        self.next_by_label.insert(label.clone(), next);
        Ok(ordinal)
    }

    fn block(&self, label: &CanonicalId, ordinal: u32, attempt: u32) -> [u8; 32] {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.seed)
            .expect("HMAC-SHA256 accepts a fixed 32-byte key");
        mac.update(RESOLUTION_DRAW_DOMAIN);
        mac.update(&(label.as_str().len() as u64).to_be_bytes());
        mac.update(label.as_str().as_bytes());
        mac.update(&ordinal.to_be_bytes());
        mac.update(&attempt.to_be_bytes());
        mac.finalize().into_bytes().into()
    }

    /// Consume the generator into provider output that has no commit token.
    /// The central coordinator binds the token while building the bundle.
    pub fn into_prepared(
        self,
        mut definitions: Vec<DefinitionRef>,
        modifiers: RulesValue,
        outcome: RulesValue,
    ) -> Result<PreparedResolution, ResolutionRecordError> {
        definitions.sort();
        PreparedResolution::new(
            self.algorithm,
            self.secret_version,
            definitions,
            self.inputs_hash,
            self.draws,
            modifiers,
            outcome,
        )
    }
}

/// Provider-produced resolution material before the coordinator assigns its
/// authoritative commit identity. This type is intentionally not serializable.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedResolution {
    algorithm: RngAlgorithmVersion,
    secret_version: u32,
    definition_refs: Vec<DefinitionRef>,
    inputs_hash: CanonicalHash,
    draws: Vec<RecordedDraw>,
    modifiers: RulesValue,
    outcome: RulesValue,
}

impl PreparedResolution {
    #[allow(clippy::too_many_arguments)]
    fn new(
        algorithm: RngAlgorithmVersion,
        secret_version: u32,
        definition_refs: Vec<DefinitionRef>,
        inputs_hash: CanonicalHash,
        draws: Vec<RecordedDraw>,
        modifiers: RulesValue,
        outcome: RulesValue,
    ) -> Result<Self, ResolutionRecordError> {
        validate_resolution_parts(secret_version, &definition_refs, &draws)?;
        Ok(Self {
            algorithm,
            secret_version,
            definition_refs,
            inputs_hash,
            draws,
            modifiers,
            outcome,
        })
    }

    #[must_use]
    pub fn definition_refs(&self) -> &[DefinitionRef] {
        &self.definition_refs
    }

    #[must_use]
    pub const fn modifiers(&self) -> &RulesValue {
        &self.modifiers
    }

    #[must_use]
    pub const fn outcome(&self) -> &RulesValue {
        &self.outcome
    }

    /// Attach the coordinator-owned commit identity.
    #[must_use]
    pub fn bind(self, action: CommitToken) -> ResolutionRecord {
        ResolutionRecord {
            schema_version: ResolutionRecord::SCHEMA_VERSION,
            action,
            algorithm: self.algorithm,
            secret_version: self.secret_version,
            definition_refs: self.definition_refs,
            inputs_hash: self.inputs_hash,
            draws: self.draws,
            modifiers: self.modifiers,
            outcome: self.outcome,
        }
    }
}

/// Immutable authoritative resolution evidence and chosen outcome.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResolutionRecord {
    schema_version: u16,
    action: CommitToken,
    algorithm: RngAlgorithmVersion,
    secret_version: u32,
    definition_refs: Vec<DefinitionRef>,
    inputs_hash: CanonicalHash,
    draws: Vec<RecordedDraw>,
    modifiers: RulesValue,
    outcome: RulesValue,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolutionRecordWire {
    schema_version: u16,
    action: CommitToken,
    algorithm: RngAlgorithmVersion,
    secret_version: u32,
    definition_refs: Vec<DefinitionRef>,
    inputs_hash: CanonicalHash,
    draws: Vec<RecordedDraw>,
    modifiers: RulesValue,
    outcome: RulesValue,
}

impl<'de> Deserialize<'de> for ResolutionRecord {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ResolutionRecordWire::deserialize(deserializer)?;
        Self::from_parts(
            wire.schema_version,
            wire.action,
            wire.algorithm,
            wire.secret_version,
            wire.definition_refs,
            wire.inputs_hash,
            wire.draws,
            wire.modifiers,
            wire.outcome,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl ResolutionRecord {
    pub const SCHEMA_VERSION: u16 = 1;

    #[allow(clippy::too_many_arguments)]
    fn from_parts(
        schema_version: u16,
        action: CommitToken,
        algorithm: RngAlgorithmVersion,
        secret_version: u32,
        definition_refs: Vec<DefinitionRef>,
        inputs_hash: CanonicalHash,
        draws: Vec<RecordedDraw>,
        modifiers: RulesValue,
        outcome: RulesValue,
    ) -> Result<Self, ResolutionRecordError> {
        if schema_version != Self::SCHEMA_VERSION {
            return Err(ResolutionRecordError::UnsupportedSchema {
                actual: schema_version,
                supported: Self::SCHEMA_VERSION,
            });
        }
        validate_resolution_parts(secret_version, &definition_refs, &draws)?;
        Ok(Self {
            schema_version,
            action,
            algorithm,
            secret_version,
            definition_refs,
            inputs_hash,
            draws,
            modifiers,
            outcome,
        })
    }

    #[must_use]
    pub const fn action(&self) -> CommitToken {
        self.action
    }

    #[must_use]
    pub const fn algorithm(&self) -> RngAlgorithmVersion {
        self.algorithm
    }

    #[must_use]
    pub const fn secret_version(&self) -> u32 {
        self.secret_version
    }

    #[must_use]
    pub fn definition_refs(&self) -> &[DefinitionRef] {
        &self.definition_refs
    }

    #[must_use]
    pub const fn inputs_hash(&self) -> CanonicalHash {
        self.inputs_hash
    }

    #[must_use]
    pub fn draws(&self) -> &[RecordedDraw] {
        &self.draws
    }

    #[must_use]
    pub const fn modifiers(&self) -> &RulesValue {
        &self.modifiers
    }

    #[must_use]
    pub const fn outcome(&self) -> &RulesValue {
        &self.outcome
    }
}

fn validate_resolution_parts(
    secret_version: u32,
    definition_refs: &[DefinitionRef],
    draws: &[RecordedDraw],
) -> Result<(), ResolutionRecordError> {
    if secret_version == 0 {
        return Err(ResolutionRecordError::ZeroSecretVersion);
    }
    if definition_refs.len() > MAX_RESOLUTION_DEFINITIONS {
        return Err(ResolutionRecordError::DefinitionLimit {
            max: MAX_RESOLUTION_DEFINITIONS,
        });
    }
    if definition_refs.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(ResolutionRecordError::NonCanonicalDefinitions);
    }
    validate_draws(draws)
}

fn validate_draws(draws: &[RecordedDraw]) -> Result<(), ResolutionRecordError> {
    if draws.len() > MAX_RESOLUTION_DRAWS {
        return Err(ResolutionRecordError::DrawLimit {
            max: MAX_RESOLUTION_DRAWS,
        });
    }
    let mut ordinals = BTreeMap::<&CanonicalId, u32>::new();
    for draw in draws {
        let expected = *ordinals.get(&draw.label).unwrap_or(&0);
        if draw.ordinal != expected {
            return Err(ResolutionRecordError::NonCanonicalOrdinal {
                label: draw.label.clone(),
                actual: draw.ordinal,
                expected,
            });
        }
        match draw.draw {
            RecordedDrawValue::RawU32 { .. } => {}
            RecordedDrawValue::RangeU64 {
                upper_exclusive,
                value,
            } => {
                if upper_exclusive == 0 || value >= upper_exclusive {
                    return Err(ResolutionRecordError::InvalidRangeDraw);
                }
            }
        }
        ordinals.insert(&draw.label, expected + 1);
    }
    if ordinals.len() > MAX_RESOLUTION_STREAMS {
        return Err(ResolutionRecordError::StreamLimit {
            max: MAX_RESOLUTION_STREAMS,
        });
    }
    Ok(())
}

/// Failure to construct or draw from an authoritative generator.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ResolutionRngError {
    #[error("resolution secret version {version} is unavailable")]
    UnknownSecretVersion { version: u32 },
    #[error("resolution uses at most {max} independent streams")]
    StreamLimit { max: usize },
    #[error("resolution uses at most {max} logical draws")]
    DrawLimit { max: usize },
    #[error("bounded draw range must not be empty")]
    EmptyRange,
    #[error("resolution draw ordinal is exhausted")]
    OrdinalExhausted,
    #[error("bounded draw exceeded {max} deterministic rejection attempts")]
    RejectionLimit { max: u32 },
    #[error("cannot hash resolution seed context: {0}")]
    Hash(#[from] CanonicalHashError),
}

/// Invalid immutable resolution evidence.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ResolutionRecordError {
    #[error("unsupported resolution record schema {actual}; supported schema is {supported}")]
    UnsupportedSchema { actual: u16, supported: u16 },
    #[error("resolution secret version zero is reserved")]
    ZeroSecretVersion,
    #[error("resolution names more than {max} exact definitions")]
    DefinitionLimit { max: usize },
    #[error("resolution definitions must be strictly sorted and duplicate-free")]
    NonCanonicalDefinitions,
    #[error("resolution records more than {max} logical draws")]
    DrawLimit { max: usize },
    #[error("resolution records more than {max} independent streams")]
    StreamLimit { max: usize },
    #[error("resolution draw {label} ordinal {actual} should be {expected}")]
    NonCanonicalOrdinal {
        label: CanonicalId,
        actual: u32,
        expected: u32,
    },
    #[error("resolution bounded draw has an empty range or out-of-range value")]
    InvalidRangeDraw,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{
        IdempotencyKey, IdempotencyScope, PrincipalId, RequestUid, ScopedIdempotencyKey,
    };
    use crate::mechanics::{
        DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder,
    };

    fn key(request_byte: u8) -> ScopedIdempotencyKey {
        let request = RequestUid::from_bytes([request_byte; 16]);
        let principal = PrincipalId::Object { id: crate::GOD };
        ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: crate::OwnerId::new("toy").unwrap(),
                namespace: CanonicalId::new("attack").unwrap(),
            },
            0,
            IdempotencyKey::Request(request),
        )
    }

    fn ring() -> ResolutionKeyRing {
        ResolutionKeyRing::new(
            ResolutionSecret::new(2, [0x22; 32]).unwrap(),
            [ResolutionSecret::new(1, [0x11; 32]).unwrap()],
        )
        .unwrap()
    }

    fn inputs() -> CanonicalHash {
        CanonicalHash::for_canonical_value("test_inputs.v1", 1, &json!({ "pool": 7 }), 1024)
            .unwrap()
    }

    fn definition(id: &str) -> DefinitionRef {
        DefinitionRegistryBuilder::new()
            .register(
                crate::OwnerId::new("toy").unwrap(),
                DefinitionDescriptor::new(
                    DefinitionKind::new("action").unwrap(),
                    DefinitionId::new(id).unwrap(),
                    1,
                    json!({}),
                )
                .unwrap(),
            )
            .unwrap()
    }

    #[test]
    fn same_context_repeats_and_labels_do_not_shift_each_other() {
        let ring = ring();
        let domain = CanonicalId::new("tactical_attack").unwrap();
        let hit = CanonicalId::new("hit_location").unwrap();
        let flavor = CanonicalId::new("flavor").unwrap();

        let mut first = ring.start(&key(1), &domain, inputs()).unwrap();
        let expected_hit_1 = first.draw_bounded_u64(&hit, 12).unwrap();
        let expected_hit_2 = first.draw_bounded_u64(&hit, 12).unwrap();

        let mut repeated = ring.start(&key(1), &domain, inputs()).unwrap();
        assert_eq!(repeated.draw_bounded_u64(&hit, 12).unwrap(), expected_hit_1);
        repeated.draw_u32(&flavor).unwrap();
        assert_eq!(repeated.draw_bounded_u64(&hit, 12).unwrap(), expected_hit_2);
        assert_ne!(
            ring.start(&key(2), &domain, inputs())
                .unwrap()
                .draw_bounded_u64(&hit, 12)
                .unwrap(),
            expected_hit_1,
            "fixture keys should demonstrate idempotency-domain separation"
        );
    }

    #[test]
    fn retained_secret_replays_exact_version_and_debug_redacts_keys() {
        let ring = ring();
        let domain = CanonicalId::new("attack").unwrap();
        let label = CanonicalId::new("damage").unwrap();
        let old = ring
            .resume(1, &key(1), &domain, inputs())
            .unwrap()
            .draw_u32(&label)
            .unwrap();
        let repeated = ring
            .resume(1, &key(1), &domain, inputs())
            .unwrap()
            .draw_u32(&label)
            .unwrap();
        assert_eq!(old, repeated);
        assert!(matches!(
            ring.resume(99, &key(1), &domain, inputs()),
            Err(ResolutionRngError::UnknownSecretVersion { version: 99 })
        ));
        let debug = format!("{ring:?}");
        assert!(!debug.contains("34, 34"));
        assert!(!debug.contains("17, 17"));
    }

    #[test]
    fn draw_and_stream_caps_fail_closed() {
        let ring = ring();
        let domain = CanonicalId::new("attack").unwrap();
        let mut rng = ring.start(&key(1), &domain, inputs()).unwrap();
        let label = CanonicalId::new("roll").unwrap();
        for _ in 0..MAX_RESOLUTION_DRAWS {
            rng.draw_u32(&label).unwrap();
        }
        assert_eq!(
            rng.draw_u32(&label).unwrap_err(),
            ResolutionRngError::DrawLimit {
                max: MAX_RESOLUTION_DRAWS
            }
        );

        let mut rng = ring.start(&key(1), &domain, inputs()).unwrap();
        for index in 0..MAX_RESOLUTION_STREAMS {
            rng.draw_u32(&CanonicalId::new(format!("stream_{index}")).unwrap())
                .unwrap();
        }
        assert_eq!(
            rng.draw_u32(&CanonicalId::new("one_more").unwrap())
                .unwrap_err(),
            ResolutionRngError::StreamLimit {
                max: MAX_RESOLUTION_STREAMS
            }
        );
    }

    #[test]
    fn resolution_record_roundtrips_and_rejects_tampering() {
        let ring = ring();
        let domain = CanonicalId::new("attack").unwrap();
        let mut rng = ring.start(&key(1), &domain, inputs()).unwrap();
        rng.draw_bounded_u64(&CanonicalId::new("damage").unwrap(), 6)
            .unwrap();
        let schema = definition("result");
        let record = rng
            .into_prepared(
                vec![definition("attack"), schema.clone()],
                RulesValue::new(schema.clone(), json!({ "pool": 7 })).unwrap(),
                RulesValue::new(schema, json!({ "successes": 3 })).unwrap(),
            )
            .unwrap()
            .bind(CommitToken::from_bytes([0x77; 16]));
        let restored: ResolutionRecord =
            serde_json::from_slice(&serde_json::to_vec(&record).unwrap()).unwrap();
        assert_eq!(restored, record);

        let mut value = serde_json::to_value(&record).unwrap();
        value["draws"][0]["ordinal"] = json!(7);
        assert!(serde_json::from_value::<ResolutionRecord>(value).is_err());
        let mut value = serde_json::to_value(&record).unwrap();
        let upper = value["draws"][0]["draw"]["upper_exclusive"].clone();
        value["draws"][0]["draw"]["value"] = upper;
        assert!(serde_json::from_value::<ResolutionRecord>(value).is_err());
    }
}
