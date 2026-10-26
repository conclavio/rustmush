//! Arena ownership of immutable cards, native bindings and sampled rule authority.
//!
//! World custody and authoring workflows stop at this boundary. The arena stores
//! definitions once, binds a complete program per hull, and never resolves a
//! mutable library name during a sortie.

use mush_core::ObjId;
use smol_str::SmolStr;

use super::conditions::{ConditionTrigger, Rule};
use super::core::{ClassTable, CoreClass};
use super::motor::MotorCounters;
use super::profile_rules::RuleSample;
use super::profile_runtime::ProfileRuntime;
use super::profile_seal::SealedCards;
use super::rules::RuleWeapon;
use super::runtime_card::CardCatalog;
use super::schema::SchemaError;
use super::state::{Arena, Channel, DueKind, UnitState};

/// Prepare one complete runtime using only immutable payloads and hull facts.
/// `catalog` must be caller-owned staging; discard it on any refusal. No live
/// table, source owner, workflow record or condition slot is mutated here.
/// A private check-picture arena may use this same builder for a working copy.
#[allow(clippy::too_many_arguments)] // Admission must bind both immutable seals to the complete captured hull.
pub(crate) fn prepare_profile_runtime(
    catalog: &mut CardCatalog,
    unit: ObjId,
    cards: &SealedCards,
    rom: &SealedCards,
    classes: &ClassTable,
    class: CoreClass,
    weapons: &[RuleWeapon],
    channels: &[Channel],
    floor: u32,
) -> Result<ProfileRuntime, SchemaError> {
    let profile = classes.row(class);
    cards.verify(&profile, weapons, channels, floor)?;
    rom.verify(&classes.row(CoreClass::Zero), weapons, channels, floor)?;
    let (profiles, _) = cards.admit(catalog)?;
    let (_, rom) = rom.admit(catalog)?;
    let ids = weapons
        .iter()
        .map(|weapon| weapon.id.clone())
        .collect::<Vec<_>>();
    ProfileRuntime::new(
        catalog, unit, profiles, rom, &profile, &ids, channels, floor,
    )
}

/// Physical component/station facts for a newly constructed private check hull.
/// Live sortie transitions use SealRef's captured facts instead.
pub(crate) fn channels_for(weapons: &[RuleWeapon]) -> Vec<Channel> {
    let mut channels = vec![Channel::Helm, Channel::Sensors];
    if !weapons.is_empty() {
        channels.push(Channel::Fire);
    }
    channels
}

pub(crate) fn weapons_for(unit: &UnitState) -> Vec<RuleWeapon> {
    unit.weapons
        .iter()
        .map(|weapon| RuleWeapon {
            id: weapon.id.clone(),
            rounds_max: weapon.count_per_load,
        })
        .collect()
}

impl Arena {
    /// Shared native authority check. Manual slots carry their own override bit;
    /// complete authored rows retain sampled gates on FIRE, HOLD and LOCK alike.
    pub(crate) fn runtime_permits(&self, unit: ObjId, slot: u8, rule: &Rule, t_ms: i64) -> bool {
        self.profiles
            .get(&unit)
            .is_some_and(|runtime| runtime.permits(slot, rule, t_ms))
    }

    /// Ordinary native triggers at a renewal boundary wait for the already
    /// scheduled complete answer. The lease remains strictly half-open; this
    /// suppresses evaluation, never grants stale truth. Landing consumes or
    /// cancels pending before its final evaluation, including stale/no-sample.
    pub(crate) fn rule_evaluation_deferred(&self, unit: ObjId, t_ms: i64) -> bool {
        self.minds
            .get(&unit)
            .and_then(|mind| mind.pending.as_ref())
            .is_some_and(|pending| pending.lands_ms == t_ms)
            && self.profiles.get(&unit).is_some_and(|runtime| {
                runtime
                    .program
                    .rows
                    .iter()
                    .any(|row| !row.program.think.is_empty())
            })
    }

    /// Physical loss and outgoing ownership disable affected immutable bindings.
    /// This records a lawful missing row without changing the sealed program.
    pub(crate) fn disable_profile_slots(
        &mut self,
        unit: ObjId,
        slots: impl IntoIterator<Item = u8>,
    ) {
        if let Some(runtime) = self.profiles.get_mut(&unit) {
            for slot in slots {
                runtime.override_slot(slot);
            }
        }
        self.invalidate_rule_sample(unit);
    }

