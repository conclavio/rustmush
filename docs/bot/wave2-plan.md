# Wave 2: checkpointed campaigns

## Implementation plan

Build on wave 1's evidence, conversation pacing and independent SSH actors.
Deliver JSON campaign records, immutable matched checkpoints, isolated child
worlds, explicit interventions, continuation selection, and a branch inspector.

1. Add versioned bot restore contributions for character, goals, instructions,
   memory, knowledge, attention, conversation scheduling, model/tape, prompts,
   debugger and budgets. Preserve counters and remaining timers. Cancel unsent
   decisions at quiescence; never serialize executable pending replies.
2. Define a constrained owned fixture adapter. Its single serialized ingress
   supports a fixed rehearsal world, public speech/movement/private messages,
   a frozen logical clock, and no NPCs, jobs or other autonomous producers.
   Each SSH actor submits an ordered fence after settling local work. Only after
   every member fences may the fixture freeze and broadcast an epoch marker.
   Bot snapshots acknowledge that marker. Validate actor membership, epoch,
   schema and hashes before assembling the immutable combined checkpoint.
3. Run parent and child sessions as separate fixture and bot processes in new
   directories. Reconnect over SSH, restore character-local state, then apply
   JSON interventions. Preserve the parent; a selected child can be the parent
   of another campaign run. No history merging or automatic branch selection.
4. Keep source records, frozen inputs, traces, probes and per-session receipts.
   Reference shared history once. Enforce a campaign-wide request allowance by
   reserving disjoint per-actor quotas before launch. Initially restored fixture
   campaigns support local model tapes only: exact zero provider spend. Paid
   campaign admission needs an enforceable provider reservation contract and
   is a recorded extension point, not a purported hard dollar cap.
5. Provide an offline branch tree and sibling comparison, also in the TUI.
   Label restored-world, reconstructed and model-only artifacts distinctly.
   Exercise the bassist's private/public/demonstration continuations, restore
   the same parent repeatedly, and prove child state cannot leak back.

## Adversarial plan review (before implementation)

- **Database backup is insufficient.** Production RustMUSH has queues, module
  state and scheduled producers not captured by an object dump. Only the
  explicitly constrained adapter is certified. Arbitrary server saves fail.
- **Quiet output is not an output barrier.** Require per-actor in-band ordered
  fences and a common acknowledgement after all ingress is frozen. Timeouts,
  disconnects, missing participants or extra commands after fencing invalidate
  the save. UI snapshots are lossy and cannot certify anything.
- **Restoring delayed replies sends stale actions.** Quiescence invalidates
  model work, staged actions and floor reservations. Preserve the unfinished
  task, attention, counters and remaining cooldowns; decide again on resume.
- **Transport endpoints change between copies.** Bind snapshots to stable
  actor identity and explicit campaign ancestry, then open fresh SSH sessions
  and fresh memory namespaces. Never hydrate one actor from another's journal.
- **Resetting session budgets refunds history.** Carry inherited usage into
  request gates, retain immutable receipts and reserve campaign quotas even
  for failed attempts. Branch turn resets must be explicit interventions.
- **A post-response cost threshold is not a hard spend cap.** Keep paid model
  campaigns unsupported until admission can reserve a trustworthy maximum.
  Existing standalone provider use remains available and honestly labelled.
- **A copied tree is not a playable branch.** Acceptance uses separately booted
  SSH worlds, real bot processes, divergent world actions and parent re-restore.
- **A generic plugin claim can hide unaccounted state.** Pin the first adapter
  schema and capability contract; reject unknown required state/versions.
- **Recovery can silently replace evidence.** Create artifacts exclusively,
  hash normalized content, retain failed directories and append attempt records.
  A half-written checkpoint is never a valid continuation source.

## Decisions and boundaries

- The first fixture is intentionally small, not a production `mushd` rollback.
  It is a development example; the bot keeps zero game runtime dependencies.
- Tapes make decision inputs repeatable; terminal transport timing remains
  observational. No causal ranking or live-provider determinism is claimed.
- Pausing a cast phase is still distinct from campaign quiescence.
- Live paid campaign reservation and a full-server adapter require new
  contracts; neither is inferred from today's database or usage receipts.

## Final design review

Completed as a distinct self-review after implementation. Findings and fixes:

- **Unpolled worker cancellation could reuse a tape reply.** Request admission
  now reserves the tape entry synchronously. An aborted worker cannot leave its
  old reply in the parent's remaining tape. Covered both before worker polling
  and through an in-flight SSH campaign checkpoint.
- **A missing accounting file could reset a restored allowance.** Only a new
  campaign creates a ledger. Continuations require the original file, retain
  failed/unused reservations, reject cap changes, and enforce both inherited
  session usage and independently reserved per-actor quotas. Goal/turn edits
  cannot bypass either gate.
- **Restoration could reread changed source documents.** Initial inputs and
  probe/tape interventions are frozen in each attempt. Restored knowledge,
  profiles, prompts and policy come from typed checkpoint state. Tests continue
  successfully after original character/wiki/tape paths disappear.
- **Elapsed times and identities could be confused across copies.** New trace
  sessions retain separate scheduler offsets; cooldowns keep their remaining
  duration. Epochs include a process ID and atomic sequence, avoiding Windows
  wall-clock resolution collisions. Campaign/branch scope is rebound explicitly.
- **Copied grades could appear to be new branch results.** Inherited probe
  history is retained but marked separately; comparison reports count new probes.
- **Protocol lines appeared in character memory in the CLI demonstration.**
  Quiescent output now retains fence traffic only in the operator trace. Ordinary
  speech remains ordered and enters the character journal, while complete-line,
  matching-epoch validation excludes administrative metadata and refuses any
  unexpected output after the boundary. Fragmentation and stale-epoch tests cover
  this distinction, and the campaign test checks all restored character journals.
- **Unbounded shutdown or oversized saves could leave ambiguous artifacts.**
  Shutdown drains have deadlines; combined saves enforce the same 128 MB bound
  as their loader. Actor/report/epoch membership and all hashes are validated
  before inspection/restoration. Raw game commands cannot submit fixture fences.
- **Display output could conceal the experiment boundary.** Reports distinguish
  restored worlds, reconstructed runs, model-only replay and live-server runs.
  The comparison TUI displays the tree, sibling scenes, new requests/costs/probes
  and trace paths, sanitizes terminal controls, and handles tiny terminals.

The constrained adapter satisfies its checkpoint contract: complete declared
world state, ordered output acknowledgement, independent restored actors and
append-only campaign admission. No production game/NPC runtime dependency was
introduced into the bot. Managed-floor and production-server checkpoint adapters
and paid-provider campaign admission remain explicit capability boundaries,
not silently degraded saves.

Validation before integration: all 221 bot tests passed, including private/public/
demonstration branches, parent re-restore, chosen-child continuation, stale staged
and in-flight reply cancellation, inherited budgets, missing-ledger refusal,
tampered/unsupported checkpoints, and terminal rendering. Added a focused
cooldown/delay restoration regression during the final pass. Formatting, Clippy
with warnings denied, diff checks and the normal/build dependency graph passed.
Final integration validation against `origin/main` at `403f695`: all **224** bot
tests pass (138 unit and 86 integration/offline/helper tests). The complete CLI
band example boots five separate four-actor worlds, exports its comparison, and
records an explicit demonstration-branch selection, with zero provider spend.
Its saved character histories exclude checkpoint protocol text. The development
example builds, Clippy passes with warnings denied, formatting/diff checks pass,
and `mush-bot` remains the only RustMUSH package in the normal/build dependency
graph. Validation uses a separate target directory, without touching live games.
