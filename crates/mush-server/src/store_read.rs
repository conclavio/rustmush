//! H1 — the off-loop half of the module store-read seam
//! (`docs/design/11-h-implementation-plan.md` §3.1).
//!
//! A module queues [`Effect::StoreRead`](mush_core::Effect::StoreRead) through
//! `Ctx::read_store`; the world task hands the effect here with the store
//! handle it holds, and this performs the bounded read **off the world loop**
//! and produces the [`StoreReadResult`] the typed
//! `WorldCommand::StoreReadDone` carries back. The world loop never waits on
//! a store read.
//!
//! Three bounds, all decided here and never by the module: one
//! `module_json_get` per requested key (never a namespace listing), at most
//! [`MAX_STORE_READ_BYTES`] of encoded documents (past it the result is
//! `Oversized` with **nothing** returned), and [`STORE_READ_TIMEOUT_MS`] for
//! the whole read (past it the result is `Failed`). A read is a view: it is
//! never retried; the requester asks again.

use std::io;
use std::time::Duration;

use futures::future::BoxFuture;
use mush_core::{
    MAX_STORE_READ_BYTES, MAX_STORE_READ_KEYS, STORE_READ_TIMEOUT_MS, StoreReadResult,
};
use mush_store::{ObjectStore, StoreError};
use smol_str::SmolStr;

/// Perform one bounded read of `keys` from module namespace `ns`.
pub async fn perform_store_read<'k>(
    store: &'k dyn ObjectStore,
    ns: &'k str,
    keys: &'k [SmolStr],
) -> StoreReadResult {
    perform_store_read_within(
        store,
        ns,
        keys,
        Duration::from_millis(STORE_READ_TIMEOUT_MS),
    )
    .await
}

/// [`perform_store_read`] with an explicit deadline.
pub async fn perform_store_read_within<'k>(
    store: &'k dyn ObjectStore,
    ns: &'k str,
    keys: &'k [SmolStr],
    timeout: Duration,
) -> StoreReadResult {
    read_within(|key| store.module_json_get(ns, key), keys, timeout).await
}