    /// A switch restores a complete immutable program, then current physical
    /// losses constrain its live table before anything can evaluate it.
    pub(crate) fn constrain_profile_hardware(&mut self, unit: ObjId) {
        let Some(own) = self.unit(unit) else {
            return;
        };
        if own.limits.wreck {
            self.clear_conditions(unit);
            return;
        }
        let absent = own
            .seal
            .weapons
            .iter()
            .filter(|weapon| own.weapon(&weapon.id).is_none())
            .map(|weapon| weapon.id.clone())
            .collect::<Vec<_>>();
        let freed = self.conditions.clear_weapons(unit, &absent);
        self.disable_profile_slots(unit, freed.iter().copied());
        self.forget_held_said(unit, &freed);
        self.truncate_slots_to(unit, self.arming_capacity(unit));
    }

    /// Ownership handback atomically restores the active selected program from
    /// captured bindings. Lost hardware remains unavailable and a degraded core
    /// does not regain slots; no pilot arming or sampled truth survives.
    pub(crate) fn restore_profile_bindings(&mut self, unit: ObjId) -> Result<(), SchemaError> {
        let own = self.unit(unit).ok_or_else(|| invalid("missing hull"))?;
        let capacity = self.arming_capacity(unit);
        let mut next = self
            .profiles
            .get(&unit)
            .ok_or_else(|| invalid("missing runtime"))?
            .clone();
        next.generation = next
            .generation
            .checked_add(1)
            .ok_or_else(|| invalid("generation exhausted"))?;
        next.overridden = 0;
        next.invalidate_sample();
        let mut rows = next.program.native_rows();
        for (index, row) in rows.iter_mut().enumerate() {
            let missing = row.as_ref().is_some_and(|rule| {
                rule.weapon()
                    .is_some_and(|weapon| own.weapon(weapon).is_none())
            });
            if index >= capacity || missing {
                *row = None;
                if let Ok(slot) = u8::try_from(index.saturating_add(1)) {
                    next.override_slot(slot);
                }
            }
        }
        rows.truncate(capacity);
        let prepared =
            super::conditions::ConditionTable::prepare_profile_rules(unit, &rows, capacity)
                .map_err(|_| invalid("cannot restore selected program"))?;
        self.conditions.apply_profile_rules(prepared);
        self.profiles.insert(unit, next);
        self.cancel_profile_think(unit);
        Ok(())
    }

    /// Ownership, fault and profile changes invalidate every sampled truth bit.
    pub(crate) fn invalidate_rule_sample(&mut self, unit: ObjId) {
        if let Some(runtime) = self.profiles.get_mut(&unit) {
            runtime.invalidate_sample();
        }
        self.due.retain(
            |kind| !matches!(kind, DueKind::RuleLeaseExpired { unit: on, .. } if *on == unit),
        );
    }

    /// Accept all sampled gates atomically after the pending token and all station
    /// epochs have passed. A missing sample deliberately clears the previous one.
    pub(crate) fn land_rule_sample(
        &mut self,
        unit: ObjId,
        sample: Option<RuleSample>,
        t_ms: i64,
        period_ms: i64,
    ) -> Result<(), SchemaError> {
        self.invalidate_rule_sample(unit);
        let runtime = self
            .profiles
            .get_mut(&unit)
            .ok_or_else(|| invalid("missing runtime"))?;
        runtime.lease.land(
            sample,
            &runtime.program,
            runtime.generation,
            t_ms,
            t_ms.saturating_add(period_ms),
        )?;
        if let Some(sample) = runtime.lease.sample() {
            self.due.schedule(
                sample.expires_ms,
                DueKind::RuleLeaseExpired {
                    unit,
                    generation: runtime.generation,
                    lands_ms: sample.lands_ms,
                },
            );
        }
        Ok(())
    }

    /// Expiry is an arena event, so an expired HOLD can release a ready automatic
    /// shot even if nothing else changes the picture at that millisecond.
    pub(crate) fn expire_rule_sample(
        &mut self,
        unit: ObjId,
        generation: u64,
        lands_ms: i64,
        t_ms: i64,
        counters: &mut MotorCounters,
    ) {
        let current = self.profiles.get(&unit).is_some_and(|runtime| {
            runtime.generation == generation
                && runtime
                    .lease
                    .sample()
                    .is_some_and(|sample| sample.lands_ms == lands_ms && sample.expires_ms == t_ms)
        });
        if !current {
            return;
        }
        self.invalidate_rule_sample(unit);
        let _ = self.evaluate_conditions(unit, t_ms, ConditionTrigger::Landing, None, counters);
    }

