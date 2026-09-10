# Checkpointed campaigns and alternate continuations

Wave 2 adds playable branches in a constrained, owned rehearsal world. Save the
band before a disagreement, try private discussion, public confrontation or an
invitation to demonstrate, then continue the branch you choose. Every branch
runs a new fixture process and independent bot processes connected over SSH.

This first adapter uses RustMUSH core objects and its public SSH transport. It
supports `look`, `help`, `say`, `pose`, `page`, `north`, `south` and a fixed `time`.
The two rooms are Rehearsal Room and Lounge. It has no NPCs, jobs, modules, RNG,
builder commands, scheduled work or other autonomous world producers. It is a
development fixture, not a checkpoint facility for production `mushd`.

## Run the example

Build the bot and the development fixture:

```text
cargo build -p mush-bot --bin mush-bot --example campaign-fixture
target/debug/mush-bot --check-campaign bots/campaign/band.json
target/debug/mush-bot --campaign bots/campaign/band.json --output band-run-01
target/debug/mush-bot --inspect-campaign band-run-01/summary.json --tui
```

Use `.exe` for commands on Windows. The example's `fixture_program` includes
`.exe`; remove that suffix on other platforms, or change the path for a custom
target directory. The fixture binds an ephemeral loopback port, generates a
fresh SSH host key, and supplies the pin to its bots. No existing server,
account or API key is needed. All model responses in this example are local
tapes, with exactly zero provider spend. The private/public branches use exact
commands; the demonstration executes a taped model decision through the ordinary
bot decision loop and speech gate.

The source record creates five completed checkpoints: the pivotal scene, three
children, and a second restore of the pivotal scene. Profiles are JSON, as are
campaign records, tapes and interventions. Existing connection/runner config
remains TOML. Parent paths, profiles and source files resolve relative to their
containing record. Input `/probe` and `/tape` paths resolve relative to the
campaign JSON. Output `/capture` and `/recap` paths are relative to the actor's
new session directory unless absolute.

## Campaign records

`version: 1` records contain a stable `id`, `fixture_program`, optional
`fixture_args`, `max_requests`, an `actors` array, and ordered `branches`.
The fixture program receives its startup JSON path through `MUSH_CAMPAIGN_START`.

Each actor has a unique identifier `name`, a `config` path and a
`request_allowance`. This adapter uses the name as both its fixture account and
character identity. The initial config must supply a local `model_tape`.
Characters, knowledge, affiliations, model catalog, prompts, context, debugger,
conversation policies and budgets use the existing bot configuration.

A branch has a unique `name`, an optional earlier `parent`, optional
`interventions`, and `steps`. Interventions are explicit changes to the inherited
character or experiment; steps are the continuation to execute. Both contain
actor-local controls:

```json
{
  "actor": "Bassist",
  "control": "/send page Lead=Could we talk about the arrangement?",
  "wait": {"kind": "command-result", "contains": "Could we talk"}
}
```

Controls carry an internal ID. A wait can only match an event after that control's
started watermark, so an earlier observation cannot accidentally satisfy it.
Omitting `wait` waits for control application, not completion of a model call or
game command. Use `command-result` for a sent command, `staged-decision` for
`/step model`, and `model-request` when intentionally checkpointing an in-flight
request. Each wait has a bounded deadline. Errors retain the attempt directory
and write `failure.json` with the completed branches and failure reason.

Supported controls include `/goal`, `/role`, `/note`, `/remember`, `/send`,
`/step`, `/resume`, `/continue`, `/pause`, `/listen`, `/ask`, `/probe`,
`/character-state`, `/state`, `/cost`, `/recall`, `/route`, `/capture`, `/recap`,
and two explicit campaign interventions:

- `/tape <file.json>` replaces the remaining response tape and pauses.
- `/turns <1..10000>` sets the remaining goal-turn allowance and pauses. It does
  not reset model requests, receipts, session budget or campaign reservations.

`/continue` resumes the execution status saved before quiescence, including an
unfinished goal or listening session. A deliberately paused parent remains
paused. `/resume` follows the normal bot behavior. Restoring never sends a saved
model response, staged command, or speech intention: those are invalidated at
quiescence and the next decision uses the restored observations.

The initial adapter excludes script mode and the managed cast speaking floor.
Use campaign steps for exact commands. Independent conversation pacing remains
available and is checkpointed. The existing harness still provides floor-managed
live/reconstructed runs; its F2 phase hold is not a world checkpoint.

## What makes a checkpoint usable

The coordinator owns checkpoint controls; they are not model actions. At the end
of a branch it fences each bot's local action ingress, cancels unsent decisions,
retains request accounting, and settles the already-sent SSH command. Each bot
then sends its ordered fixture fence through its own SSH connection.

