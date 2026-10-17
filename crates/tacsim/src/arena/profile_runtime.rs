//! Atomic active-card/program selection and immutable pending-think tokens.

use super::conditions::{ConditionTable, PreparedProfileRules, Rule};
use super::core::CoreProfile;
use super::profile_rules::{BoundProgram, RuleLease};
use super::runtime_card::{CardCatalog, ProfileKey, RuntimeCard, RuntimeProfiles};
use super::schema::SchemaError;
use super::state::Channel;
use mush_core::ObjId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

/// Pending answers must match both generation and exact immutable card identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileToken {
    pub unit: ObjId,
    pub generation: u64,
    pub key: ProfileKey,
}

/// One hull's carried runtime card/program/sample state. Canonical definitions
/// live in the arena catalog. No field is reconstructed from current presets.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileRuntime {
    pub unit: ObjId,
    pub weapons: Vec<SmolStr>,
    pub channels: Vec<Channel>,
    pub profiles: RuntimeProfiles,
    pub index: u8,
    pub switched_ms: Option<i64>,
    pub generation: u64,
    pub card: RuntimeCard,
    pub program: BoundProgram,
    pub lease: RuleLease,
    /// A pilot manually replacing/removing a slot overrides its sampled binding.
    /// Profile switch clears the whole set. At most six bits may be set.
    pub overridden: u8,
    /// Captured Standard ROM card, hydrated from the same immutable catalog.
    pub rom: RuntimeCard,
    pub dropped: bool,
    /// Explicit staff diagnostic override; ordinary profile switches clear it.
    pub diagnostic_override: bool,
}

/// Opaque ready-to-publish switch; commit rechecks owner and generation first.
#[derive(Clone, Debug)]
pub struct PreparedSwitch {
    expected: ProfileToken,
    next: ProfileRuntime,
    rules: PreparedProfileRules,
}

impl ProfileRuntime {
    /// Initial state after validated custody cards and explicit ROM admission.
    /// # Errors
    /// Missing keys/capacity or an invalid bound profile.
    #[allow(clippy::too_many_arguments)] // Explicit immutable custody plus captured hull facts at admission.
    pub fn new(
        catalog: &CardCatalog,
        unit: ObjId,
        profiles: RuntimeProfiles,
        rom: RuntimeCard,
        profile: &CoreProfile,
        weapons: &[SmolStr],
        channels: &[Channel],
        floor: u32,
    ) -> Result<Self, SchemaError> {
        profiles.validate_for(catalog, profile)?;
        let key = profiles
            .key(0)
            .ok_or_else(|| invalid("profiles", "missing opening"))?;
        let card = catalog.resolve(key)?;
        let program = bind(&card, profile, weapons, channels, floor)?;
        validate_rom(catalog, &rom, channels, floor)?;
        Ok(Self {
            unit,
            weapons: weapons.to_vec(),
            channels: channels.to_vec(),
            profiles,
            index: 0,
            switched_ms: None,
            generation: 0,
            card,
            program,
            lease: RuleLease::default(),
            overridden: 0,
            rom,
            dropped: false,
            diagnostic_override: false,
        })
    }
    #[must_use]
    pub fn token(&self) -> ProfileToken {
        ProfileToken {
            unit: self.unit,
            generation: self.generation,
            key: self.card.key(),
        }
    }
    #[must_use]
    pub fn accepts(&self, token: ProfileToken) -> bool {
        self.token() == token
    }

