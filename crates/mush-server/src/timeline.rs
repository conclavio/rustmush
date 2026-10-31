//! Boot hydration for the engine-owned world in-character timeline.
//!
//! The configuration clock is a genesis seed only.  Once generation one is
//! present, the immutable epoch documents and their materialized head are the
//! authority across restart.  This keeps a later configuration edit from
//! reinterpreting timestamps that were recorded under an older rate.

use std::collections::BTreeMap;

use mush_core::{
    GameClock, MAX_WORLD_TIMELINE_EPOCHS, WORLD_TIMELINE_EPOCH_PREFIX, WORLD_TIMELINE_HEAD_KEY,
    WORLD_TIMELINE_NAMESPACE, WORLD_TIMELINE_STATE_KEY, World, WorldClockCheckpoint,
    WorldClockFreeze, WorldClockFreezeKind, WorldTimeline, WorldTimelineEpoch, WorldTimelineHead,
    WorldTimelineState, world_timeline_epoch_key, world_timeline_persistence_ops,
    world_timeline_state_op,
};
use mush_store::{ObjectStore, WriteBatch};

const BOOT_SERIAL: u64 = 0;

/// Load persisted world-time truth, seeding generation one atomically from
/// configuration only when the reserved namespace is empty.
pub async fn load_or_seed_world_timeline(
    store: &dyn ObjectStore,
    configured: GameClock,
) -> Result<WorldTimeline, String> {
    let mut docs = store
        .module_json_list(WORLD_TIMELINE_NAMESPACE)
        .await
        .map_err(|error| format!("loading world timeline: {error}"))?;
    if docs.is_empty() {
        let seeded = WorldTimeline::from_game_clock(configured);
        let epoch = seeded.current();
        let batch = WriteBatch {
            serial: BOOT_SERIAL,
            module_ops: world_timeline_persistence_ops(epoch)
                .map_err(|error| format!("encoding world timeline genesis: {error}"))?
                .into_iter()
                .collect(),
            durable: true,
            ..WriteBatch::default()
        };
        let receipt = store
            .write(batch)
            .await
            .map_err(|error| format!("persisting world timeline genesis: {error}"))?;
        if receipt.serial != BOOT_SERIAL {
            return Err(format!(
                "persisting world timeline genesis: expected receipt serial {BOOT_SERIAL}, got {}",
                receipt.serial
            ));
        }
        docs = store
            .module_json_list(WORLD_TIMELINE_NAMESPACE)
            .await
            .map_err(|error| format!("verifying world timeline genesis: {error}"))?;
    }

    let timeline = decode_world_timeline(docs)?;
    let configured_genesis = WorldTimeline::from_game_clock(configured).current();
    if timeline.epochs()[0] != configured_genesis {
        tracing::warn!(
            persisted_generation = timeline.current().generation,
            configured_rate = configured_genesis.rate,
            "persisted world timeline overrides changed [game] clock configuration"
        );
    }
    Ok(timeline)
}

/// Hydrate the world before module boot so every ordinary [`mush_core::Ctx`]
/// observes the same versioned timeline.
pub async fn hydrate_world_timeline(
    world: &mut World,
    store: &dyn ObjectStore,
    configured: GameClock,
) -> Result<(), String> {
    let timeline = load_or_seed_world_timeline(store, configured).await?;
    world.set_world_timeline(timeline);
    Ok(())
}

/// Outage detection and recovery countdown policy (`W-clock` / `FO-11` / `Q-R4`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutagePolicy {
    /// Restart downtime threshold in real seconds (`[game].outage_freeze_threshold_s`, default `30`).
    pub freeze_threshold_s: u64,
    /// Post-outage recovery countdown in real seconds (`[game].outage_countdown_s`, default `300`).
    pub countdown_s: u64,
}

impl Default for OutagePolicy {
    fn default() -> Self {
        Self {
            freeze_threshold_s: 30,
            countdown_s: 300,
        }
    }
}

