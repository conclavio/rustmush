//! Report-only observers around the actual sealed runners and streamed ledger.
//! Nothing here changes a result, source, arena cell, seed or hash.
use super::{BulkError, invalid, report::Game};
use crate::arena::{
    Arena,
    ledger::{EventKind, LedgerChunk, LedgerEvent},
    mind::{MindRunner, ThinkJob, ThinkResult},
    motor::MotorCounters,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

#[derive(Debug)]
pub(super) struct Operations {
    histogram: Vec<u32>,
    unmeasured: u32,
    invalid: bool,
}
impl Default for Operations {
    fn default() -> Self {
        Self {
            histogram: vec![0; 4501],
            unmeasured: 0,
            invalid: false,
        }
    }
}
#[derive(Debug)]
struct Measured {
    runner: Arc<dyn MindRunner>,
    totals: Arc<Mutex<Operations>>,
}
impl MindRunner for Measured {
    fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
        let result = self.runner.think(job);
        if let Ok(mut totals) = self.totals.lock() {
            match result.diagnostics.as_ref().and_then(|d| d.counts()) {
                Some((ops, _)) if ops <= 4500 => {
                    if let Some(bucket) = totals.histogram.get_mut(ops as usize) {
                        *bucket = bucket.saturating_add(1);
                    } else {
                        totals.invalid = true;
                    }
                }
                Some(_) => totals.invalid = true,
                None => totals.unmeasured = totals.unmeasured.saturating_add(1),
            }
        }
        result
    }
}
pub(super) fn instrument(arena: &mut Arena) -> Arc<Mutex<Operations>> {
    let totals = Arc::new(Mutex::new(Operations::default()));
    arena.mind_runner = Arc::new(Measured {
        runner: Arc::clone(&arena.mind_runner),
        totals: Arc::clone(&totals),
    });
    for runner in arena.runners.values_mut() {
        *runner = Arc::new(Measured {
            runner: Arc::clone(runner),
            totals: Arc::clone(&totals),
        });
    }
    totals
}
#[derive(Default)]
pub(super) struct Metrics {
    sequence: u64,
    shots: BTreeMap<u64, String>,
    section_losses: BTreeMap<String, u32>,
    weapon_outcomes: BTreeMap<String, u32>,
    facet_outcomes: BTreeMap<String, u32>,
    rounds: u32,
    shot_count: u32,
    rules_fired: u32,
    rules_held: u32,
    faults: u32,
    thinks: u32,
    first_section_unit: Option<mush_core::ObjId>,
}
impl Metrics {
    pub fn motor(&mut self, c: &MotorCounters) {
        self.thinks = self.thinks.saturating_add(c.thinks);
        self.rules_fired = self.rules_fired.saturating_add(c.cond_fires);
        self.rules_held = self.rules_held.saturating_add(c.cond_held);
        self.faults = self.faults.saturating_add(c.mind_faults);
    }
    pub fn chunks(&mut self, chunks: &[LedgerChunk]) -> Result<(), BulkError> {
        for chunk in chunks {
            for line in chunk
                .bytes
                .split(|b| *b == b'\n')
                .filter(|line| !line.is_empty())
            {
                let row: LedgerEvent =
                    serde_json::from_slice(line).map_err(|_| invalid("proving ledger decode"))?;
                if row.seq != self.sequence + 1 {
                    return Err(invalid("proving ledger gap or duplicate"));
                }
                self.sequence = row.seq;
                match row.kind {
                    EventKind::Fired {
                        shot_seq,
                        weapon,
                        rounds,
                        ..
                    } => {
                        if self.shots.len() >= 4096
                            || self.shots.insert(shot_seq, weapon.to_string()).is_some()
                        {
                            return Err(invalid("proving shot correlation bound"));
                        }
                        increment(&mut self.weapon_outcomes, format!("{weapon}:fired"), 1)?;
                        self.rounds = self.rounds.saturating_add(rounds);
                        self.shot_count = self.shot_count.saturating_add(1);
                    }
                    EventKind::Hit {
                        shot_seq,
                        facet,
                        rounds,
                        ..
                    } => {
                        let weapon = self
                            .shots
                            .remove(&shot_seq)
                            .ok_or_else(|| invalid("hit without fired shot"))?;
                        increment(&mut self.weapon_outcomes, format!("{weapon}:hit"), rounds)?;
                        increment(&mut self.facet_outcomes, facet.to_string(), rounds)?;
                    }
                    EventKind::Miss { shot_seq, reason } => {
                        let weapon = self
                            .shots
                            .remove(&shot_seq)
                            .ok_or_else(|| invalid("miss without fired shot"))?;
                        increment(
                            &mut self.weapon_outcomes,
                            format!("{weapon}:miss:{reason}"),
                            1,
                        )?;
                    }
                    EventKind::SectionDestroyed { unit, section, .. } => {
                        self.first_section_unit.get_or_insert(unit);
                        increment(&mut self.section_losses, section.to_string(), 1)?
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }
    pub fn finish(
        self,
        game: &mut Game,
        ops: &Mutex<Operations>,
        arena: &Arena,
        focus: crate::arena::Team,
    ) -> Result<(), BulkError> {
        let ops = ops
            .lock()
            .map_err(|_| invalid("proving operation observer failed"))?;
        if ops.invalid {
            return Err(invalid(
                "operation count exceeded the declared report bound",
            ));
        }
        game.section_losses = self.section_losses;
        game.weapon_outcomes = self.weapon_outcomes;
        game.facet_outcomes = self.facet_outcomes;
        game.rounds = self.rounds;
        game.rounds_left = arena
            .units
            .iter()
            .filter(|u| u.team == focus)
            .flat_map(|u| u.ammo.values())
            .copied()
            .fold(0_u32, u32::saturating_add);
        game.lost_first_section = self
            .first_section_unit
            .and_then(|id| arena.unit(id))
            .map(|u| u.team == focus);
        game.rules_fired = self.rules_fired;
        game.rules_held = self.rules_held;
        game.faults = self.faults;
        game.operations = ops.histogram.clone();
        let measured = ops
            .histogram
            .iter()
            .fold(0_u32, |n, on| n.saturating_add(*on));
        game.unmeasured_thinks = ops.unmeasured.max(self.thinks.saturating_sub(measured));
        // The existing ledger does not retain acceptance odds. Keep this
        // explicit until a report-only acceptance observer provides the fact.
        game.odds_unobserved = self.shot_count;
        Ok(())
    }
}
fn increment(map: &mut BTreeMap<String, u32>, key: String, amount: u32) -> Result<(), BulkError> {
    if key.is_empty()
        || key.len() > 128
        || key.chars().any(char::is_control)
        || (map.len() >= 128 && !map.contains_key(&key))
    {
        return Err(invalid("proving metric key bound"));
    }
    let n = map.entry(key).or_default();
    *n = n.saturating_add(amount);
    Ok(())
}
