//! Immutable execution custody and compact per-hull references.
use super::core::{ClassTable, CoreClass};
use super::mind::{SealEngine, SortieId};
use super::profile_seal::{MindIdentity, MindOrigin, SealedCards};
use super::runtime_card::{CardCatalog, ProfileKey, RuntimeProfiles};
use super::schema::SchemaError;
use super::state::Channel;
use mush_core::ObjId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SealRef {
    pub sortie: SortieId,
    pub digest: [u8; 32],
    pub core_class: CoreClass,
    pub class: CoreClass,
    pub engine: SealEngine,
    pub governed: bool,
    pub closed: bool,
    pub doctrine: bool,
    /// Own-hull selected name, retained as provenance when execution falls to ROM.
    pub name: SmolStr,
    /// Exact selected catalog keys; never reconstructed from preset names.
    pub profiles: RuntimeProfiles,
    /// Frozen carried component/ammunition and station facts from initial custody.
    pub weapons: Vec<super::rules::RuleWeapon>,
    pub channels: Vec<Channel>,
    /// Immutable ROM binding, checked against mutable runtime on every restore.
    pub rom: ProfileKey,
}
impl SealRef {
    #[must_use]
    pub fn none() -> Self {
        Self {
            sortie: SortieId::none(),
            digest: [0; 32],
            core_class: CoreClass::Zero,
            class: CoreClass::Zero,
            engine: SealEngine::Reflex,
            governed: true,
            closed: false,
            doctrine: false,
            name: SmolStr::default(),
            profiles: RuntimeProfiles::unbound(),
            weapons: Vec::new(),
            channels: Vec::new(),
            rom: invalid_key(),
        }
    }
}
impl Default for SealRef {
    fn default() -> Self {
        Self::none()
    }
}

