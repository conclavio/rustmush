# K-sequence pulse-adapter review follow-up

Scope: the three defects reproduced on `673db4c`; implementation started from
`f7bc801` and was brought current with `a18b345` before final verification.
This is not an E7k authority migration or a new combat rules model.

## Contact authority (C3, C8, C9)

`tactical_intent` keeps its single public intent parameter. Targeted intents now
carry both an opaque contact id and its observed generation. Commands obtain
that generation from their existing contact projection; structured callers are
held to the same contract. Missing generations fail closed, including previously
queued targeted intents from an older build. Non-targeted intents carry neither.

`ActionProvider::resolved_authority_witnesses` is a default-empty, read-only
callback in the common resolver. The exact ready provider selects witness
requests; the kernel issues them through the registered authority owner, checks
the bounds, and adds their guards to the final pre-commit guard set. Readiness
is checked on both sides of the callback. `Engine::authority_witness` exposes
the same owner-validated read to internal adapters without a foreign family
handle or a new contact owner.

Sim selects only audiences allowed by its existing crew/proxy policy. This
matters because a vehicle, rather than its human pilot, can hold the contact.
Tactics still supplies the sole authoritative contact validator. Only the
selected contact is checked: another contact changing does not invalidate an
unrelated order. No list of contact truth or audience snapshots is copied into
each durable intent. At tick time the captured contact generation and current
audience access are checked again; a stale display cannot authorize targeting,
and loss/reacquisition cannot resurrect an old intent. Proxy abandonment also
refuses in the shared provider and at tick time, not just in `+eject`.

## Retention progress (C2, C11)

`sim/tick_reap_progress` is a new typed family, one bounded record per active
domain. It holds the opaque cursors returned by the commit, window and order
family scans. Its post-image commits with the tick's removals and kernel-seeded
schedule advance. An abort advances none of them; boot hydrates the cursors
through the normal typed-state path. Disarming removes the progress record.

Each family scan visits at most 256 records and removes at most 64 per fire.
Reaching the removal cap resumes after the last **visited** record, not the end
of an incompletely visited page. Exhausting a family wraps its cursor. Thus a
prefix of other domains' live records cannot permanently hide expired records.
This is eventual bounded-work cleanup, not an instantaneous 64-record RAM cap;
scan latency can temporarily retain additional history. The measurement now
includes progress, the schedule, and all three maximum removal sets: 453 replay
writes (44% of the kernel cap) at 64 units, still below the 60% admission budget.

## Resolution keys (C4, C5)

The resolving window retains its pinned key version and records the resolution
under that version. A newly opened successor pins the **current** key instead.
Continuous ticking therefore releases the old unresolved-window reference
instead of copying it forever. Existing open windows are never re-keyed.

## Regression evidence

`crates/mush-sim/tests/pulse_contracts/mod.rs`, included by the E7c8 target:

- Foreign/missing contacts and missing generations refuse structured ingress.
- A proxy's live movement offer cannot be turned into an abandonment intent.
- A platform-held contact produces a kernel guard; loss/reacquisition fails both
  final guard validation and the later tick despite an unchanged display projection.
- Rotating 3 to 4 resolves the old window under 3 and subsequent windows under 4.
- Five active domains with a 256-record live prefix: cleanup resumes past the
  prefix, an aborted tick advances nothing, the cursor hydrates after boot, and
  a 100-record expired suffix is removed in 64/36 batches without skipping.

The journal/SpatialState exemptions, tactical Window guard migration item, and
outbox emitter-owner binding remain the separately tracked E7k/K follow-ups.

Verification commands (offline, all features):

```text
cargo test -p mush-core -p mush-sim -p mush-contract-tests -p mush-server --all-features --offline -j 2
cargo clippy -p mush-core -p mush-sim -p mush-server -p mush-contract-tests --all-targets --all-features --offline -j 2 -- -D warnings
```

Result: 1,203 tests passed, 10 ignored, no failures; strict lint and scoped
rustfmt checks passed. These runs do not exercise a live Redis backend or the
opt-in scale harness.