    /// Cancel an answer frozen under an earlier program while preserving memory
    /// and the next regular think's already assigned sequence number.
    pub(crate) fn cancel_profile_think(&mut self, unit: ObjId) {
        self.trace_cancel_pending(unit, super::author_trace::Cancelled::Superseded);
        self.invalidate_rule_sample(unit);
        self.awaiting.remove(&unit);
        self.collected.retain(|think| think.unit != unit);
        self.unsettled.retain(|think| think.unit != unit);
        if let Some(mind) = self.minds.get_mut(&unit) {
            mind.pending = None;
        }
        self.due
            .retain(|kind| !matches!(kind, DueKind::MindLand { unit: on, .. } if *on == unit));
        self.held_said.retain(|(on, _, _), _| *on != unit);
    }

    /// Replace a failed mind's complete card and bindings with its captured ROM.
    /// No current configuration preset is consulted and no authored rule survives.
    pub(crate) fn drop_runtime_to_rom(&mut self, unit: ObjId) -> Result<(), SchemaError> {
        let own = self.unit(unit).ok_or_else(|| invalid("missing hull"))?;
        let weapons = own.seal.weapons.clone();
        let ids = weapons
            .iter()
            .map(|weapon| weapon.id.clone())
            .collect::<Vec<SmolStr>>();
        let channels = own.seal.channels.clone();
        let profile = self.classes.row(CoreClass::Zero);
        let prepared = self
            .profiles
            .get(&unit)
            .ok_or_else(|| invalid("missing runtime"))?
            .prepare_rom(unit, &profile, &ids, &channels, self.odds_floor_permille)?;
        self.profiles
            .get_mut(&unit)
            .ok_or_else(|| invalid("missing runtime"))?
            .commit(&mut self.conditions, prepared)?;
        self.invalidate_rule_sample(unit);
        self.held_said.retain(|(on, _, _), _| *on != unit);
        Ok(())
    }

