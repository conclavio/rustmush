//! Self-contained canonical profile custody before per-arena deduplication.

use super::core::{CoreClass, CoreProfile};
use super::profile_rules::BoundProgram;
use super::runtime_card::{CardCatalog, CardDefinition, RuntimeCard, RuntimeProfiles};
use super::schema::{SchemaError, validate_envelope};
use super::state::Channel;
use mush_core::{DefinitionDigest, ObjId};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use smol_str::SmolStr;

/// Immutable certificate identity carried by a filed seal. These values are
/// evidence, not a licence or a trusted Registry result. Only the world-side
/// owner reader can authorize their use; the arena checks internal coherence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FiledCertificate {
    pub revision_character: ObjId,
    pub revision_mind: u64,
    pub revision_number: u64,
    pub revision_digest: DefinitionDigest,
    pub source_digest: DefinitionDigest,
    /// Durable receipt key, distinct from the measured certificate content ID.
    pub rating_job: mush_core::CommitToken,
    pub rating_id: DefinitionDigest,
    pub rating_schema: u16,
    pub rating_digest: DefinitionDigest,
    pub compatibility: DefinitionDigest,
    pub definition: DefinitionDigest,
    pub declared: CoreClass,
    pub minimum: CoreClass,
}
impl FiledCertificate {
    /// Checks the immutable shape, without reading workflow or interpreter state.
    pub fn validate(&self) -> Result<(), SchemaError> {
        if self.rating_schema == 0
            || self.revision_mind == 0
            || self.revision_number == 0
            || self.declared == CoreClass::Zero
            || self.minimum < self.declared
            || [
                self.revision_digest,
                self.source_digest,
                self.rating_id,
                self.rating_digest,
                self.compatibility,
                self.definition,
            ]
            .iter()
            .any(|digest| digest.as_bytes() == &[0; 32])
        {
            return Err(invalid("certificate", "invalid filed certificate binding"));
        }
        Ok(())
    }

    pub fn verify_definition(&self, definition: &CardDefinition) -> Result<(), SchemaError> {
        self.validate()?;
        if definition.declared_class != self.declared || definition.validate()? != self.definition {
            return Err(invalid(
                "certificate",
                "filed definition differs from its certificate",
            ));
        }
        Ok(())
    }
}

/// Workflow-neutral immutable provenance; personal drafts invent no charter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MindOrigin {
    /// Registry stock source version.
    Doctrine { revision: u32 },
    /// Saved owned revision, resolved exactly once before checkout.
    Draft {
        character: ObjId,
        mind: u64,
        revision: u64,
        digest: DefinitionDigest,
    },
    /// M7 filed execution identity; personal drafts never use this variant.
    Filed {
        charter: SmolStr,
        line: SmolStr,
        revision: u64,
        licence: SmolStr,
        certificate: FiledCertificate,
    },
}
impl MindOrigin {
    #[must_use]
    pub fn certificate(&self) -> Option<&FiledCertificate> {
        match self {
            Self::Filed { certificate, .. } => Some(certificate),
            Self::Doctrine { .. } | Self::Draft { .. } => None,
        }
    }
    #[must_use]
    pub const fn is_doctrine(&self) -> bool {
        matches!(self, Self::Doctrine { .. })
    }
    /// A concise public custody label; a draft has no licence declaration.
    #[must_use]
    pub fn licence_label(&self) -> &str {
        match self {
            Self::Doctrine { .. } => "doctrine",
            Self::Draft { .. } => "personal draft",
            Self::Filed { licence, .. } => licence,
        }
    }
    /// Shape validation at seal/decode, independent of workflow permissions.
    /// # Errors
    /// Empty/exhausted identities or unbounded display words.
    pub fn validate(&self) -> Result<(), SchemaError> {
        let bad = match self {
            Self::Doctrine { revision } => *revision == 0,
            Self::Draft {
                mind,
                revision,
                digest,
                ..
            } => *mind == 0 || *revision == 0 || digest.as_bytes() == &[0; 32],
            Self::Filed {
                charter,
                line,
                revision,
                licence,
                certificate,
            } => {
                certificate.validate()?;
                *revision == 0
                    || [charter, line, licence]
                        .iter()
                        .any(|v| v.is_empty() || v.len() > 128 || v.chars().any(char::is_control))
            }
        };
        if bad {
            Err(invalid("origin", "invalid immutable origin"))
        } else {
            Ok(())
        }
    }
}

