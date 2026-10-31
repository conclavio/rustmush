# Recorded IPC frames (doc 33 §4.1)

One directory per boundary protocol version, `v<N>/`, holding one
length-prefixed frame per sample of `tests/samples/mod.rs`, in two
directions:

- `to_server/<Variant>-<nn>.frame`, the gateway → server frames
  (`mush_ipc::ToServer`);
- `to_gateway/<Variant>-<nn>.frame`, the server → gateway frames
  (`mush_ipc::ToGateway`).

A set is written once, by the build whose `IPC_VERSION` is N, and never
edited by hand:

```
cargo run -p mush-ipc --example record_golden_frames
```

The recorder is an example binary, not an `#[ignore]`d test: a tool that
writes into the tree proves nothing by passing, and the gated-proof inventory
in `mush-contract-tests` reads an ignored test as a proof some CI job must run.

`tests/golden.rs` checks that the current version's set is byte-for-byte what
the build encodes (re-record after any deliberate, additive wire change; the
diff of this directory is then the record of the change) and that the previous
version's set still decodes (the one-step compatibility window).
`crates/mush-contract-tests/tests/ipc_golden_frames.rs` fails a bump that has
no set. Sets older than the window stay as history.