/// What [`evaluate_boot_outage`] decided for a restart at `boot_unix`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutageEvaluation {
    /// No prior durable checkpoint existed; initial checkpoint recorded at `boot_unix`.
    NoPriorCheckpoint,
    /// Restart gap was below `freeze_threshold_s`; silently re-anchored with no
    /// countdown and no catch-up (`Q-R4`).
    SilentReanchor {
        /// Wall-clock seconds excluded by the silent re-anchor.
        lost_secs: i64,
    },
    /// Unplanned outage met or exceeded `freeze_threshold_s`; IC world held in
    /// the recovery countdown (`FO-11`, `Q-H11`, `Q-R4`).
    OutageFreeze {
        /// Wall-clock seconds lost before `boot_unix`.
        lost_secs: i64,
        /// Installed outage freeze descriptor.
        freeze: WorldClockFreeze,
    },
    /// World clock was already deliberately frozen before the restart.
    DeliberateFreeze {
        /// Installed deliberate freeze descriptor.
        freeze: WorldClockFreeze,
    },
}

/// Evaluate a hydrated [`WorldTimeline`] against `boot_unix` and `policy`
/// (`W-clock` / `Q-R4` / `FO-11`).
pub fn evaluate_boot_outage(
    timeline: &mut WorldTimeline,
    boot_unix: i64,
    policy: OutagePolicy,
) -> Result<OutageEvaluation, String> {
    let threshold = i64::try_from(policy.freeze_threshold_s).unwrap_or(i64::MAX);
    let countdown = i64::try_from(policy.countdown_s).unwrap_or(300);

    if let Some(existing) = timeline.freeze() {
        return match existing.kind {
            WorldClockFreezeKind::Deliberate => {
                timeline.record_checkpoint(boot_unix);
                Ok(OutageEvaluation::DeliberateFreeze { freeze: existing })
            }
            WorldClockFreezeKind::OutageCountdown {
                unfreeze_at_real_unix,
                ..
            } => {
                let last_real = timeline
                    .checkpoint()
                    .map_or(existing.frozen_real_unix, |cp| cp.real_unix);
                let gap = boot_unix.saturating_sub(last_real);
                if gap >= threshold && countdown > 0 {
                    // Another unplanned outage occurred during the countdown: restart
                    // the recovery countdown from `boot_unix` while keeping the original
                    // `frozen_real_unix` and `frozen_ic_unix`.
                    let unfreeze_at = boot_unix.saturating_add(countdown);
                    let _ = timeline.resume_at(existing.frozen_real_unix);
                    let freeze = timeline.freeze_for_outage(
                        existing.frozen_real_unix,
                        existing.frozen_ic_unix,
                        boot_unix,
                        unfreeze_at,
                    );
                    timeline.record_checkpoint(boot_unix);
                    Ok(OutageEvaluation::OutageFreeze {
                        lost_secs: boot_unix.saturating_sub(existing.frozen_real_unix),
                        freeze,
                    })
                } else if boot_unix < unfreeze_at_real_unix {
                    timeline.record_checkpoint(boot_unix);
                    Ok(OutageEvaluation::OutageFreeze {
                        lost_secs: boot_unix.saturating_sub(existing.frozen_real_unix),
                        freeze: existing,
                    })
                } else {
                    timeline
                        .resume_at(boot_unix)
                        .map_err(|e| format!("resuming elapsed outage countdown: {e}"))?;
                    Ok(OutageEvaluation::SilentReanchor {
                        lost_secs: boot_unix.saturating_sub(existing.frozen_real_unix),
                    })
                }
            }
        };
    }

    let Some(cp) = timeline.checkpoint() else {
        timeline.record_checkpoint(boot_unix);
        return Ok(OutageEvaluation::NoPriorCheckpoint);
    };

    if boot_unix <= cp.real_unix {
        return Ok(OutageEvaluation::SilentReanchor { lost_secs: 0 });
    }

    let lost_secs = boot_unix.saturating_sub(cp.real_unix);
    if lost_secs >= threshold {
        if countdown > 0 {
            let unfreeze_at = boot_unix.saturating_add(countdown);
            let freeze =
                timeline.freeze_for_outage(cp.real_unix, cp.ic_unix, boot_unix, unfreeze_at);
            timeline.record_checkpoint(boot_unix);
            Ok(OutageEvaluation::OutageFreeze { lost_secs, freeze })
        } else {
            timeline
                .reanchor_from_checkpoint(boot_unix)
                .map_err(|e| format!("re-anchoring world timeline after outage: {e}"))?;
            Ok(OutageEvaluation::SilentReanchor { lost_secs })
        }
    } else {
        timeline
            .reanchor_from_checkpoint(boot_unix)
            .map_err(|e| format!("silently re-anchoring world timeline: {e}"))?;
        Ok(OutageEvaluation::SilentReanchor { lost_secs })
    }
}