/// Self-contained custody: requested provenance, effective executable and ROM.
/// Main execution fields describe what actually runs, including initial ROM.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MindSeal {
    pub unit: ObjId,
    pub sortie: SortieId,
    pub core: Option<SmolStr>,
    pub core_class: CoreClass,
    pub class: CoreClass,
    pub governed: bool,
    pub mind: SmolStr,
    pub origin: MindOrigin,
    #[serde(with = "crate::arena::snapshot::digest_serde")]
    pub source_sha256: [u8; 32],
    /// The exact selected revision, even when effective execution is ROM.
    pub requested: MindIdentity,
    /// Required byte-array representation of the complete opening card.
    pub card: Vec<u8>,
    #[serde(with = "crate::arena::snapshot::digest_serde")]
    pub card_digest: [u8; 32],
    pub abi: SmolStr,
    pub principal: Option<crate::deliberation::order::CharacterRef>,
    pub channels: Vec<Channel>,
    pub weapons: Vec<super::rules::RuleWeapon>,
    pub odds_floor_permille: u32,
    pub fallback: Option<SmolStr>,
    pub engine: SealEngine,
    /// Complete immutable effective profiles and programs.
    pub cards: SealedCards,
    /// Server-owned frozen ROM source and lawful full24 card.
    pub rom_identity: MindIdentity,
    pub rom: SealedCards,
}
impl MindSeal {
    #[must_use]
    pub fn identity(&self) -> MindIdentity {
        MindIdentity {
            mind: self.mind.clone(),
            origin: self.origin.clone(),
            source_sha256: self.source_sha256,
        }
    }
    #[must_use]
    pub fn digest(&self) -> [u8; 32] {
        use sha2::{Digest as _, Sha256};
        Sha256::digest(serde_json::to_vec(self).unwrap_or_default()).into()
    }
    #[must_use]
    pub fn seal_ref(&self) -> SealRef {
        SealRef {
            sortie: self.sortie,
            digest: self.digest(),
            core_class: self.core_class,
            class: self.class,
            engine: self.engine,
            governed: self.governed,
            closed: false,
            doctrine: self.origin.is_doctrine(),
            name: self.requested.mind.clone(),
            profiles: self.profile_set(),
            weapons: self.weapons.clone(),
            channels: self.channels.clone(),
            rom: self.rom.selected.key(0).unwrap_or_else(invalid_key),
        }
    }
    #[must_use]
    pub fn profile_set(&self) -> RuntimeProfiles {
        self.cards.selected.clone()
    }
    /// **The seal's `mind_sealed` line** (17b cut M3.3; doc 17 §13). `licence_state` is the
    /// state the licence is read under at the site that seals (doc 17h cut M8a, A1): a
    /// parameter, never a field of the seal, so [`MindSeal::digest`] and every record keyed on
    /// it stand whatever the holding does later.
    #[must_use]
    pub fn sealed_line(
        &self,
        licence_state: crate::minds::licence_state::SealLicenceState,
    ) -> super::ledger::EventKind {
        let source = super::snapshot::digest_text(&self.source_sha256);
        super::ledger::EventKind::MindSealed {
            unit: self.unit,
            sortie: self.sortie,
            mind: self.mind.clone(),
            source: SmolStr::new(&source[..16]),
            card_digest: super::snapshot::digest_text(&self.card_digest).into(),
            class: self.class,
            governed: self.governed,
            licence: self.origin.licence_label().into(),
            licence_state: Some(licence_state),
            principal: self.principal.clone(),
        }
    }
    /// Validate complete custody before any arena catalog/source publication.
    pub fn verify(
        &self,
        classes: &ClassTable,
        weapons: &[super::rules::RuleWeapon],
    ) -> Result<(), SchemaError> {
        if self.weapons != weapons {
            return Err(invalid("captured weapon facts differ from custody"));
        }
        self.identity().validate()?;
        self.requested.validate()?;
        self.rom_identity.validate()?;
        if let Some(certificate) = self.requested.origin.certificate()
            && self.class != CoreClass::Zero
        {
            if self.identity() != self.requested
                || self.engine != SealEngine::Scripted
                || self.class != certificate.minimum
                || self.core_class < certificate.minimum
            {
                return Err(invalid(
                    "filed execution differs from its certified identity or class",
                ));
            }
            certificate.verify_definition(&self.cards.definition)?;
        }
        if self.origin.certificate().is_some() && self.identity() != self.requested {
            return Err(invalid(
                "filed execution lacks its exact requested certificate",
            ));
        }
        if self.unit == ObjId(0)
            || self.odds_floor_permille > 1000
            || self.card != self.cards.opening_card
            || self.card_digest != self.cards.opening_digest
            || !self.rom_identity.is_doctrine()
            || self.engine == SealEngine::Reflex
        {
            return Err(invalid("invalid seal identity, opening bytes or engine"));
        }
        let expected_channels = if weapons.is_empty() {
            vec![Channel::Helm, Channel::Sensors]
        } else {
            vec![Channel::Helm, Channel::Sensors, Channel::Fire]
        };
        if self.channels != expected_channels {
            return Err(invalid("sealed stations do not match the hull"));
        }
        self.cards.verify(
            &classes.row(self.class),
            weapons,
            &self.channels,
            self.odds_floor_permille,
        )?;
        self.rom.verify(
            &classes.row(CoreClass::Zero),
            weapons,
            &self.channels,
            self.odds_floor_permille,
        )?;
        if self.rom.definition.declared_class != CoreClass::Zero
            || !self.rom.programs.iter().all(|p| p.rows.is_empty())
        {
            return Err(invalid("ROM is not the captured class0 stock program"));
        }
        let mut catalog = CardCatalog::default();
        let (_, rom_card) = self.rom.admit(&mut catalog)?;
        if rom_card.stock().is_none() {
            return Err(invalid("ROM lacks complete Standard values"));
        }
        if self.class == CoreClass::Zero
            && (self.engine != SealEngine::Native
                || self.identity() != self.rom_identity
                || self.cards != self.rom)
        {
            return Err(invalid("class0 execution must be the captured native ROM"));
        }
        Ok(())
    }
    #[must_use]
    pub fn placeholder() -> Self {
        let identity = MindIdentity {
            mind: SmolStr::default(),
            origin: MindOrigin::Doctrine { revision: 0 },
            source_sha256: [0; 32],
        };
        Self {
            unit: ObjId(0),
            sortie: SortieId::none(),
            core: None,
            core_class: CoreClass::Zero,
            class: CoreClass::Zero,
            governed: true,
            mind: SmolStr::default(),
            origin: identity.origin.clone(),
            source_sha256: [0; 32],
            requested: identity.clone(),
            card: Vec::new(),
            card_digest: [0; 32],
            abi: SmolStr::default(),
            principal: None,
            channels: Vec::new(),
            weapons: Vec::new(),
            odds_floor_permille: 0,
            fallback: None,
            engine: SealEngine::Reflex,
            cards: SealedCards::placeholder(),
            rom_identity: identity,
            rom: SealedCards::placeholder(),
        }
    }
}
fn invalid(message: &str) -> SchemaError {
    SchemaError {
        field: "seal".into(),
        message: message.into(),
    }
}

fn invalid_key() -> ProfileKey {
    ProfileKey {
        definition: mush_core::DefinitionDigest::from_bytes([0; 32]),
        index: 0,
    }
}