    /// Validate, allocate and bind before touching card, slots or dormant memory.
    /// Caller checks authenticated Helm seat authority, captures event hash, then
    /// commits synchronously. Memory is not part of this operation and stays byte
    /// identical. Pending results with the prior token cannot subsequently land.
    /// # Errors
    /// Same/missing profile, 10s cooldown, exhausted generation, cap/card/rules.
    #[allow(clippy::too_many_arguments)] // Rechecks captured hull facts and authority before staging a switch.
    pub fn prepare_switch(
        &self,
        catalog: &CardCatalog,
        unit: ObjId,
        index: u8,
        t_ms: i64,
        profile: &CoreProfile,
        weapons: &[SmolStr],
        channels: &[Channel],
        floor: u32,
    ) -> Result<PreparedSwitch, SchemaError> {
        if unit != self.unit
            || weapons != self.weapons
            || channels != self.channels
            || self.dropped
            || index == self.index
            || self
                .switched_ms
                .is_some_and(|at| t_ms < at.saturating_add(10_000))
        {
            return Err(invalid(
                "profile",
                "profile is unchanged, ROM is active, or ten-second cooldown has not elapsed",
            ));
        }
        self.profiles.validate_for(catalog, profile)?;
        let key = self
            .profiles
            .key(index)
            .ok_or_else(|| invalid("profile", "profile not sealed"))?;
        let card = catalog.resolve(key)?;
        let program = bind(&card, profile, weapons, channels, floor)?;
        let rules =
            ConditionTable::prepare_profile_rules(unit, &program.native_rows(), profile.slots)
                .map_err(|_| invalid("rules", "cannot prepare profile bindings"))?;
        let generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| invalid("profile", "generation exhausted"))?;
        Ok(PreparedSwitch {
            expected: self.token(),
            next: Self {
                unit: self.unit,
                weapons: self.weapons.clone(),
                channels: self.channels.clone(),
                profiles: self.profiles.clone(),
                index,
                switched_ms: Some(t_ms),
                generation,
                card,
                program,
                lease: RuleLease::default(),
                overridden: 0,
                rom: self.rom.clone(),
                dropped: false,
                diagnostic_override: false,
            },
            rules,
        })
    }
    /// Commit a prepared change with its complete native table in one synchronous
    /// arena transition. Owner/generation refusal happens before either write.
    pub fn commit(
        &mut self,
        conditions: &mut ConditionTable,
        prepared: PreparedSwitch,
    ) -> Result<(), SchemaError> {
        if self.token() != prepared.expected {
            return Err(invalid("profile", "prepared profile change is stale"));
        }
        conditions.apply_profile_rules(prepared.rules);
        *self = prepared.next;
        Ok(())
    }
    /// Prepare a permanent drop to the captured ROM, never current defaults.
    /// Caller journals the drop and applies the existing ROM memory policy.
    /// # Errors
    /// ROM payload/capacity invalid or generation exhausted.
    pub fn prepare_rom(
        &self,
        unit: ObjId,
        rom_profile: &CoreProfile,
        weapons: &[SmolStr],
        channels: &[Channel],
        floor: u32,
    ) -> Result<PreparedSwitch, SchemaError> {
        if unit != self.unit
            || weapons != self.weapons
            || channels != self.channels
            || rom_profile.class != super::core::CoreClass::Zero
            || self.rom.stock().is_none()
        {
            return Err(invalid(
                "ROM",
                "requires captured Standard values and class zero",
            ));
        }
        let program = bind(&self.rom, rom_profile, weapons, channels, floor)?;
        let rules =
            ConditionTable::prepare_profile_rules(unit, &program.native_rows(), rom_profile.slots)
                .map_err(|_| invalid("ROM", "cannot prepare ROM bindings"))?;
        let generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| invalid("profile", "generation exhausted"))?;
        Ok(PreparedSwitch {
            expected: self.token(),
            next: Self {
                unit: self.unit,
                weapons: self.weapons.clone(),
                channels: self.channels.clone(),
                profiles: self.profiles.clone(),
                index: self.index,
                switched_ms: self.switched_ms,
                generation,
                card: self.rom.clone(),
                program,
                lease: RuleLease::default(),
                overridden: 0,
                rom: self.rom.clone(),
                dropped: true,
                diagnostic_override: false,
            },
            rules,
        })
    }

    /// Staff diagnostic adapter, using the same immutable catalog transaction.
    /// Only complete stock cards can be edited; captured ROM is never overridden.
    #[allow(clippy::too_many_arguments)] // Diagnostic card staging uses the same complete hull binding inputs.
    pub fn prepare_card(
        &self,
        catalog: &CardCatalog,
        unit: ObjId,
        card: RuntimeCard,
        profile: &CoreProfile,
        weapons: &[SmolStr],
        channels: &[Channel],
        floor: u32,
    ) -> Result<PreparedSwitch, SchemaError> {
        if self.unit != unit
            || weapons != self.weapons
            || channels != self.channels
            || self.dropped
            || profile.class == super::core::CoreClass::Zero
            || self.card.stock().is_none()
            || card.stock().is_none()
        {
            return Err(invalid(
                "card",
                "diagnostic edits require an active complete stock card above class0",
            ));
        }
        let resolved = catalog.resolve(card.key())?;
        if resolved != card
            || card
                .stock()
                .is_some_and(|stock| u32::from(stock.min_odds_permille) < floor)
                && channels.contains(&Channel::Fire)
        {
            return Err(invalid(
                "card",
                "diagnostic card does not match the catalog or Compact floor",
            ));
        }
        let program = bind(&resolved, profile, weapons, channels, floor)?;
        let rules =
            ConditionTable::prepare_profile_rules(unit, &program.native_rows(), profile.slots)
                .map_err(|_| invalid("card", "diagnostic rules cannot replace slots"))?;
        let generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| invalid("profile", "generation exhausted"))?;
        let mut next = self.clone();
        next.card = resolved;
        next.program = program;
        next.generation = generation;
        next.lease.clear();
        next.overridden = 0;
        next.diagnostic_override = true;
        Ok(PreparedSwitch {
            expected: self.token(),
            next,
            rules,
        })
    }

    /// Record a successful manual slot change; it has ordinary seat authority.
    /// Invalidation prevents an identical-looking later row inheriting old gates.
    pub fn override_slot(&mut self, slot: u8) {
        if let Some(index) = slot.checked_sub(1).filter(|index| *index < 6) {
            self.overridden |= 1 << index;
        }
    }
    /// Controller/seat/core/wreck/fault invalidation never preserves sampled truth.
    pub fn invalidate_sample(&mut self) {
        self.lease.clear();
    }
    /// Native evaluator gate shared by first FIRE, every HOLD veto and LOCK.
    #[must_use]
    pub fn permits(&self, slot: u8, rule: &Rule, t_ms: i64) -> bool {
        let Some(index) = slot.checked_sub(1).filter(|index| *index < 6) else {
            return false;
        };
        if self.overridden & (1 << index) != 0 || usize::from(index) >= self.program.rows.len() {
            return true;
        }
        self.lease
            .permits(&self.program, self.generation, slot, rule, t_ms)
    }
    /// Rebuild skipped card caches and verify all references before publishing a
    /// decoded arena. The caller also checks lease time/token and slot overrides.
    /// # Errors
    /// Missing catalog, stale active key, corrupt program, ROM, or switch stamp.
    #[allow(clippy::too_many_arguments)] // Snapshot hydration validates all carried facts against arena time.
    pub fn hydrate(
        &mut self,
        catalog: &CardCatalog,
        unit: ObjId,
        profile: &CoreProfile,
        rom_profile: &CoreProfile,
        weapons: &[SmolStr],
        channels: &[Channel],
        floor: u32,
        now_ms: i64,
    ) -> Result<(), SchemaError> {
        self.profiles.validate_for(catalog, profile)?;
        if self.unit != unit
            || weapons != self.weapons
            || channels != self.channels
            || rom_profile.class != super::core::CoreClass::Zero
        {
            return Err(invalid("profile", "wrong runtime owner or ROM class"));
        }
        if self.overridden & !0b11_1111 != 0
            || self.switched_ms.is_some_and(|at| at < 0 || at > now_ms)
        {
            return Err(invalid("profile", "invalid override bits or switch time"));
        }
        catalog.hydrate_card(&mut self.card)?;
        catalog.hydrate_card(&mut self.rom)?;
        validate_rom(catalog, &self.rom, channels, floor)?;
        let expected = if self.dropped {
            self.rom.key()
        } else {
            self.profiles
                .key(self.index)
                .ok_or_else(|| invalid("profile", "invalid active index"))?
        };
        if self.diagnostic_override
            && (self.dropped
                || self.card.stock().is_none()
                || profile.class == super::core::CoreClass::Zero)
        {
            return Err(invalid("card", "invalid diagnostic override"));
        }
        if !self.diagnostic_override && expected != self.card.key() {
            return Err(invalid(
                "profile",
                "active card differs from selected or ROM key",
            ));
        }
        let active_profile = if self.dropped { rom_profile } else { profile };
        self.program
            .verify(&bind(&self.card, active_profile, weapons, channels, floor)?)?;
        // The arena validates the lease against the current sampled class and
        // exact scheduled expiry. Original sealed class is only the immutable
        // program admission budget; damage can lower its live sampling rate.
        Ok(())
    }
}
fn validate_rom(
    catalog: &CardCatalog,
    rom: &RuntimeCard,
    channels: &[Channel],
    floor: u32,
) -> Result<(), SchemaError> {
    let definition = catalog
        .definition(rom.key().definition)
        .ok_or_else(|| invalid("ROM", "missing captured definition"))?;
    if definition.declared_class != super::core::CoreClass::Zero
        || definition.profiles.len() != 1
        || rom.key().index != 0
        || rom.rules().is_none_or(|rules| !rules.is_empty())
        || rom.stock().is_none_or(|card| {
            channels.contains(&Channel::Fire) && u32::from(card.min_odds_permille) < floor
        })
    {
        return Err(invalid(
            "ROM",
            "requires complete lawful captured Standard class-zero values",
        ));
    }
    Ok(())
}
fn bind(
    card: &RuntimeCard,
    profile: &CoreProfile,
    weapons: &[SmolStr],
    channels: &[Channel],
    floor: u32,
) -> Result<BoundProgram, SchemaError> {
    if channels.contains(&Channel::Fire)
        && card
            .stock()
            .is_some_and(|stock| u32::from(stock.min_odds_permille) < floor)
    {
        return Err(invalid(
            "card",
            "stock firing floor is below the Compact floor",
        ));
    }
    BoundProgram::bind(
        card.rules()
            .ok_or_else(|| invalid("profile", "unhydrated card"))?,
        weapons,
        channels,
        profile,
        floor,
    )
}
fn invalid(field: impl Into<String>, message: impl Into<String>) -> SchemaError {
    SchemaError {
        field: field.into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::conditions::{ASPECTS_ANY, FireCondition, SlotTarget};
    use super::super::core::{ClassTable, CoreClass};
    use super::super::runtime_card::{CardDefinition, ProfileDefinition, stock_definition};
    use super::super::schema::{
        AuthoredRule, CardField, CardSchema, CardSection, CardValue, Widget,
    };
    use super::*;
    fn fixture() -> (CardCatalog, ProfileRuntime, ConditionTable) {
        fixture_with_floor(100)
    }
    fn fixture_with_floor(floor: u32) -> (CardCatalog, ProfileRuntime, ConditionTable) {
        let rule = AuthoredRule {
            rule: Rule::fire_rule(
                FireCondition {
                    weapon: "cannon".into(),
                    target: SlotTarget::Lock,
                    min_permille: u16::try_from(floor).expect("fixture floor fits permille"),
                    aspects: ASPECTS_ANY,
                    min_rounds_left: 0,
                    min_range_m: 0,
                    max_range_m: u32::MAX,
                },
                vec![],
            ),
            think: vec![],
        };
        let definition = CardDefinition {
            schema: CardSchema {
                sections: vec![CardSection {
                    id: "helm".into(),
                    label: "Helm".into(),
                    station: Channel::Helm,
                    fields: vec![CardField {
                        id: "enabled".into(),
                        label: "Enabled".into(),
                        widget: Widget::Toggle {
                            default: true,
                            help: "".into(),
                        },
                    }],
                }],
            },
            declared_class: CoreClass::I,
            profiles: vec![
                ProfileDefinition {
                    name: "One".into(),
                    values: std::collections::BTreeMap::from([(
                        "enabled".into(),
                        CardValue::Boolean(true),
                    )]),
                    rules: vec![rule],
                },
                ProfileDefinition {
                    name: "Two".into(),
                    values: std::collections::BTreeMap::from([(
                        "enabled".into(),
                        CardValue::Boolean(false),
                    )]),
                    rules: vec![],
                },
            ],
        };
        let mut catalog = CardCatalog::default();
        let digest = catalog.admit(definition).unwrap();
        let selected =
            RuntimeProfiles::bind(&catalog, digest, &["One".into(), "Two".into()]).unwrap();
        let mut captured = super::super::card::line(&crate::config::ReflexConfig {
            weight_range: 123_456,
            ..crate::config::ReflexConfig::default()
        });
        captured.min_odds_permille = captured
            .min_odds_permille
            .max(u16::try_from(floor).unwrap());
        let rom = catalog
            .admit(stock_definition(&[("ROM".into(), captured)], CoreClass::Zero).unwrap())
            .unwrap();
        let rom = catalog
            .resolve(ProfileKey {
                definition: rom,
                index: 0,
            })
            .unwrap();
        let runtime = ProfileRuntime::new(
            &catalog,
            ObjId(7),
            selected,
            rom,
            &ClassTable::STOCK.row(CoreClass::I),
            &["cannon".into()],
            &Channel::ALL,
            floor,
        )
        .unwrap();
        let mut conditions = ConditionTable::default();
        runtime
            .program
            .install(&mut conditions, ObjId(7), 2)
            .unwrap();
        (catalog, runtime, conditions)
    }
    #[test]
    fn m6_switch_commits_card_and_rules_and_rejects_old_pending_token() {
        let (catalog, mut runtime, mut conditions) = fixture();
        let before = runtime.token();
        assert!(runtime.card.stock().is_none());
        assert!(!conditions.is_empty_for(ObjId(7)));
        let change = runtime
            .prepare_switch(
                &catalog,
                ObjId(7),
                1,
                100,
                &ClassTable::STOCK.row(CoreClass::I),
                &["cannon".into()],
                &Channel::ALL,
                100,
            )
            .unwrap();
        // Preparation cannot expose half a profile.
        assert!(runtime.accepts(before));
        assert!(!conditions.is_empty_for(ObjId(7)));
        runtime.commit(&mut conditions, change).unwrap();
        assert!(!runtime.accepts(before));
        assert_eq!(runtime.generation, 1);
        assert_eq!(
            runtime.card.get("enabled"),
            Some(&CardValue::Boolean(false))
        );
        assert!(conditions.is_empty_for(ObjId(7)));
        assert!(runtime.lease.sample().is_none());
        assert!(
            runtime
                .prepare_switch(
                    &catalog,
                    ObjId(7),
                    0,
                    10_099,
                    &ClassTable::STOCK.row(CoreClass::I),
                    &["cannon".into()],
                    &Channel::ALL,
                    100
                )
                .is_err()
        );
        assert!(
            runtime
                .prepare_switch(
                    &catalog,
                    ObjId(7),
                    0,
                    10_100,
                    &ClassTable::STOCK.row(CoreClass::I),
                    &["cannon".into()],
                    &Channel::ALL,
                    100
                )
                .is_ok()
        );
    }
    #[test]
    fn m6_fault_drop_uses_captured_rom_not_custom_or_current_config() {
        let (catalog, mut runtime, mut conditions) = fixture();
        let before = runtime.token();
        let change = runtime
            .prepare_rom(
                ObjId(7),
                &ClassTable::STOCK.row(CoreClass::Zero),
                &["cannon".into()],
                &Channel::ALL,
                100,
            )
            .unwrap();
        runtime.commit(&mut conditions, change).unwrap();
        assert!(runtime.dropped);
        assert!(!runtime.accepts(before));
        assert!(conditions.is_empty_for(ObjId(7)));
        assert_eq!(runtime.card.stock().unwrap().weight_range, 123_456);
        assert_ne!(
            runtime.card.stock().unwrap().weight_range,
            crate::config::ReflexConfig::default().weight_range
        );
        assert!(
            runtime
                .prepare_switch(
                    &catalog,
                    ObjId(7),
                    1,
                    100_000,
                    &ClassTable::STOCK.row(CoreClass::I),
                    &["cannon".into()],
                    &Channel::ALL,
                    100
                )
                .is_err()
        );
    }
    #[test]
    fn m6_hydration_keeps_original_admission_separate_from_damaged_sample_period() {
        let (catalog, mut runtime, _) = fixture();
        let period = ClassTable::STOCK.row(CoreClass::I).think_ms;
        let sample = super::super::profile_rules::RuleSample {
            program: runtime.program.digest,
            generation: runtime.generation,
            sampled_ms: 0,
            lands_ms: 100,
            expires_ms: 100 + period,
            truth: vec![true; runtime.program.rows.len()],
        };
        runtime
            .lease
            .land(
                Some(sample),
                &runtime.program,
                runtime.generation,
                100,
                100 + period,
            )
            .unwrap();
        let mut restored: ProfileRuntime =
            serde_json::from_slice(&serde_json::to_vec(&runtime).unwrap()).unwrap();
        restored
            .hydrate(
                &catalog,
                ObjId(7),
                &ClassTable::STOCK.row(CoreClass::II),
                &ClassTable::STOCK.row(CoreClass::Zero),
                &["cannon".into()],
                &Channel::ALL,
                100,
                100,
            )
            .unwrap();
        restored
            .lease
            .validate(&restored.program, restored.generation, 100, period)
            .unwrap();
        assert!(
            restored
                .lease
                .validate(
                    &restored.program,
                    restored.generation,
                    100,
                    ClassTable::STOCK.row(CoreClass::II).think_ms
                )
                .is_err()
        );
    }
    #[test]
    fn m6_profile_resume_hydrates_exact_keys_and_refuses_missing_catalog() {
        let (catalog, runtime, _) = fixture();
        let bytes = serde_json::to_vec(&runtime).unwrap();
        let mut resumed: ProfileRuntime = serde_json::from_slice(&bytes).unwrap();
        assert!(!resumed.card.is_hydrated());
        assert!(
            resumed
                .hydrate(
                    &CardCatalog::default(),
                    ObjId(7),
                    &ClassTable::STOCK.row(CoreClass::I),
                    &ClassTable::STOCK.row(CoreClass::Zero),
                    &["cannon".into()],
                    &Channel::ALL,
                    100,
                    0
                )
                .is_err()
        );
        resumed
            .hydrate(
                &catalog,
                ObjId(7),
                &ClassTable::STOCK.row(CoreClass::I),
                &ClassTable::STOCK.row(CoreClass::Zero),
                &["cannon".into()],
                &Channel::ALL,
                100,
                0,
            )
            .unwrap();
        assert_eq!(resumed, runtime);
        assert!(resumed.card.is_hydrated());
    }
    #[test]
    fn m6_dropped_multi_profile_runtime_restores_original_selection_and_rom() {
        let (catalog, mut runtime, mut conditions) = fixture();
        let change = runtime
            .prepare_rom(
                ObjId(7),
                &ClassTable::STOCK.row(CoreClass::Zero),
                &["cannon".into()],
                &Channel::ALL,
                100,
            )
            .unwrap();
        runtime.commit(&mut conditions, change).unwrap();
        assert_eq!(runtime.profiles.len(), 2);
        let mut restored_catalog: CardCatalog =
            serde_json::from_slice(&serde_json::to_vec(&catalog).unwrap()).unwrap();
        restored_catalog.rebuild().unwrap();
        let mut restored: ProfileRuntime =
            serde_json::from_slice(&serde_json::to_vec(&runtime).unwrap()).unwrap();
        restored
            .hydrate(
                &restored_catalog,
                ObjId(7),
                &ClassTable::STOCK.row(CoreClass::I),
                &ClassTable::STOCK.row(CoreClass::Zero),
                &["cannon".into()],
                &Channel::ALL,
                100,
                100,
            )
            .unwrap();
        assert_eq!(restored, runtime);
        assert_eq!(restored.profiles.len(), 2);
        assert_eq!(restored.card.stock().unwrap().weight_range, 123_456);
    }
    #[test]
    fn m6_prepared_switch_refuses_crossed_unit_and_stale_generation_atomically() {
        let (catalog, mut a, mut conditions) = fixture();
        let mut b = a.clone();
        b.unit = ObjId(8);
        let prepare = |runtime: &ProfileRuntime| {
            runtime
                .prepare_switch(
                    &catalog,
                    runtime.unit,
                    1,
                    100,
                    &ClassTable::STOCK.row(CoreClass::I),
                    &["cannon".into()],
                    &Channel::ALL,
                    100,
                )
                .unwrap()
        };
        let for_a = prepare(&a);
        let before_b = b.clone();
        let before_table = serde_json::to_vec(&conditions).unwrap();
        assert!(b.commit(&mut conditions, for_a.clone()).is_err());
        assert_eq!(b, before_b);
        assert_eq!(serde_json::to_vec(&conditions).unwrap(), before_table);
        a.commit(&mut conditions, for_a.clone()).unwrap();
        let after_a = a.clone();
        let after_table = serde_json::to_vec(&conditions).unwrap();
        assert!(a.commit(&mut conditions, for_a).is_err());
        assert_eq!(a, after_a);
        assert_eq!(serde_json::to_vec(&conditions).unwrap(), after_table);
    }
    #[test]
    fn m6_custom_profile_drop_captures_lawful_rom_above_stock_floor() {
        let (mut catalog, mut runtime, mut conditions) = fixture_with_floor(450);
        let rom = catalog
            .admit(
                super::super::runtime_card::rom_definition(
                    &crate::config::ReflexConfig::default(),
                    450,
                )
                .unwrap(),
            )
            .unwrap();
        runtime.rom = catalog
            .resolve(ProfileKey {
                definition: rom,
                index: 0,
            })
            .unwrap();
        // The lawful custom profile has no StockCard adapter. The server-owned
        // ROM separately captures a lawful firing floor at seal time.
        assert!(runtime.card.stock().is_none());
        let change = runtime
            .prepare_rom(
                ObjId(7),
                &ClassTable::STOCK.row(CoreClass::Zero),
                &["cannon".into()],
                &Channel::ALL,
                450,
            )
            .unwrap();
        runtime.commit(&mut conditions, change).unwrap();
        assert_eq!(runtime.card.stock().unwrap().min_odds_permille, 450);
        let mut catalog_back: CardCatalog =
            serde_json::from_slice(&serde_json::to_vec(&catalog).unwrap()).unwrap();
        catalog_back.rebuild().unwrap();
        let mut back: ProfileRuntime =
            serde_json::from_slice(&serde_json::to_vec(&runtime).unwrap()).unwrap();
        back.hydrate(
            &catalog_back,
            ObjId(7),
            &ClassTable::STOCK.row(CoreClass::I),
            &ClassTable::STOCK.row(CoreClass::Zero),
            &["cannon".into()],
            &Channel::ALL,
            450,
            0,
        )
        .unwrap();
        assert_eq!(back.card.canonical_bytes(), runtime.card.canonical_bytes());
        assert_eq!(back.card.stock().unwrap().min_odds_permille, 450);
    }
    #[test]
    fn m6_staff_card_transaction_retains_rom_and_restores_only_with_explicit_override() {
        let (mut catalog, old, _) = fixture();
        let stock = super::super::card::line(&crate::config::ReflexConfig::default());
        let digest = catalog
            .admit(stock_definition(&[("Line".into(), stock.clone())], CoreClass::I).unwrap())
            .unwrap();
        let selected = RuntimeProfiles::bind(&catalog, digest, &[]).unwrap();
        let mut runtime = ProfileRuntime::new(
            &catalog,
            ObjId(7),
            selected,
            old.rom.clone(),
            &ClassTable::STOCK.row(CoreClass::I),
            &["cannon".into()],
            &Channel::ALL,
            100,
        )
        .unwrap();
        let mut changed = stock;
        changed.min_odds_permille = 650;
        let edited = catalog
            .admit(stock_definition(&[("Line".into(), changed)], CoreClass::I).unwrap())
            .unwrap();
        let edited = catalog
            .resolve(ProfileKey {
                definition: edited,
                index: 0,
            })
            .unwrap();
        let pending = runtime.token();
        let prepared = runtime
            .prepare_card(
                &catalog,
                ObjId(7),
                edited,
                &ClassTable::STOCK.row(CoreClass::I),
                &["cannon".into()],
                &Channel::ALL,
                100,
            )
            .unwrap();
        let mut table = ConditionTable::default();
        runtime.commit(&mut table, prepared).unwrap();
        assert!(!runtime.accepts(pending));
        assert!(runtime.diagnostic_override);
        assert_eq!(runtime.rom, old.rom);
        assert_ne!(runtime.profiles.key(0), Some(runtime.card.key()));
        let mut restored: ProfileRuntime =
            serde_json::from_slice(&serde_json::to_vec(&runtime).unwrap()).unwrap();
        restored
            .hydrate(
                &catalog,
                ObjId(7),
                &ClassTable::STOCK.row(CoreClass::I),
                &ClassTable::STOCK.row(CoreClass::Zero),
                &["cannon".into()],
                &Channel::ALL,
                100,
                0,
            )
            .unwrap();
        assert_eq!(restored.card.stock().unwrap().min_odds_permille, 650);
        restored.diagnostic_override = false;
        assert!(
            restored
                .hydrate(
                    &catalog,
                    ObjId(7),
                    &ClassTable::STOCK.row(CoreClass::I),
                    &ClassTable::STOCK.row(CoreClass::Zero),
                    &["cannon".into()],
                    &Channel::ALL,
                    100,
                    0
                )
                .is_err()
        );
    }
}