/// Apply [`evaluate_boot_outage`] to `world`'s hydrated timeline and durably
/// persist the result (`W-clock`).
///
/// Every evaluation records the boot as the timeline's checkpoint (a clock that
/// did not advance is the one exception, and re-writing the unchanged state is
/// harmless), and that checkpoint is what the next boot measures its outage
/// from — so the state is written unconditionally, one small durable op. On a
/// fresh store this is the world's first durable checkpoint. Until 2026-10-31
/// `NoPriorCheckpoint` was the one evaluation that did not persist, so a world
/// seeded from configuration never wrote a checkpoint and every later restart
/// was a first boot to the policy (doc 33 GG.5: `boot_outage=NoPriorCheckpoint`
/// on every boot of both boxes). While the server runs, the world task moves the
/// checkpoint forward every `[game].world_clock_checkpoint_s`
/// ([`crate::world_task::step_world_clock_checkpoint`]).
pub async fn apply_boot_outage_policy(
    world: &mut World,
    store: &dyn ObjectStore,
    boot_unix: i64,
    policy: OutagePolicy,
) -> Result<OutageEvaluation, String> {
    let mut timeline = world.world_timeline().clone();
    let evaluation = evaluate_boot_outage(&mut timeline, boot_unix, policy)?;
    let op = world_timeline_state_op(&timeline)
        .map_err(|error| format!("encoding world timeline state: {error}"))?;
    store
        .write(WriteBatch {
            serial: BOOT_SERIAL,
            module_ops: vec![op],
            durable: true,
            ..WriteBatch::default()
        })
        .await
        .map_err(|error| format!("persisting world timeline outage state: {error}"))?;
    world.set_world_timeline(timeline);
    world.set_now(boot_unix);
    Ok(evaluation)
}

/// Record and durably persist a world-IC clock checkpoint at `real_unix`,
/// awaiting the store inline (`W-clock` / `Q-R4`).
///
/// The live world does not checkpoint through here: the world task records the
/// checkpoint on its cadence and queues the same state op on its deferred
/// persistence path ([`crate::world_task::step_world_clock_checkpoint`]). This
/// inline writer is the boot-path tests' way of planting a checkpoint.
pub async fn persist_world_clock_checkpoint(
    world: &mut World,
    store: &dyn ObjectStore,
    real_unix: i64,
) -> Result<WorldClockCheckpoint, String> {
    world.set_now(real_unix);
    let cp = world.record_world_clock_checkpoint();
    let op = world_timeline_state_op(world.world_timeline())
        .map_err(|error| format!("encoding world timeline checkpoint: {error}"))?;
    store
        .write(WriteBatch {
            serial: BOOT_SERIAL,
            module_ops: vec![op],
            durable: true,
            ..WriteBatch::default()
        })
        .await
        .map_err(|error| format!("persisting world timeline checkpoint: {error}"))?;
    Ok(cp)
}

