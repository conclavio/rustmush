# Activities, maps and learning

The external bot can follow a quiet daily plan and retain lessons, corrections and
proposed character development. All world actions still go through its ordinary
SSH character. No in-game NPC service, hidden character sheet, skill points or
offscreen work system is involved.

## Daily plans

Set `activities = 'daily-plan.json'` in the bot's connection config, or use
`/activity load path.json`. Inspect the loaded plan with `/activity`; enable it
with `/activity on`. Activities run while **listening**, yield to direct engagement
and explicit goals, and resume after the exchange. `/pause` stops further actions;
`/listen` resumes listening. `/activity off` disables the plan and pauses. Loading
a replacement plan pauses and requires explicitly enabling it again.

The [dog-walker plan](../bots/life/dog-walker.json) is a complete JSON example:

- `clock` is `session_monotonic` for ordinary play or `manual_test` for a controlled
  experiment. The latter requires `--test-controls`; `/activity advance 1000`
  advances this local scheduler by 1000 milliseconds. **Neither mode advances the
  server's fictional clock.** Session time continues while paused but excludes
  disconnected time. A certified branch inherits its parent's scheduler offset.
- `period_ms` defines a recurring cycle. Each routine has `start_ms`, `end_ms`,
  `priority`, a remembered `destination`, and optional prerequisite routine IDs.
  Prerequisites must complete in the same cycle. Highest priority selects new work;
  an active procedure keeps its cursor until completed, interrupted, or expired.
  Missed windows are coalesced and do not become a backlog.
- `limits.decision_ms`, `action_ms` and `unsolicited_speech_ms` are separate minimum
  intervals. `max_decisions`, `max_actions` and `max_unsolicited_speech` are retained
  across plan reloads and campaign restore. Unsolicited speech defaults to zero.
  Speech, when allowed, also passes the existing conversation pacing/floor rules.
  Routine bookkeeping makes no model requests; engagement uses the ordinary
  conversation model and its request/spend limits.
- A procedure has an ID, increasing `revision`, an `applicability` description,
  optional exact local episode citations in `source`, `failure: "suspend"`, and
  steps containing `command`, `expect`, and optional `reject` lines. Applicability
  is authored guidance; the routine's destination and prerequisites are executable
  gates. Procedures are candidates for reuse, not universal guarantees.

Travel uses previously observed directed routes. Unknown or invalidated routes
suspend work without repeated `look` requests or blind wandering. Each step needs
complete expected lines in observed command-associated output. Command echoes and
speech (including hanging-indent continuations) do not satisfy a non-speech work
receipt. A communication step confirms its public utterance only. Text matching is
a fixture/game-specific observation contract, not an authoritative engine API.

After rejection or a missing receipt, inspect `/activity`, `/map`, and `/recall`.
Move or investigate through ordinary game commands, then `/activity retry` clears
the suspension and `/listen` resumes. Retry is explicit because an ambiguous
command might already have had an effect. It does not refund action counts.
Changing steps requires a newer procedure revision. Reloading cancels its old
cursor, retains counters/completions and stores bounded revision history. Already
completed work does not automatically rerun because its procedure was edited.

Ordinary reconnect disables activities. An unsettled command is marked
unresumable for inspection, rather than replayed. Certified checkpoints wait for
sent commands to settle and discard unsent intents. In-progress work with receipts
can then continue in the restored world with its cursor and allowances retained.

## Maps

`/map` exposes traveled edges, reported connections, uncertain paths, invalidated
edges and the original reports. `/route <name|#dbref>` uses only traveled edges.
Reverse routes are never inferred. Observed exit rejection invalidates the affected
edge; later successful travel establishes it again. Other changes, including a
reported relocation, can be recorded by a cited `navigation` model annotation.
Reports remain fallible and do not create a traveled path.

## Lessons and corrections

`/learning` shows the local ledger. `/learning guitar` selects relevant lessons;
`/learning save new.json` exports this actor's complete operator-side record.
The learning TUI panel and campaign comparison expose the same acknowledged state.
Export intentionally requires a new path. Review any record before sharing it with
a PC: it can include other lessons known to this actor.