/// Complete execution identity. Display names never resolve mutable state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MindIdentity {
    pub mind: SmolStr,
    pub origin: MindOrigin,
    #[serde(with = "crate::arena::snapshot::digest_serde")]
    pub source_sha256: [u8; 32],
}
impl MindIdentity {
    pub fn validate(&self) -> Result<(), SchemaError> {
        self.origin.validate()?;
        if self.mind.is_empty()
            || self.mind.len() > 128
            || self.mind.chars().any(char::is_control)
            || self.source_sha256 == [0; 32]
        {
            return Err(invalid("identity", "invalid immutable mind identity"));
        }
        if self
            .origin
            .certificate()
            .is_some_and(|certificate| certificate.source_digest.as_bytes() != &self.source_sha256)
        {
            return Err(invalid(
                "certificate",
                "filed source differs from its certificate",
            ));
        }
        Ok(())
    }
    #[must_use]
    pub fn is_doctrine(&self) -> bool {
        self.origin.is_doctrine()
    }
}

/// Full values/schema/programs for world custody, plus exact selected order.
/// Arena admission deduplicates definition and keeps only the compact bindings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SealedCards {
    pub definition: CardDefinition,
    pub selected: RuntimeProfiles,
    /// Canonical full first-profile payload. Keep byte-array wire representation.
    pub opening_card: Vec<u8>,
    #[serde(with = "crate::arena::snapshot::digest_serde")]
    pub opening_digest: [u8; 32],
    /// Bound native + think programs in the selected profile order.
    pub programs: Vec<BoundProgram>,
}
impl SealedCards {
    /// Construction-only invalid marker, never accepted for a live hull.
    #[must_use]
    pub fn placeholder() -> Self {
        Self {
            definition: CardDefinition {
                schema: super::schema::CardSchema {
                    sections: Vec::new(),
                },
                declared_class: super::core::CoreClass::Zero,
                profiles: Vec::new(),
            },
            selected: RuntimeProfiles::unbound(),
            opening_card: Vec::new(),
            opening_digest: [0; 32],
            programs: Vec::new(),
        }
    }
    /// Validate all selected complete cards against the actual class and hull.
    /// This is shared by selection preview and checkout; neither clamps values.
    /// # Errors
    /// Schema/card/floor/symbol/capacity errors, before any visible publication.
    pub fn prepare(
        definition: CardDefinition,
        names: &[SmolStr],
        profile: &CoreProfile,
        weapons: &[super::rules::RuleWeapon],
        channels: &[Channel],
        floor: u32,
    ) -> Result<Self, SchemaError> {
        let weapon_ids = weapons.iter().map(|w| w.id.clone()).collect::<Vec<_>>();
        let mut catalog = CardCatalog::default();
        let digest = catalog.admit(definition.clone())?;
        let selected = RuntimeProfiles::bind(&catalog, digest, names)?;
        selected.validate_for(&catalog, profile)?;
        let mut selected_values = std::collections::BTreeMap::new();
        let mut selected_rules = std::collections::BTreeMap::new();
        let mut programs = Vec::new();
        for name in selected.names() {
            let row = definition
                .profiles
                .iter()
                .find(|p| p.name == name)
                .ok_or_else(|| invalid("profiles", "missing selected profile"))?;
            selected_values.insert(row.name.clone(), row.values.clone());
            selected_rules.insert(row.name.clone(), row.rules.clone());
            let program = BoundProgram::bind(&row.rules, &weapon_ids, channels, profile, floor)?;
            for row in program.rows.iter().filter(|row| !row.inert) {
                for predicate in row.program.rule.predicates() {
                    if let super::conditions::RulePredicate::Rounds {
                        weapon,
                        amount,
                        percent: false,
                        ..
                    } = predicate
                    {
                        let selected = weapon
                            .as_ref()
                            .or_else(|| row.program.rule.weapon())
                            .or_else(|| weapons.first().map(|w| &w.id));
                        let maximum = selected
                            .and_then(|id| weapons.iter().find(|w| &w.id == id))
                            .map(|w| w.rounds_max)
                            .ok_or_else(|| invalid("rounds", "missing bound carried weapon"))?;
                        if *amount > maximum {
                            return Err(invalid(
                                "rounds",
                                "absolute threshold exceeds the weapon's full load",
                            ));
                        }
                    }
                }
            }
            programs.push(program);
        }
        let checked = validate_envelope(
            &json(&definition.schema)?,
            &serde_json::json!({}),
            &json(&selected_values)?,
            &json(&selected_rules)?,
            definition.declared_class,
        )?;
        if channels.contains(&Channel::Fire) {
            checked.validate_floor(
                u16::try_from(floor).map_err(|_| invalid("floor", "invalid Compact floor"))?,
            )?;
            for index in 0..selected.len() {
                let key = selected
                    .key(u8::try_from(index).map_err(|_| invalid("profiles", "invalid index"))?)
                    .ok_or_else(|| invalid("profiles", "missing profile"))?;
                if catalog
                    .resolve(key)?
                    .stock()
                    .is_some_and(|card| u32::from(card.min_odds_permille) < floor)
                {
                    return Err(invalid("min_odds", "below the Compact floor"));
                }
            }
        }
        let first = selected
            .key(0)
            .ok_or_else(|| invalid("profiles", "missing opening profile"))?;
        let active = catalog.resolve(first)?;
        let opening_card = active
            .canonical_bytes()
            .ok_or_else(|| invalid("card", "not hydrated"))?
            .to_vec();
        let opening_digest = Sha256::digest(&opening_card).into();
        Ok(Self {
            definition,
            selected,
            opening_card,
            opening_digest,
            programs,
        })
    }