    /// Rehydrate every immutable cache and verify cross-owner references before a
    /// decoded arena is published or its hash accepted.
    pub(crate) fn hydrate_profiles(&mut self) -> Result<(), SchemaError> {
        self.cards.rebuild()?;
        // Collection freezes its landing timestamp. Hardware losses may change
        // the current speed ceiling and therefore the next substep length.
        let delay_range = i64::from(super::clock::MIN_SUBSTEP_MS)
            ..=i64::from(
                self.arena_cfg
                    .tempo
                    .substep_max_ms
                    .max(super::clock::MIN_SUBSTEP_MS),
            );
        if self.profiles.len() != self.units.len() {
            return Err(invalid("runtime/hull count mismatch"));
        }
        for own in &self.units {
            if own
                .own_last_hit_ms
                .is_some_and(|at| at < 0 || at > self.now_ms)
            {
                return Err(invalid("own-hit timestamp lies outside arena history"));
            }
            if self
                .minds
                .get(&own.id)
                .is_some_and(|mind| mind.seal_digest != own.seal.digest)
            {
                return Err(invalid("mind row differs from custody seal"));
            }
            let weapons = own.seal.weapons.clone();
            let ids = weapons
                .iter()
                .map(|weapon| weapon.id.clone())
                .collect::<Vec<_>>();
            let channels = own.seal.channels.clone();
            let runtime = self
                .profiles
                .get_mut(&own.id)
                .ok_or_else(|| invalid("missing hull runtime"))?;
            if runtime.profiles != own.seal.profiles || runtime.rom.key() != own.seal.rom {
                return Err(invalid("runtime differs from captured seal bindings"));
            }
            runtime.hydrate(
                &self.cards,
                own.id,
                &self.classes.row(own.seal.class),
                &self.classes.row(CoreClass::Zero),
                &ids,
                &channels,
                self.odds_floor_permille,
                self.now_ms,
            )?;
            let effective = if runtime.dropped {
                CoreClass::Zero
            } else {
                super::core::read_core(&own.unit)
                    .map_or(CoreClass::Zero, |core| core.class.min(own.seal.class))
            };
            runtime.lease.validate(
                &runtime.program,
                runtime.generation,
                self.now_ms,
                self.classes.row(effective).think_ms,
            )?;
            if runtime.index != own.card_profile.index
                || runtime.switched_ms != own.card_profile.switched_ms
            {
                return Err(invalid("active profile mirror mismatch"));
            }
            if self
                .minds
                .get(&own.id)
                .is_some_and(|mind| mind.dropped.is_some())
                != runtime.dropped
            {
                return Err(invalid("ROM drop mismatch"));
            }
            if own.limits.wreck
                && (!self.conditions.is_empty_for(own.id) || runtime.lease.sample().is_some())
            {
                return Err(invalid("wreck retains rule authority"));
            }
            if own.controller != super::state::Controller::Mind && runtime.lease.sample().is_some()
            {
                return Err(invalid("dormant hull retains sampled authority"));
            }
            let mut expiry = self.due.iter().filter(|event| {
                matches!(event.kind,
                DueKind::RuleLeaseExpired { unit, .. } if unit == own.id)
            });
            match runtime.lease.sample() {
                Some(sample) => {
                    // A sample is stamped with its picture's millisecond (the
                    // substep boundary at or after the scheduled think,
                    // ae959449), and lands one substep after the think: the lead
                    // runs from zero to one maximum substep, not from MIN.
                    let lead = sample.lands_ms.saturating_sub(sample.sampled_ms);
                    if !(0..=*delay_range.end()).contains(&lead) {
                        return Err(invalid(
                            "active sample differs from collection landing delay",
                        ));
                    }
                    if !expiry.next().is_some_and(|event| event.t_ms == sample.expires_ms
                        && matches!(event.kind, DueKind::RuleLeaseExpired { generation, lands_ms, .. }
                            if generation == runtime.generation && lands_ms == sample.lands_ms))
                        || expiry.next().is_some() || (sample.expires_ms <= self.now_ms && self.ended.is_none()) {
                        return Err(invalid("sample expiry schedule mismatch"));
                    }
                }
                None if expiry.next().is_some() => return Err(invalid("expiry without sample")),
                None => {}
            }
            for (index, expected) in runtime.program.native_rows().iter().enumerate() {
                let bit = 1_u8
                    .checked_shl(u32::try_from(index).unwrap_or(8))
                    .unwrap_or(0);
                if runtime.overridden & bit == 0
                    && self
                        .conditions
                        .slots(own.id)
                        .get(index)
                        .and_then(Option::as_ref)
                        != expected.as_ref()
                {
                    return Err(invalid("native table differs from active program"));
                }
            }
            if let Some(pending) = self
                .minds
                .get(&own.id)
                .and_then(|mind| mind.pending.as_ref())
            {
                if !runtime.accepts(pending.profile_token) {
                    return Err(invalid("stale pending profile"));
                }
                let mut lease = super::profile_rules::RuleLease::default();
                lease.land(
                    pending.rule_sample.clone(),
                    &runtime.program,
                    runtime.generation,
                    pending.lands_ms,
                    pending.lands_ms.saturating_add(pending.period_ms),
                )?;
                if pending.profile_class > own.seal.class
                    || pending.period_ms != self.classes.row(pending.profile_class).think_ms
                    || pending.t_ms < 0 || pending.t_ms > self.now_ms
                    || !delay_range.contains(&pending.lands_ms.saturating_sub(pending.t_ms))
                    || pending.rule_sample.as_ref().is_some_and(|sample| {
                        // Stamped at its picture: never before the think, never
                        // after its landing or the arena's present.
                        sample.sampled_ms < pending.t_ms
                            || sample.sampled_ms > pending.lands_ms.min(self.now_ms)
                    })
                    || !self.due.iter().any(|event| event.t_ms == pending.lands_ms && matches!(event.kind,
                        DueKind::MindLand { unit, think_seq } if unit == own.id && think_seq == pending.think_seq)) {
                    return Err(invalid("pending sample schedule differs from collection"));
                }
            }
        }
        Ok(())
    }
}

fn invalid(message: &str) -> SchemaError {
    SchemaError {
        field: "runtime".into(),
        message: message.into(),
    }
}