The decision prompt supports an optional `learning` annotation array. It is checked
separately from the game action, so malformed annotations produce feedback without
discarding an otherwise valid command. A lesson annotation looks like:

```json
{
  "type": "lesson",
  "id": "guitar-counterpoint",
  "teacher": {"name": "PC", "kind": "pc"},
  "student": {"name": "Bot", "kind": "external_bot"},
  "topic": "guitar counterpoint",
  "phase": "assess",
  "revision": 1,
  "content": "Ask the student to demonstrate their current pattern.",
  "evidence": [{"episode": 12, "quote": "PC says, \"Show me your counterpoint.\""}]
}
```

Episode IDs and quotes must come from the receiving actor's actual observed
journal. Phases proceed through `assess`, `demonstrate`, `practice`, `feedback`,
then `apply`. Teacher/student/topic remain fixed for the lesson ID. Teacher phases
cite the teacher's observed speech or pose; practice/application cite the student.
Source order is checked even when several utterances share one journal episode.
Transport speaker labels do not independently establish a known in-character name.

A correction is `feedback` at the next revision after practice. The student must
practice that revision and receive feedback before claiming transfer. `apply`
requires an observed location different from practice. Positive feedback does not
erase the corrected instruction. Old attempts remain in history; current instruction
and revision appear in later model contexts along with recent/relevant lessons.
Use `context.learning_bytes` to bound retrieval. Disabling retrieval also removes
the learning context. These checks establish provenance and protocol order; they
do not grade musical quality or make model interpretations infallible.

Participants may be `pc`, `external_bot`, or `game_npc`. Both PC/bot teaching
directions use public speech/poses, and the bot can teach the corrected technique
back. The bot owns its ledger. A PC keeps control of their character; an engine NPC
needs a separate future adapter to persist learning in its own implementation.

## Character development

`/development` shows the policy, proposals and applied overlay. Default `frozen`
retains proposals for inspection without applying them. `suggest` labels proposals
for runner review. `automatic` can apply a small, explicitly permitted change:

```text
/development set {"version":1,"mode":"automatic","allowed_keys":["confidence.guitar"],"max_changes":4,"max_delta":10}
```

Growth annotations contain `type: "growth"`, `subject: {name,kind}`, `key`, `value`,
`rationale`, and exact observed `evidence`. Keys start with `confidence.`, `opinion.`,
or `ambition.`. Values are tendencies from -100 to 100, starting at neutral zero;
the rationale states the proposed concrete change. Automatic application requires
the logged-in external bot as subject, a listed key, an available lifetime change
allowance, and a bounded delta. Other proposals remain suggestions. No policy
applies changes to a PC or game NPC. Temporary `/character-state`, practice history,
this learned overlay, and the authored character JSON remain separate.

## Repeatable demonstrations

```powershell
cargo build -p mush-bot --bin mush-bot --example campaign-fixture
cargo run -p mush-bot -- --campaign bots/life/campaign.json --output .mush-bot/life-demo
cargo run -p mush-bot -- --campaign bots/life/guitar.json --output .mush-bot/guitar-demo
cargo test -p mush-bot --test life
```

The fixed-clock owned fixture offers dog-work and study receipts; these are test
mechanics, not a general RustMUSH economy or skill system. The quiet-life campaign
learns routes, encounters a blocked door, works, answers a PC and studies, then
restores the saved day. The guitar campaign supplies repeatable public exchanges.
The executable SSH test derives citations from those actual observations, feeds
them through separate model tapes, verifies correction-bearing later requests,
records transfer and preserves the PC sheet. Tapes use zero provider spend; live
model pedagogy still needs qualitative evaluation.

Actor checkpoints are version 3. Activity and learning contributions are required
and checked against the journal. Older wave 2/version 1 and wave 3/version 2 saves
retain their original normalized identities and may not smuggle in new state.
All examples use JSON for executable definitions; TOML remains connection config.
