# Conversation, speaking floors and scene observation

Wave 1 adds two gates to the standalone SSH player. An idle bot first waits for
relevant attention, a quiet speech burst, a request cooldown, and room in its
rolling social request allowance. After a model prepares speech, the entire
decision waits for pacing and an utterance allowance before applying memory or
sending anything. New terminal output cancels an unsent reply. New parsed speech,
including speech without the bot's name, cancels a social model request.

The existing character profile and memory still drive what it says. Temporal
exchange threads, participant labels, addressed-to-self event IDs and question
candidates help inspect what it heard. Thread grouping is heuristic; punctuation
does not prove a question is unresolved, and acknowledging attention does not
certify that it was answered. Interface labels do not prove acquaintance.

## Independent players

```powershell
cargo run -p mush-bot -- --config mush-bot.local.toml --conversation bots/conversation/independent.json --goal "" --tui
```

The default policy also works without a file. It coalesces 600 ms of speech,
holds replies for 700–2000 ms, and permits 12 idle social requests and eight
utterances per 60 seconds. Delays use a fixed algorithm with the configured seed,
actor login and intent counter. These are starting values to tune for your game.
The existing attention cooldown also applies; a policy cannot shorten it.
Active goals also wait for a quiet speech burst before their next autonomous
request, while retaining their goal and session allowances.

`/conversation` inspects state. `/conversation load policy.json`,
`/conversation save new.json`, and `/conversation set {"version":1,...}` use JSON.
Loading or editing pauses and cancels pending work. Counts and paid requests are
retained; changing the configured rolling window deliberately changes its scope.
Campaign, branch and scene IDs are fixed for the session; start another session
to change them. Connection configuration remains TOML, with an optional
`conversation = "policy.json"` selector relative to that file.

Use F11 to edit the policy in the TUI, and F9 to reach the Conversation panel.
It shows the phase, reason, queued command, source request, observations used,
thread candidates, pacing deadline and session costs. The runner's `/ask`,
structured probes and active goal decisions keep their existing budget rules.
The rolling social request limit also counts cancelled social requests. A denied
social eligibility check makes no provider request. `/step` is an explicit model
request and still observes session limits; a committed speech decision still
passes through the speech gate.

The paced commands are `say`, `pose`, `semipose`, `whisper`, `page`, `@emit`, and
their leading `"`, `:`, `;` shortcuts. Unknown server-defined command aliases
cannot be identified as speech by this client. Explicit `/send`, unprefixed
operator commands, bootstrap and exact JSON scripts retain their direct command
semantics. The bot neither sees nor predicts that a human is typing.

## Managed casts

Add `"speaking_floor": {}` to a harness manifest to enable cooperative scheduling.
See [the three-player example](../bots/harness/social-floor.json). Configure its
accounts, SSH credentials, host key and model through your ordinary local config;
the named characters must already exist and be together in the game. The example
uses live models, so outcomes are sampled and costs use the normal ledger.

```powershell
cargo run -p mush-bot -- --harness bots/harness/social-floor.json --check-harness
cargo run -p mush-bot -- --harness bots/harness/social-floor.json --output .mush-bot/social-001 --tui
```

All managed actors must declare the same campaign, branch and scene scope. This
is one runner-declared floor for the cast, not a hidden engine room detector. A
slow actor requests a place only once its reply is prepared and locally eligible;
model latency does not reserve the floor. Direct address receives priority. A
speaker cannot monopolize consecutive turns while others wait, and the oldest
aged request outranks newer directed requests.

The parent sends a grant through local stdin and receives completion through
ordered JSON stdout. The lease holds until a matching acknowledgment, followed
by the speaking window. Expiry requests revocation; it does **not** immediately
give someone else a grant. Missing revocation acknowledgment stops the cast.
Disconnect/error handling stops further grants and preserves reports. This
serializes recognized autonomous client speech dispatches; it does not guarantee
server arrival spacing, control other clients, or apply to explicit operator
sends. Floor metadata contains actor/intent/timing/priority only and never enters
a character prompt as knowledge or authority.

The cast TUI has a Conversation and floor panel, plus actor activity in the roster.
Policies are frozen into each run directory. `run.json` and `summary.json` retain
floor state, initial policies and independent speech measurements; policy content
and floor mode distinguish comparison variants. Removing `speaking_floor` gives
an independent comparison. Its near-simultaneous rate counts independent dispatch
events received by the harness within 1800 ms after another actor's event. It is
null when no independent speech was measured. This is an explicitly limited
proxy, not a measurement of human interruption or server-side collision.

## Spectator and chronicler

Use [spectator.json](../bots/conversation/spectator.json) with `--goal ""` for an
observer. It connects as an ordinary SSH character and makes no autonomous model
requests. Position it using local game commands. Runner questions remain available
when a model is configured; those are deliberate paid requests.

`/recap new.json` creates a local, extractive recap without a provider call. Each
entry includes the exact observed quote, journal episode/session ID, time, and
observed place/speaker labels. It includes at most the latest 64 current-session
observed episodes, each capped at 1800 characters, and reports omissions and
excerpting. It excludes authored priors, inferred beliefs, previous visits and
other actors' journals. Private speech actually received by this character may
appear. Recaps are never automatically announced in the game.

Coverage is always marked incomplete: the observer has one character's received
view and cannot account for events before login, while absent, or offscreen.
This first chronicler preserves excerpts rather than generating paraphrased prose.
Optional generated summaries and separately labeled whole-cast runner recaps are
future extensions.

## Bot bookmarks

After `/pause` and after command output settles, `/bookmark new.json` records a
bot bookmark. It rejects staged decisions, model work and outbound intentions.
The artifact has a content-derived ID, versioned required contributions, SHA-256
integrity checks, scene identity, session clock, output sequence and observation/
speech watermarks. Contributions include character, runner state, memory journal,
attention buffers, conversation policy/scheduling history, knowledge and prompt
configuration, and cost references. JSON serialization is round-trip validated.

```powershell
cargo run -p mush-bot -- --check-bookmark .mush-bot/bookmark.json
```

This command validates the explicitly supplied artifact offline without dotenv,
credentials or a connection. Unknown required contribution versions, corrupt
checksums and oversized bookmarks are rejected. The artifact limit is 32 MB,
with a 24 MB memory-journal limit. All artifact writes require a new path.

A bookmark is an inspection foundation for wave 2. There is no restoration
command: it freezes neither the world nor remote queues, cannot reconnect a
pending SSH operation, and never refunds recorded costs. A cast F2 phase hold also
leaves bots and the world running. Matched world/bot checkpoints need the separate
wave-2 quiescence and restoration protocol.

## Deterministic regression demo

```powershell
cargo test -p mush-bot --test e2e conversation:: --offline
```

The fixture starts a real game server, three bot players and a PC on separate SSH
connections, with local HTTP stubs or JSON model tapes. It checks silence, pacing,
corrections, private context, slow-model cancellation, floor revocation, missing
acknowledgments, disconnects, observer recaps and bookmark validation. No live
model/provider account is needed. `--test-controls` additionally permits
`/fault floor-grants` and `/fault floor-controls` in managed sessions to simulate
a stalled grant receiver or a receiver that cannot acknowledge revocation.

See the [implementation plan and reviews](bot-wave1-plan.md) and
[five-wave roadmap](bot-social-campaign-plan.md).