    /// Full decode check; digest/name bytes alone never reconstruct values.
    /// # Errors
    /// Every admission error or mismatch against the complete recomputed seal.
    pub fn verify(
        &self,
        profile: &CoreProfile,
        weapons: &[super::rules::RuleWeapon],
        channels: &[Channel],
        floor: u32,
    ) -> Result<(), SchemaError> {
        let names = self.selected.names().map(SmolStr::new).collect::<Vec<_>>();
        let expected = Self::prepare(
            self.definition.clone(),
            &names,
            profile,
            weapons,
            channels,
            floor,
        )?;
        if self != &expected {
            return Err(invalid(
                "sealed cards",
                "payload, bindings or digest differ",
            ));
        }
        Ok(())
    }

    /// Admit to an arena-local staged catalog, then resolve the exact opening.
    /// Caller publishes the staged aggregate only after all hulls and the whole
    /// snapshot byte cap pass. Returns the selected binding set and active card.
    /// # Errors
    /// Missing/tampered definition or bounded catalog admission failure.
    pub fn admit(
        &self,
        catalog: &mut CardCatalog,
    ) -> Result<(RuntimeProfiles, RuntimeCard), SchemaError> {
        // Do not mutate the caller on a cross-reference failure.
        let mut staged = catalog.clone();
        let digest = staged.admit(self.definition.clone())?;
        let names = self.selected.names().map(SmolStr::new).collect::<Vec<_>>();
        let bindings = RuntimeProfiles::bind(&staged, digest, &names)?;
        if bindings != self.selected {
            return Err(invalid("sealed cards", "binding mismatch"));
        }
        let active = staged.resolve(
            bindings
                .key(0)
                .ok_or_else(|| invalid("profiles", "missing opening"))?,
        )?;
        let bytes = active
            .canonical_bytes()
            .ok_or_else(|| invalid("card", "not hydrated"))?;
        if bytes != self.opening_card
            || <[u8; 32]>::from(Sha256::digest(bytes)) != self.opening_digest
        {
            return Err(invalid("sealed cards", "opening payload mismatch"));
        }
        *catalog = staged;
        Ok((bindings, active))
    }
}
fn json(v: &impl Serialize) -> Result<serde_json::Value, SchemaError> {
    serde_json::to_value(v).map_err(|_| invalid("card", "cannot serialize values"))
}
fn invalid(field: impl Into<String>, message: impl Into<String>) -> SchemaError {
    SchemaError {
        field: field.into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::conditions::{
        ASPECTS_ANY, Comparison, FireCondition, Rule, RulePredicate, SlotTarget,
    };
    use super::super::core::{ClassTable, CoreClass};
    use super::super::rules::RuleWeapon;
    use super::super::runtime_card::ProfileDefinition;
    use super::super::schema::{AuthoredRule, CardSchema};
    use super::*;
    fn definition() -> CardDefinition {
        CardDefinition {
            schema: CardSchema { sections: vec![] },
            declared_class: CoreClass::III,
            profiles: vec![ProfileDefinition {
                name: "Default".into(),
                values: Default::default(),
                rules: vec![AuthoredRule {
                    rule: Rule::fire_rule(
                        FireCondition {
                            weapon: "all".into(),
                            target: SlotTarget::Lock,
                            min_permille: 100,
                            aspects: ASPECTS_ANY,
                            min_rounds_left: 0,
                            min_range_m: 0,
                            max_range_m: u32::MAX,
                        },
                        vec![RulePredicate::Rounds {
                            weapon: None,
                            comparison: Comparison::Greater,
                            amount: 40,
                            percent: false,
                        }],
                    ),
                    think: vec![],
                }],
            }],
        }
    }
    #[test]
    fn m6_seal_binds_all_and_checks_each_actual_magazine_atomically() {
        let weapons = vec![
            RuleWeapon {
                id: "cannon".into(),
                rounds_max: 41,
            },
            RuleWeapon {
                id: "mg".into(),
                rounds_max: 300,
            },
        ];
        let seal = SealedCards::prepare(
            definition(),
            &[],
            &ClassTable::STOCK.row(CoreClass::III),
            &weapons,
            &Channel::ALL,
            100,
        )
        .unwrap();
        assert_eq!(seal.programs[0].rows.len(), 2);
        seal.verify(
            &ClassTable::STOCK.row(CoreClass::III),
            &weapons,
            &Channel::ALL,
            100,
        )
        .unwrap();
        let mut smaller = weapons.clone();
        smaller[1].rounds_max = 30;
        assert!(
            SealedCards::prepare(
                definition(),
                &[],
                &ClassTable::STOCK.row(CoreClass::III),
                &smaller,
                &Channel::ALL,
                100
            )
            .is_err()
        );
        assert!(
            seal.verify(
                &ClassTable::STOCK.row(CoreClass::III),
                &smaller,
                &Channel::ALL,
                100
            )
            .is_err()
        );
    }
    #[test]
    fn m6_missing_fire_station_keeps_symbolic_rule_readable_and_inert() {
        let seal = SealedCards::prepare(
            definition(),
            &[],
            &ClassTable::STOCK.row(CoreClass::III),
            &[],
            &[Channel::Helm, Channel::Sensors],
            100,
        )
        .unwrap();
        assert_eq!(seal.programs[0].rows.len(), 1);
        assert!(seal.programs[0].rows[0].inert);
        assert_eq!(
            seal.programs[0].rows[0].program.rule.weapon().unwrap(),
            "all"
        );
    }
}