fn decode_world_timeline(docs: Vec<(String, serde_json::Value)>) -> Result<WorldTimeline, String> {
    let has_state = docs.iter().any(|(key, _)| key == WORLD_TIMELINE_STATE_KEY);
    let max_docs = MAX_WORLD_TIMELINE_EPOCHS + 1 + usize::from(has_state);
    if docs.len() > max_docs {
        return Err(format!(
            "world timeline namespace contains {} documents; maximum is {} epochs plus one head",
            docs.len(),
            MAX_WORLD_TIMELINE_EPOCHS
        ));
    }
    let mut head = None;
    let mut state = None;
    let mut epochs = BTreeMap::new();
    for (key, value) in docs {
        if key == WORLD_TIMELINE_HEAD_KEY {
            if head.is_some() {
                return Err("world timeline contains duplicate head documents".to_owned());
            }
            let parsed: WorldTimelineHead = serde_json::from_value(value)
                .map_err(|error| format!("invalid world timeline head: {error}"))?;
            head = Some(
                parsed
                    .validated_generation()
                    .map_err(|error| format!("invalid world timeline head: {error}"))?,
            );
            continue;
        }
        if key == WORLD_TIMELINE_STATE_KEY {
            if state.is_some() {
                return Err("world timeline contains duplicate state documents".to_owned());
            }
            let parsed: WorldTimelineState = serde_json::from_value(value)
                .map_err(|error| format!("invalid world timeline state: {error}"))?;
            state = Some(parsed);
            continue;
        }
        let Some(suffix) = key.strip_prefix(WORLD_TIMELINE_EPOCH_PREFIX) else {
            return Err(format!(
                "unknown document {key:?} in reserved world timeline namespace"
            ));
        };
        if suffix.len() != 20 || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(format!("non-canonical world timeline epoch key {key:?}"));
        }
        let generation = suffix
            .parse::<u64>()
            .map_err(|error| format!("invalid world timeline generation in {key:?}: {error}"))?;
        let epoch: WorldTimelineEpoch = serde_json::from_value(value)
            .map_err(|error| format!("invalid world timeline epoch {generation}: {error}"))?;
        if epoch.generation != generation || world_timeline_epoch_key(generation) != key.as_str() {
            return Err(format!(
                "world timeline epoch key {key:?} disagrees with generation {}",
                epoch.generation
            ));
        }
        if epochs.insert(generation, epoch).is_some() {
            return Err(format!(
                "world timeline contains duplicate generation {generation}"
            ));
        }
    }
    let head = head.ok_or_else(|| "world timeline head is missing".to_owned())?;
    let timeline = WorldTimeline::from_epochs_and_state(
        epochs.into_values().collect(),
        state.unwrap_or_default(),
    )
    .map_err(|error| format!("invalid world timeline: {error}"))?;
    if timeline.current().generation != head {
        return Err(format!(
            "world timeline head names generation {head}, but contiguous history ends at {}",
            timeline.current().generation
        ));
    }
    Ok(timeline)
}

#[cfg(test)]
mod tests {
    use mush_core::{ModuleOp, ModuleWrite};
    use mush_store::{InMemoryStore, RedisStore};
    use smol_str::SmolStr;

    use super::*;

    #[tokio::test]
    async fn empty_store_gets_one_durable_genesis() {
        let store = InMemoryStore::new();
        let configured = GameClock {
            ic_ratio: 4,
            epoch_offset: 900,
        };

        let first = load_or_seed_world_timeline(&store, configured)
            .await
            .unwrap();
        let second = load_or_seed_world_timeline(&store, configured)
            .await
            .unwrap();

        assert_eq!(first, second);
        assert_eq!(first.epochs().len(), 1);
        assert_eq!(first.current().generation, 1);
        assert_eq!(first.current().ic_at(25), 1_000);
    }