/// The read itself, over any per-key fetch — the seam the timeout and bound
/// tests drive without a second store implementation.
async fn read_within<'k, F>(fetch: F, keys: &'k [SmolStr], timeout: Duration) -> StoreReadResult
where
    F: Fn(&'k str) -> BoxFuture<'k, Result<Option<serde_json::Value>, StoreError>>,
{
    // The engine bounds this before the effect exists; a driver that sees
    // more is a programming error, and it still must not run an unbounded
    // loop against the store.
    if keys.len() > MAX_STORE_READ_KEYS {
        return StoreReadResult::Failed(format!(
            "store read names {} keys, more than the {MAX_STORE_READ_KEYS} allowed",
            keys.len()
        ));
    }
    let read = async {
        let mut docs = Vec::with_capacity(keys.len());
        let mut absent = Vec::new();
        let mut bytes = 0usize;
        for key in keys {
            match fetch(key.as_str()).await {
                Ok(Some(value)) => {
                    let remaining = MAX_STORE_READ_BYTES.saturating_sub(bytes);
                    match encoded_len_within(&value, remaining) {
                        Some(len) => bytes = bytes.saturating_add(len),
                        None => {
                            return StoreReadResult::Oversized {
                                bytes: bytes.saturating_add(remaining).saturating_add(1),
                            };
                        }
                    }
                    docs.push((key.clone(), value));
                }
                Ok(None) => absent.push(key.clone()),
                Err(error) => {
                    return StoreReadResult::Failed(format!("store read of {key} failed: {error}"));
                }
            }
        }
        StoreReadResult::Docs { docs, absent }
    };
    match tokio::time::timeout(timeout, read).await {
        Ok(result) => result,
        Err(_) => StoreReadResult::Failed(format!(
            "store read timed out after {} ms",
            timeout.as_millis()
        )),
    }
}

/// The encoded JSON length of `value` if it is at most `limit` bytes, else
/// `None` — counted through a sink that stops at the limit, so an oversized
/// document is never fully re-encoded.
fn encoded_len_within(value: &serde_json::Value, limit: usize) -> Option<usize> {
    struct Counter {
        seen: usize,
        limit: usize,
    }
    impl io::Write for Counter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.seen = self.seen.saturating_add(buf.len());
            if self.seen > self.limit {
                return Err(io::Error::other("over the read limit"));
            }
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter { seen: 0, limit };
    serde_json::to_writer(&mut counter, value)
        .ok()
        .map(|()| counter.seen)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::FutureExt as _;
    use mush_core::{ModuleOp, ModuleWrite};
    use mush_store::{InMemoryStore, WriteBatch};

    async fn store_with(docs: Vec<(&str, serde_json::Value)>) -> InMemoryStore {
        let store = InMemoryStore::new();
        let module_ops = docs
            .into_iter()
            .map(|(key, value)| ModuleOp {
                ns: SmolStr::new("scenes"),
                op: ModuleWrite::JsonSet {
                    key: SmolStr::new(key),
                    value,
                },
            })
            .collect();
        store
            .write(WriteBatch {
                serial: 1,
                module_ops,
                ..WriteBatch::default()
            })
            .await
            .expect("seed write");
        store
    }

    fn keys(names: &[&str]) -> Vec<SmolStr> {
        names.iter().map(SmolStr::new).collect()
    }

    #[tokio::test]
    async fn h1_reads_present_docs_and_reports_absent_keys_in_request_order() {
        let store = store_with(vec![
            ("log:7:0", serde_json::json!({ "chunk": 0 })),
            ("log:7:2", serde_json::json!({ "chunk": 2 })),
        ])
        .await;
        let wanted = keys(&["log:7:0", "log:7:1", "log:7:2"]);
        let result = perform_store_read(&store, "scenes", &wanted).await;
        let StoreReadResult::Docs { docs, absent } = result else {
            panic!("expected docs: {result:?}");
        };
        assert_eq!(
            docs.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
            vec!["log:7:0", "log:7:2"]
        );
        assert_eq!(docs[1].1["chunk"], 2);
        assert_eq!(absent, keys(&["log:7:1"]));
    }

    #[tokio::test]
    async fn h1_a_read_never_crosses_namespaces() {
        let store = store_with(vec![("log:7:0", serde_json::json!({ "chunk": 0 }))]).await;
        let wanted = keys(&["log:7:0"]);
        let result = perform_store_read(&store, "canon", &wanted).await;
        assert_eq!(
            result,
            StoreReadResult::Docs {
                docs: Vec::new(),
                absent: wanted
            }
        );
    }

    #[tokio::test]
    async fn h1_an_oversized_result_returns_nothing_but_the_verdict() {
        let big = "x".repeat(MAX_STORE_READ_BYTES / 2 + 1024);
        let store = store_with(vec![
            ("a", serde_json::json!({ "pad": big })),
            ("b", serde_json::json!({ "pad": big })),
        ])
        .await;
        let both = keys(&["a", "b"]);
        let result = perform_store_read(&store, "scenes", &both).await;
        assert!(
            matches!(result, StoreReadResult::Oversized { bytes } if bytes > MAX_STORE_READ_BYTES),
            "{result:?}"
        );
        // One of them alone is fine.
        let one = keys(&["a"]);
        let result = perform_store_read(&store, "scenes", &one).await;
        assert!(matches!(result, StoreReadResult::Docs { .. }), "{result:?}");
    }

    #[tokio::test]
    async fn h1_too_many_keys_is_refused_by_the_driver_too() {
        let store = store_with(Vec::new()).await;
        let many: Vec<SmolStr> = (0..=MAX_STORE_READ_KEYS)
            .map(|i| SmolStr::new(format!("k{i}")))
            .collect();
        let result = perform_store_read(&store, "scenes", &many).await;
        assert!(matches!(result, StoreReadResult::Failed(_)), "{result:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn h1_a_read_that_never_answers_fails_closed_at_the_deadline() {
        // A fetch that never resolves: the deadline wins, the result is
        // `Failed`, and nothing is returned — the world loop was never
        // involved either way.
        let wanted = keys(&["a"]);
        let result = read_within(
            |_key| futures::future::pending().boxed(),
            &wanted,
            Duration::from_millis(50),
        )
        .await;
        assert!(
            matches!(result, StoreReadResult::Failed(ref d) if d.contains("timed out")),
            "{result:?}"
        );
        // A store error is `Failed` with the key named, and stops the read.
        let wanted = keys(&["a", "b"]);
        let result = read_within(
            |_key| async { Err(StoreError::Invalid("boom".to_owned())) }.boxed(),
            &wanted,
            Duration::from_millis(50),
        )
        .await;
        assert!(
            matches!(result, StoreReadResult::Failed(ref d) if d.contains("a failed")),
            "{result:?}"
        );
    }

    #[test]
    fn h1_encoded_len_stops_counting_at_the_limit() {
        let value = serde_json::json!({ "pad": "y".repeat(1000) });
        assert!(encoded_len_within(&value, 10).is_none());
        let exact = serde_json::to_vec(&value).unwrap().len();
        assert_eq!(encoded_len_within(&value, exact), Some(exact));
        assert!(encoded_len_within(&value, exact - 1).is_none());
    }
}
