//! Boot hydration for the engine-owned world in-character timeline.
//!
//! The configuration clock is a genesis seed only.  Once generation one is
//! present, the immutable epoch documents and their materialized head are the
//! authority across restart.  This keeps a later configuration edit from
//! reinterpreting timestamps that were recorded under an older rate.

use std::collections::BTreeMap;

use mush_core::{
    GameClock, MAX_WORLD_TIMELINE_EPOCHS, WORLD_TIMELINE_EPOCH_PREFIX, WORLD_TIMELINE_HEAD_KEY,
    WORLD_TIMELINE_NAMESPACE, World, WorldTimeline, WorldTimelineEpoch, WorldTimelineHead,
    world_timeline_epoch_key, world_timeline_persistence_ops,
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

fn decode_world_timeline(docs: Vec<(String, serde_json::Value)>) -> Result<WorldTimeline, String> {
    if docs.len() > MAX_WORLD_TIMELINE_EPOCHS + 1 {
        return Err(format!(
            "world timeline namespace contains {} documents; maximum is {} epochs plus one head",
            docs.len(),
            MAX_WORLD_TIMELINE_EPOCHS
        ));
    }
    let mut head = None;
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
    let timeline = WorldTimeline::from_epochs(epochs.into_values().collect())
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
}