    /// **E7 cut 1 (10b §7, OPEN 4; 08i §3's configuration row): `[tacsim.time]` reaches
    /// this function never.**
    ///
    /// Two worlds whose configuration differs only in `[tacsim.time]` — a campaign rate
    /// policy of one IC second per real second against one of four thousand — seed the
    /// **same** generation 1, anchored on `[game]`. The world timeline is the kernel's
    /// and `[game]` is its only seed; `[tacsim.time]` is *policy* that cuts 4, 8 and 11
    /// read, and cut 1 hands it to nothing at all.
    ///
    /// The second half is the one that would actually bite: a persisted timeline still
    /// wins, with the existing warning, no matter what `[tacsim.time]` says.
    #[tokio::test]
    async fn e7c1_config_tacsim_time_never_seeds_the_world_timeline() {
        let quiet = "[game]\nic_ratio = 4\nic_epoch = \"2020-01-01 00:00\"\n";
        let loud = "[game]\nic_ratio = 4\nic_epoch = \"2020-01-01 00:00\"\n\n\
                    [tacsim]\nenabled = true\n\n\
                    [tacsim.time]\nrate = 4000\ncatch_up_journeys = \"restamp\"\n\
                    catch_up_production = \"pause\"\ncatch_up_fronts = \"none\"\n\
                    catch_up_domains = \"refuse\"\n";
        let mut seeded = Vec::new();
        for text in [quiet, loud] {
            let config: mush_config::config::Config =
                toml::from_str(text).expect("the config parses");
            // `[tacsim]` converts, and produces no clock of any kind: `tacsim_config`'s
            // return type carries pack bytes and inert policy words, and nothing else.
            let tacsim = crate::config_ext::tacsim_config(&config.tacsim)
                .expect("the [tacsim] block converts");
            assert_eq!(tacsim.time.rate, config.tacsim.time.rate);
            // Only `[game]` builds the clock this function is seeded from.
            let clock = config.game.game_clock().expect("the [game] clock builds");
            let store = InMemoryStore::new();
            let timeline = load_or_seed_world_timeline(&store, clock).await.unwrap();
            assert_eq!(timeline.epochs().len(), 1);
            assert_eq!(timeline.current().generation, 1);
            seeded.push(timeline);
        }
        assert_eq!(
            seeded[0], seeded[1],
            "[tacsim.time] moved generation 1; it must reach the world timeline never"
        );
        assert_eq!(
            seeded[0].current().rate,
            4,
            "generation 1's rate is [game]'s"
        );

        // And persisted truth still wins: open a second generation, then re-load with a
        // wildly different `[tacsim.time]` and an unchanged `[game]`.
        let store = InMemoryStore::new();
        let clock = GameClock {
            ic_ratio: 4,
            epoch_offset: 0,
        };
        let mut timeline = load_or_seed_world_timeline(&store, clock).await.unwrap();
        let next = timeline.next_rate_epoch(100, 9).unwrap();
        store
            .write(WriteBatch {
                serial: 1,
                module_ops: world_timeline_persistence_ops(next)
                    .unwrap()
                    .into_iter()
                    .collect(),
                durable: true,
                ..WriteBatch::default()
            })
            .await
            .unwrap();
        timeline.append(next).unwrap();
        let reloaded = load_or_seed_world_timeline(&store, clock).await.unwrap();
        assert_eq!(reloaded, timeline);
        assert_eq!(reloaded.current().rate, 9);
    }

    /// Persisted truth beats a changed `[game].ic_ratio`.
    ///
    /// *Amended in place by the E7 residue pass 1 independent verify fold (C9/B10,
    /// owner decision F9(a)).* The second generation used to be installed at
    /// `rate = 0`, which `WorldTimeline::next_rate_epoch` now refuses — a stopped
    /// clock is not a legal rate. The property under test is that the **persisted**
    /// generation wins over the configured one, and it is asserted at a legal rate.
    #[tokio::test]
    async fn persisted_rate_generation_wins_over_changed_config() {
        let store = InMemoryStore::new();
        let initial = GameClock {
            ic_ratio: 2,
            epoch_offset: 10,
        };
        let mut timeline = load_or_seed_world_timeline(&store, initial).await.unwrap();
        let next = timeline.next_rate_epoch(100, 1).unwrap();
        store
            .write(WriteBatch {
                serial: 1,
                module_ops: world_timeline_persistence_ops(next)
                    .unwrap()
                    .into_iter()
                    .collect(),
                durable: true,
                ..WriteBatch::default()
            })
            .await
            .unwrap();
        timeline.append(next).unwrap();

        let loaded = load_or_seed_world_timeline(
            &store,
            GameClock {
                ic_ratio: 99,
                epoch_offset: -1,
            },
        )
        .await
        .unwrap();
        assert_eq!(loaded, timeline);
        // Generation 2 anchors at IC 210 and runs at 1:1 from real second 100, so
        // real 10 000 is IC 210 + 9 900 — the CONFIGURED 99:1 reaches it never.
        assert_eq!(loaded.stamp_at(10_000).at, 10_110);
        assert_eq!(
            loaded.stamp_at(10_000).clock,
            mush_core::ClockRef::WorldIc { generation: 2 }
        );
    }

    #[tokio::test]
    async fn incomplete_or_noncontiguous_history_fails_closed() {
        let store = InMemoryStore::new();
        store
            .write(WriteBatch {
                serial: 0,
                module_ops: vec![ModuleOp {
                    ns: SmolStr::new_static(WORLD_TIMELINE_NAMESPACE),
                    op: ModuleWrite::JsonSet {
                        key: SmolStr::new_static(WORLD_TIMELINE_HEAD_KEY),
                        value: serde_json::to_value(WorldTimelineHead::new(2)).unwrap(),
                    },
                }],
                ..WriteBatch::default()
            })
            .await
            .unwrap();

        let error = load_or_seed_world_timeline(&store, GameClock::default())
            .await
            .unwrap_err();
        assert!(error.contains("empty"), "{error}");
    }