The fixture serializes all ingress. A fenced actor can continue receiving other
actors' output but cannot act. When all declared actors have fenced, there is no
remaining world producer. The fixture saves its objects, command sequence and
fixed logical clock, then sends the same epoch marker after earlier output on
every connection. A bot saves only after receiving that marker. Missing markers,
disconnects, invalid ingress, unknown adapter capabilities and mismatched epochs
or actor sets refuse certification.

Fence traffic is retained in `checkpoint-wire` operator trace events and excluded
from character memory. Only a complete marker line acknowledges the boundary;
ordinary speech before it remains in order, and unexpected output after it fails
the checkpoint.

The coordinator validates every bot acknowledgement and publishes a combined
`checkpoint.json` only when world and bot state agree. Its content hash binds
the world, each character's state, parent reference, campaign and ledger path.
Unknown schema versions, unknown required fields, corrupt hashes and partial
JSON refuse restoration. Files and output directories are created exclusively;
reusing a destination is an error.

Captured bot state includes the character and affiliations, temporary condition,
goal/instructions/working notes, the full bounded evidence journal, current place
and routes, knowledge grants/documents, attention records and partner timing,
conversation exchanges and counters, remaining tape, model catalog, prompts,
context settings, debugger settings, last exact request capture, probes, budgets,
inherited usage, goal timer and remaining decision delay. Journal evidence IDs
survive; a new namespace ownership header and journal session identify the child.
Transport sessions, decoder state and executed/invalidated work are not replayed.
The checkpoint limit is 24 MB of journal per actor and 128 MB combined.

Cooldowns retain their remaining duration across the offline interval. New SSH
login output is a new child observation, collected before the coordinator starts
continuation controls. Trace elapsed time belongs to the new process; scheduler
time additionally records the inherited elapsed-time offset. World state remains
in the operator artifact and never becomes character knowledge.

## Choose and continue a branch

The TUI shows a branch tree and two continuation panes with recent scenes,
goal/outcome, probes, delta costs and trace paths. Use arrows to select, Space to
set the comparison anchor, and Page Up/Down to scroll. This is an offline viewer;
it does not call models or choose a winner. JSON inspection is available by
omitting `--tui` or supplying `--output comparison.json`.

Record an explicit choice:

```text
target/debug/mush-bot --select-campaign band-run-01/summary.json --branch demonstration --output selected.json
```

Create another campaign record with the same `id`, `max_requests` and cast, set
`from_checkpoint` to the selected checkpoint path, and add new branches. A branch
without `parent` starts from that checkpoint; subsequent branches can name an
earlier branch in the new record. The saved character state is self-contained:
changed or missing original documents cannot overwrite inherited knowledge.
There is no merge of contradictory histories and no automatic selection.

## Accounting and reproducibility

The initial run creates an append-only `accounting.jsonl`. Before each attempted
branch, the coordinator locks it and reserves the sum of actor allowances. Each
bot independently enforces its reserved quota before a model request, even after
goal changes or turn interventions. The campaign cap cannot be raised by
restoring an older checkpoint. Missing or incomplete ledgers refuse continuation;
they are never silently recreated. Concurrent reservations use the same file lock.

Reservations are deliberately conservative: failed and unused allowances are
retained. Per-session cost journals/reports record actual requests by model and
prompt type. Parent histories are referenced once; child reports contain only
their new session's activity. Character budget gates additionally include
inherited usage, so creating a child does not replenish a session limit.

Restored fixture campaigns currently require local tapes. Their provider spend
is exactly zero, including cancelled test requests. Paid models remain supported
by standalone bots and the existing harness; enabling them inside a capped
campaign needs a trustworthy maximum-cost reservation contract. A post-response
OpenRouter receipt alone cannot enforce a hard cap on concurrent requests. This
is recorded in the [wave-2 review](bot-wave2-plan.md), along with the future
production-server adapter boundary.

Artifacts distinguish these operations:

| Mode | Meaning |
| --- | --- |
| `restored_world_branch` | Matched owned world and bot checkpoint, new SSH world copy |
| `reconstructed_run` | Existing harness starts its configured server/setup and runs a new prefix; no saved-world guarantee |
| `model_only_replay` | Existing capture replay asks a model without connecting or sending game commands |

Existing live-server harness runs are labelled `live_world_run`. Tapes control
model replies, not network scheduling. Live model samples are not claimed to be
deterministic or a causal comparison. Poses remain roleplay, not authoritative
skill awards. Production world rollback, managed-floor restoration, arbitrary
server modules/jobs and paid campaign admission are outside this first adapter.