    #[tokio::test]
    #[ignore = "requires REDIS_URL pointing at a disposable Redis database"]
    async fn redis_round_trip_preserves_exact_generations() {
        let url = std::env::var("REDIS_URL")
            .ok()
            .filter(|value| !value.is_empty())
            .expect("REDIS_URL must name a disposable Redis database");
        let store = RedisStore::connect(&url).await.expect("connect Redis");
        store
            .wipe_namespaces()
            .await
            .expect("wipe disposable Redis namespaces");
        let configured = GameClock {
            ic_ratio: 3,
            epoch_offset: 17,
        };
        let timeline = load_or_seed_world_timeline(&store, configured)
            .await
            .expect("seed timeline");
        let next = timeline
            .next_rate_epoch(500, 1)
            .expect("prepare generation two");
        store
            .write(WriteBatch {
                serial: 2,
                module_ops: world_timeline_persistence_ops(next)
                    .unwrap()
                    .into_iter()
                    .collect(),
                durable: true,
                ..WriteBatch::default()
            })
            .await
            .expect("append generation two");

        let loaded = load_or_seed_world_timeline(&store, GameClock::default())
            .await
            .expect("reload timeline");
        assert_eq!(loaded.epochs(), &[timeline.current(), next]);
        assert_eq!(loaded.stamp_at(600).at, next.ic_anchor + 100);
    }

    #[tokio::test]
    async fn w_clock_restart_below_threshold_silently_reanchors_with_no_catch_up() {
        let store = InMemoryStore::new();
        let clock = GameClock {
            ic_ratio: 4,
            epoch_offset: 1_000,
        };
        let mut world = World::new_minimal();
        hydrate_world_timeline(&mut world, &store, clock)
            .await
            .unwrap();
        // Record a durable checkpoint at real_unix = 100 (IC = 1_400).
        let cp = persist_world_clock_checkpoint(&mut world, &store, 100)
            .await
            .unwrap();
        assert_eq!(cp.real_unix, 100);
        assert_eq!(cp.ic_unix, 1_400);

        // Simulate restart 20s later (< 30s default threshold).
        let mut booted = World::new_minimal();
        hydrate_world_timeline(&mut booted, &store, clock)
            .await
            .unwrap();
        let eval = apply_boot_outage_policy(&mut booted, &store, 120, OutagePolicy::default())
            .await
            .unwrap();
        assert_eq!(eval, OutageEvaluation::SilentReanchor { lost_secs: 20 });
        assert!(!booted.world_timeline().is_frozen());
        // IC time at real_unix = 120 is re-anchored to the checkpoint's 1_400 (zero catch-up).
        assert_eq!(booted.world_ic_now().at, 1_400);
        assert_eq!(booted.game_now_unix(), 1_400);
        // 10 real seconds later (real_unix = 130), IC advances at 4:1 -> 1_440.
        booted.set_now(130);
        assert_eq!(booted.world_ic_now().at, 1_440);
        assert_eq!(booted.game_now_unix(), 1_440);
    }

    #[tokio::test]
    async fn w_clock_restart_above_threshold_enters_5m_outage_countdown_and_persists_state() {
        let store = InMemoryStore::new();
        let clock = GameClock {
            ic_ratio: 4,
            epoch_offset: 1_000,
        };
        let mut world = World::new_minimal();
        hydrate_world_timeline(&mut world, &store, clock)
            .await
            .unwrap();
        persist_world_clock_checkpoint(&mut world, &store, 100)
            .await
            .unwrap();

        // Simulate unplanned outage of 90s (>= 30s threshold): boot at real_unix = 190.
        let mut booted = World::new_minimal();
        hydrate_world_timeline(&mut booted, &store, clock)
            .await
            .unwrap();
        let eval = apply_boot_outage_policy(&mut booted, &store, 190, OutagePolicy::default())
            .await
            .unwrap();
        let OutageEvaluation::OutageFreeze { lost_secs, freeze } = eval else {
            panic!("expected OutageFreeze, got {eval:?}");
        };
        assert_eq!(lost_secs, 90);
        assert_eq!(freeze.frozen_real_unix, 100);
        assert_eq!(freeze.frozen_ic_unix, 1_400);
        assert_eq!(
            freeze.kind,
            WorldClockFreezeKind::OutageCountdown {
                countdown_started_real_unix: 190,
                unfreeze_at_real_unix: 490,
            }
        );
        assert!(booted.world_timeline().is_frozen());
        // Throughout the 5-minute countdown, both `world_ic_now` and `game_now_unix`
        // stay pinned at 1_400 without advancing.
        booted.set_now(350);
        assert_eq!(booted.world_ic_now().at, 1_400);
        assert_eq!(booted.game_now_unix(), 1_400);

        // Reloading from the store preserves the persisted outage freeze state.
        let reloaded = load_or_seed_world_timeline(&store, clock).await.unwrap();
        assert_eq!(reloaded.freeze(), Some(freeze));
    }

    /// Doc 33 GG.5, as seen on both boxes on 2026-10-31: the first boot on an empty
    /// store and the hot restart minutes later both logged
    /// `boot_outage=NoPriorCheckpoint`, because the fresh store's first checkpoint
    /// was recorded in memory and never written. A fresh store's first boot now
    /// persists its checkpoint, and every later boot evaluates against the one the
    /// boot before it left: a short gap re-anchors silently, a long one freezes.
    #[tokio::test]
    async fn w_clock_fresh_store_first_boot_persists_its_checkpoint_and_later_boots_measure_from_it()
     {
        let store = InMemoryStore::new();
        let clock = GameClock {
            ic_ratio: 4,
            epoch_offset: 1_000,
        };
        let policy = OutagePolicy::default();

        // First boot at real 1_000 on an empty store: no prior checkpoint, and the
        // one it records is on the store, not only in RAM.
        let mut first = World::new_minimal();
        hydrate_world_timeline(&mut first, &store, clock)
            .await
            .unwrap();
        let eval = apply_boot_outage_policy(&mut first, &store, 1_000, policy)
            .await
            .unwrap();
        assert_eq!(eval, OutageEvaluation::NoPriorCheckpoint);
        let persisted = load_or_seed_world_timeline(&store, clock).await.unwrap();
        assert_eq!(
            persisted.checkpoint().map(|cp| (cp.real_unix, cp.ic_unix)),
            Some((1_000, 5_000)),
            "the fresh world's first checkpoint must be durable"
        );
        assert!(!persisted.is_frozen());

        // A hot restart 4 s later measures from that checkpoint (< 30 s): a silent
        // re-anchor with zero catch-up, and its own checkpoint persisted in turn.
        let mut second = World::new_minimal();
        hydrate_world_timeline(&mut second, &store, clock)
            .await
            .unwrap();
        let eval = apply_boot_outage_policy(&mut second, &store, 1_004, policy)
            .await
            .unwrap();
        assert_eq!(eval, OutageEvaluation::SilentReanchor { lost_secs: 4 });
        assert!(!second.world_timeline().is_frozen());
        assert_eq!(second.world_ic_now().at, 5_000);
        let persisted = load_or_seed_world_timeline(&store, clock).await.unwrap();
        assert_eq!(
            persisted.checkpoint().map(|cp| cp.real_unix),
            Some(1_004),
            "the second boot's checkpoint must be durable too"
        );

        // The boxes' case: a restart four minutes after the previous boot is an
        // outage (>= 30 s), frozen at that boot's checkpoint for the countdown.
        let mut third = World::new_minimal();
        hydrate_world_timeline(&mut third, &store, clock)
            .await
            .unwrap();
        let eval = apply_boot_outage_policy(&mut third, &store, 1_244, policy)
            .await
            .unwrap();
        let OutageEvaluation::OutageFreeze { lost_secs, freeze } = eval else {
            panic!("expected OutageFreeze, got {eval:?}");
        };
        assert_eq!(lost_secs, 240);
        assert_eq!(freeze.frozen_real_unix, 1_004);
        assert_eq!(freeze.frozen_ic_unix, 5_000);
        assert_eq!(
            freeze.kind,
            WorldClockFreezeKind::OutageCountdown {
                countdown_started_real_unix: 1_244,
                unfreeze_at_real_unix: 1_544,
            }
        );
        assert!(third.world_timeline().is_frozen());
        let persisted = load_or_seed_world_timeline(&store, clock).await.unwrap();
        assert_eq!(persisted.freeze(), Some(freeze));
    }
}
