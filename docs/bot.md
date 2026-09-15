# The standalone player bot

`mush-bot` is an external SSH player and test runner. Run it in its own terminal:
the terminal displays its game session, chosen commands, concise action summaries,
and local control messages. Its production dependency graph contains no RustMUSH
engine, store, gateway, or world protocol. The server can run on another machine.

[Conversation and scene observation](bot-conversation.md) covers reply pacing,
managed speaking floors, the spectator, cited recaps and bot bookmarks.

[Checkpointed campaigns](bot-campaigns.md) adds an owned rehearsal-world adapter,
JSON branches, character restoration, continuation selection and a comparison TUI.

[Social memory and group agency](bot-social.md) adds scoped background retrieval,
directed relationships, attributed claims, observed commitments and perspective inspection.

[Adversarial playtests and usability feedback](bot-playtesting.md) adds bounded
missions, feedback/refutation, reproduction reduction and blind scene ratings.

## Intentional independence from NPC agents

The external bot and the in-game NPC system are two independent approaches to
agent behavior. Their overlap is intentional: they should be developed and tested
against each other without sharing the implementation being evaluated.

The bot owns its model client, provider protocol, prompts, decisions, memory,
knowledge retrieval, scheduling, limits and replay scripts. It is not built on
`mush-npc`, `mush-llm`, or the game's worldgen pipeline. It interacts with NPCs and
the rest of the world through the same SSH commands and observations as a player.
The shared Cargo workspace and bundled player documentation do not change this
runtime boundary.

General-purpose libraries and model providers may overlap; the agent logic stays
separate. Compare the approaches with common world fixtures and observable success
criteria, rather than sharing planners, memory, internal action APIs or hidden
world state. A bot can run while server-side NPC/LLM features are disabled. If its
task uses `@gen`, that particular game command still requires the server's generation
service, just as it does for a human builder; the bot's own decisions remain external.

Game crates in `mush-bot`'s **development dependencies** only stand up the server
under test. In particular, the `mush-llm` mock provides the test server's `@gen`
responses. The actual bot executable talks to that server over SSH and makes its
own model calls to a separate HTTP stub. None of those server dependencies are in
the bot's runtime or build dependency graph. The package's
[maintenance instructions](../crates/mush-bot/AGENTS.md) preserve this separation.

The first version uses RustMUSH's **SSH line interface** with a real PTY. TUI
screen navigation is a future adapter; `@tui` is rejected so a terminal repaint
cannot silently corrupt the bot's observations. The runner shows explicit action
summaries and remembered state, not private model reasoning.

## Start a bot

Build `cargo build -p mush-bot` and run `target/debug/mush-bot` (`.exe` on Windows),
or install with `cargo install --path crates/mush-bot`. The installed binary carries
all `docs/help/*.md` topics from the build. It does not need the checkout at runtime.

1. Create a normal game account and character through the usual player onboarding.
   Prefer a separate account per simultaneously connected bot. Configure the username
   as `account+Character` to skip a multi-character selection menu. Chargen gates and
   character permissions still apply.
2. Copy [mush-bot.toml.example](../mush-bot.toml.example) to `mush-bot.local.toml`.
   Set SSH host, account, authentication and model endpoint/name. Passwords, key
   passphrases and provider keys resolve from named environment variables. The bot
   loads the nearest `.env` in the current working directory or its ancestors,
   preserving variables already set in the shell. For example, launching from
   `codexprojs/rustmush` finds `rustmush/.env`, or `codexprojs/.env` if the nearer
   file is absent. Discovery follows the working directory, not the config path;
   files are not merged. A missing `.env` is fine; an invalid or unreadable file
   stops startup without printing its contents. `--help`, `--check-script` and `--check-character`
   do not load `.env`.
3. Verify the server key through a normal SSH connection, which records it in your
   `~/.ssh/known_hosts`, or configure a fingerprint verified with the server operator.
   The bot refuses unknown or changed keys and never auto-trusts them.
4. Launch in a terminal:

```powershell
$env:MUSH_BOT_PASSWORD = "your-game-password"
# Only needed if OPENROUTER_API_KEY is not already in your shell or .env:
# $env:OPENROUTER_API_KEY = "your-provider-key"
cargo run -p mush-bot -- --config mush-bot.local.toml --goal "Find a flower shop and buy a rose" --transcript rose-session.jsonl
```

Alternatively register a public key in-game with `@sshkey/add` and set `identity_file`.
The client reads an OpenSSH private key directly; it does not invoke a system shell,
consult `ssh_config`, or use an SSH agent. Password auth occurs inside SSH, before
terminal observations or model calls. In-band credential commands are rejected to
keep credentials out of transcripts. Account and session verbs (`@sshkey`,
`@newchar`, `QUIT`, `LOGOUT`) are rejected as well, whatever their origin: game
text reaches the model as untrusted data, and a line another player wrote must
not be able to register a key on the bot's account or end its session. Keys,
characters and disconnects are managed from the runner. `play`/`guest` without credentials reaches the
welcome screen, not an authenticated character; autonomous play should use an account.

The model must support a chat-completions endpoint and text JSON responses. The bot
implements its own HTTP adapter, request/response types, configuration and credential
handling. It does not use the game's provider adapters or LLM service. A scenario
does not construct a model client or require API credentials, even if a `[model]`
block exists in the config.

Model decisions require an action and a short summary. An omitted or null `memory`
preserves the bot's existing notes; an explicit empty string clears them. If a
`wait` decision omits `wait_ms`, the bot waits one second while continuing to
receive game output. Invalid actions, commands and field values still stop
autonomous play. In an interactive session, `/resume` continues from the current
observations. A delayed `@gen` draft can still arrive while the bot is paused,
because generation runs on the game server.

## Model profiles and switching

The existing `[model]` configuration remains available as the `configured`
profile. For runtime selection, copy [bots/models/example.json](../bots/models/example.json),
replace its model ID placeholders and set the top-level TOML key
`model_profiles = "bots/models/example.json"`. Each JSON profile has its own
endpoint, model ID, credential environment-variable name, token limit, timeout
and optional temperature. Omit temperature for the provider's default.

```json
{
  "version": 1,
  "default": "fast",
  "profiles": {
    "fast": {
      "endpoint": "https://openrouter.ai/api/v1/chat/completions",
      "name": "YOUR_MODEL_ID",
      "api_key_env": "OPENROUTER_API_KEY",
      "max_tokens": 2048
    }
  }
}
```

Catalog files are JSON only, at most 128 KB, with 1..64 profiles. Profile names
use letters, digits, `_` and `-`; `configured` is reserved for the TOML model.
The JSON `default` selects the starting profile. A top-level TOML `model_profile`
overrides it, and CLI `--model <profile>` overrides that. CLI
`--models <profiles.json>` chooses the catalog file. Config paths are relative
to the TOML file; CLI paths are relative to the launch directory.

```text
mush-bot --config mush-bot.local.toml --models bots/models/example.json --model careful
/model list
/model use fast
/resume
```

`/model` shows the active selection. `/model use` validates the selected profile
and resolves its credentials before making a change, then cancels any pending
decision and pauses. `/resume`, `/listen` or `/step` continues with that model.
Already sent game commands still finish. The character, working notes, durable
experiences, role, instructions, goal and remaining run budget are retained.
Unknown profiles and unavailable credentials leave the current selection intact.
Profile files load at startup; model selection does not rewrite the source file.

Each model request records its selected profile and settings, request/session
IDs, purpose and a SHA-256 revision of the system instructions. These are in
`model-request` transcript events and the cost ledger, so switching models does
not misattribute a previous request. Raw API keys are resolved only inside the
client and are absent from these records.

## Session and lifetime costs

`/cost` shows this process session and all recorded sessions for this bot,
including breakdowns by resolved response model (requested model when unavailable),
profile and request purpose. Current purposes are `goal_decision`, `conversation`
and `runner_question`. A recall followed by another decision belongs to that
decision's purpose; local `/recall` and `/knowledge` controls make no model calls.
Memory annotations produced inside a decision share its cost.
`/state` and the final JSON report include the same accounting data.

OpenRouter returns the account charge in `usage.cost`, with upstream inference
cost reported separately. Costs are denominated in USD. The bot records those
amounts rather than estimating them from token prices or adding the upstream
amount to the account charge. The current API returns usage automatically; no
extra request flag or billing API call is needed.
See [usage accounting](https://openrouter.ai/docs/cookbook/administration/usage-accounting)
and [credit units](https://openrouter.ai/docs/faq#credit-and-billing-systems).

The report's `provider_cost_by_currency` contains known account charges;
`upstream_inference_cost_by_currency` is a separate measure, useful when examining
BYOK usage. Other chat-completions endpoints can set `cost_currency = "USD"`
(or another three-letter currency) when their `usage.cost` has that meaning.
Without a known currency, a returned amount is retained but not counted as a
priced request. Currency totals are kept separate.

Costs use decimal arithmetic with up to 18 fractional places, and totals serialize
as decimal strings to preserve precision in downstream tools. Missing, invalid,
or unrepresentable amounts count as `unknown_cost_requests`; a provider-reported
zero is a known zero. Token counts, generation IDs, resolved models and available
caching/reasoning usage are also retained.

Truncated output, invalid decisions and HTTP errors still retain any cost present
in a parsed response. Cancellation prevents the response from acting; a receipt
already received remains counted. If cancellation, a timeout, invalid response
JSON or a crash prevents the cost arriving, the request remains unpriced and may
still have incurred a provider charge. The report exposes pending, interrupted
and cancelled counts so a partial total is not mistaken for a complete bill;
a cancellation without a receipt is `cancelled_unpriced_requests`, kept apart
from `unknown_cost_requests` so a session budget does not block on it.
There is no automatic generation-ID reconciliation yet.

Accounting is enabled independently of memory persistence, under `.mush-bot/costs`
relative to the config. Each SSH host/port/username and `memory.namespace` has its
own append-only JSONL ledger, protected against concurrent writers. Starts and
receipts are flushed to disk; incomplete or corrupt ledgers are rejected rather
than silently reducing the total. `costs.max_bytes` bounds the file; the runner
reserves room for a receipt before starting a request. `costs.persistent = false`
keeps session-only accounting for disposable fixtures.

Changing goals or models does not reset cost totals. “All sessions” means all
sessions captured in this ledger, starting when tracking was enabled, rather
than the whole OpenRouter account. Old transcripts are not imported automatically.
Server-side `@gen` costs and other applications using the same API key are outside
this bot's own model accounting. A future harness can report server costs alongside
bot costs without sharing their agent implementations.

Optional [session budgets](bot-budgets.md) gate model requests using these ledger
totals. `--budget bots/budgets/session.json` sets request and exact spending
thresholds; `/budget` inspects them and F10 edits them in the local TUI. Goal and
model changes retain session usage. Spending limits block the next request after
receipts reach a threshold; they cannot cap an already-running request's charge.

## Control the running session

Local stdin is a REPL. A model call runs asynchronously, so state inspection and
pause/goal changes remain available while it is thinking or waiting for the game.

| Input | Effect |
| --- | --- |
| `/state` | Role, goal, memory, recent terminal output, counters, limits, pending command |
| `/goal Find the station` | Replace the goal, reset its model/time budget, pause |
| `/role A tired courier` | Change role and pause |
| `/note Avoid the northern gate` | Add an instruction and pause |
| `/resume` | Run autonomously |
| `/pause` | Cancel a pending model request and pause |
| `/step` | Execute one model decision, then remain paused |
| `/ask Why are you at the market?` | Pause and answer from state; sends no game command |
| `/send look` or `look` | Pause and pass one line to the game |
| `/knowledge flower shop` | Search the reference documents locally |
| `/recall Marco` | Search durable experiences and belief evidence without a model call |
| `/recall #42` | Retrieve a particular journal record and nearby evidence |
| `/remember Marco is an old friend whose name I know` | Save prior character knowledge and pause |
| `/route Flower Shop` | Show a previously travelled path from the currently observed room |
| `/character` | Inspect the authored biography, motivations, opinions and voice |
| `/attention` | Inspect recent speech, attention cues and the handled cursor |
| `/listen` | Listen and respond without pursuing the stored goal; `/pause` stops |
| `/model` or `/model list` | Inspect the current model or configured profiles |
| `/model use careful` | Switch to a named profile and pause, retaining memory and budget |
| `/cost` | Show session and all-session costs, grouped by model, profile and purpose |
| `/budget` | Show session limits, remaining amounts and blocking reasons; load/save/set JSON policies |
| `/help` | List controls |
| `/quit` | Disconnect and save the result |

Changing a goal or sending a command cancels any pending model decision. Commands
already sent cannot be undone. While collecting a command's response, passthrough
asks the runner to retry shortly, preserving one-command-at-a-time observations.
`--paused` starts with bootstrap commands only. With a model, attention enabled and
no goal, the bot listens for engagement. Otherwise it starts paused. After completion,
interactive mode stays connected and, by default, can respond to conversation.
Blockers stop autonomous action until the runner resumes it.
EOF or Ctrl-C closes the session; headless mode ignores stdin entirely.

## Character, personality and voice

The [character and cast studio](bot-studio.md) develops JSON sheets from a concept:
generate ten numbered alternatives, select or blend them in the REPL/TUI, then
export profiles and a runnable harness with individual goals and briefings.

Use an authored character sheet alongside the goal and the experience journal.
The sheet says **who this person is**; the journal records **what has happened and
what they believe**; the goal says **what they are trying to do now**. Personality
influences choices and speech within the runner's goal. A favourite team does not
automatically become a task to investigate sports.

Character sheets are JSON files. Start with
[rosebot.json](../bots/characters/rosebot.json). Greyhaven and the Harbor Rooks are
fictional example backstory: adapt them to the setting before use. A small sheet
with just `name` and `voice.style`
also works. Unspecified biography remains unknown instead of being invented in
conversation.

```text
cargo run -p mush-bot -- --character bots/characters/rosebot.json --check-character
cargo run -p mush-bot -- --character bots/characters/rosebot.json --goal "Find a flower shop and buy a rose"
```

Alternatively set `character = "bots/characters/rosebot.json"` at the **top level**
of the TOML config. Config paths are relative to that file; the CLI override is
relative to the launch directory. `--check-character` validates and prints the
normalized sheet without config, credentials, memory writes or a connection.
The profile is loaded at startup; restart after editing it. Reports and transcript
events retain the normalized profile used for the run.

The fields deliberately describe more than a list of adjectives:

| Fields | Purpose |
| --- | --- |
| `name`, `aliases`, `pronouns`, `identity` | Self-description and names that might get their attention |
| `appearance` | Authored physical description, clothing and bearing; does not change the server description |
| `hometown`, `background`, `relationships`, `prior_knowledge` | Authored personal history and things already known |
| `personality`, `values`, `motivations`, `tensions` | Disposition, priorities, desires and competing impulses |
| `opinions` | A topic, stance and reason; useful for conversation and disagreement |
| `voice` | Style, occasional mannerisms, things to avoid, and situation/line examples |
| `conversation` | Approach, starters, follow-ups and social boundaries |
| `notes` | Additional nuance in prose |

An opinion with a reason gives the bot something to talk about: “I support the
Rooks because my father took me to games” can lead to a conversation about family,
home or loyalty. Voice examples demonstrate rhythm and restraint, without becoming
lines to repeat. Tensions such as wanting company but fearing intrusion help the
model choose how to behave in context. Voice here means written dialogue, not audio.

JSON uses these fields directly; put additional prose in `notes`:

```json
{
  "name": "Rosebot",
  "aliases": ["Rosie"],
  "voice": {"style": "Warm, plain speech. Leave space for a reply."},
  "notes": "Reserved at first, but interested when someone describes their work."
}
```

Only `.json` files are accepted for character sheets. Unknown fields fail
validation, so misspellings are not silently ignored. Input
is limited to 64 KB and the normalized profile to 32 KB. `version` defaults to 1.
Without a sheet, the bot uses its SSH character name and the existing role.
Use `account+Character` to identify the actual character explicitly; an account
name alone may not be the character's name.

The name, hometown, background, relationships and explicit prior knowledge also
enter the durable journal as authored prior knowledge, deduplicated on restart.
They are not observations proving facts about the world. The complete current
profile accompanies each decision and `/ask`; the model can reflect new experiences
in working notes and evidence-backed beliefs without rewriting the source file.
Editing the profile does not erase earlier journal records. A different character
or restarted campaign should use its own login or memory namespace.

## Hearing, attention and conversation

Every model request includes a dedicated window of recent says, whispers and pages,
alongside the last completed working notes and retrieved experiences. Room prose
and help listings do not evict this speech window. It defaults to 64 utterances
within approximately 24 KB; older dialogue remains in the experience journal for
recall. `/attention` shows the records, timestamps, speaker labels, cues and which
snapshot has been handled. The window is local to the current connection.

Name/alias mentions, direct whispers/pages and configurable phrases such as
“excuse me” invite an attention decision. After a reply, speech from that speaker
can continue the conversation for 120 seconds without repeating the bot's name.
These are heuristics for possible engagement: the model considers the whole
exchange and can choose silence when someone is talking about it, addressing
someone else, or ending a conversation. Speaker labels remain interface evidence,
not automatic in-character name knowledge. Own speech and OOC/channel speech are
retained where recognized but do not trigger in-character replies. General poses
and nonverbal gestures are not yet attention triggers.

While working on a goal, new engagement cancels an **unsent** model decision and
refreshes it after the configured cooldown with current speech and the last
completed memory. It does not roll back an already sent command. A decision only
acknowledges the speech snapshot it actually saw, so newer utterances remain
pending. Conversation-based retrieval searches the engagement text, and the model
can take a local `recall` turn to find older evidence before answering.

With `attention.listen_when_idle = true`, an interactive bot whose goal is complete
can respond without restarting that goal or replacing its completion result.
`/listen` explicitly enters this mode while retaining any goal as context;
`/resume` returns to pursuing that goal, or listens if there is no goal. Idle bots
make no model calls until engagement arrives. Starters guide a model when it is
already deciding; there is no periodic unsolicited small-talk timer.

`/pause`, `/ask`, `/note`, `/role` and manual commands still pause autonomous action.
Speech continues to be recorded while paused and can be considered after resuming.
`/step` allows exactly one decision. `attention.enabled = false` disables automatic
attention wakeups and decision refreshes, while ordinary turns still receive recent
speech. `/listen` explicitly enables attention for the current process.
Headless runs exit at goal completion, and deterministic scripts never add model
replies to their recorded command sequence.

The cooldown coalesces bursts of engagement. Attention decisions and cancelled
requests consume the existing `max_steps` budget; waking up does not replenish it.
`max_seconds` bounds a goal or an active conversation decision/recall/inspection
chain. Idle listening time does not run down a conversation timer. Provider errors
or exhausted limits pause action as usual. Deciding whether and how to answer is
still model work, so the cues and profile do not guarantee perfect social judgment.

Where the bot believes it is comes only from the room the game shows at
connect and from the answer to its own `look`, walk or teleport. A room header
that arrives unsolicited, in another player's
narration or a builder's description, never moves it; the game's own rules
make every say, pose, whisper and page carry the speaker's name, so an
unattributed line is staff narration or authored prose, not evidence of place.

## Experiences, beliefs and identity

Memory is enabled by default. Each SSH host/port/username and `memory.namespace`
has its own append-only JSONL journal under `.mush-bot/memory`, relative to the
config file. It survives goal changes and process restarts. Use `account+Character`
to keep characters distinct; change the namespace when resetting a world or
starting a different campaign. An explicit `memory.file` can choose the filename.
The journal checks ownership and permits only one writer. Files are gitignored.
`memory.persistent = false` keeps an in-process journal for disposable tests.

The bot records sent commands as **attempts**, received terminal text as
**observations**, and `/remember` entries as **prior character knowledge**.
Records retain timestamps, session IDs, observed locations and quoted dialogue,
including conversations the bot overhears. A temporal link to a command is not
proof that the command caused later ambient output. PTY echoes are excluded from
observations. Password authentication and API keys never enter this journal.
Working notes remain a concise model-written summary; clearing them does not
erase the journal. Existing sessions/transcripts from before this feature are
not imported automatically.

Decisions may also add structured `beliefs`: subject, predicate, value, kind,
basis, confidence, and exact quotations with episode IDs. For example, a payment
belief should retain the particular transaction, amount and currency, rather than
reducing it to “bought something.” Basis distinguishes observation, testimony,
inference and prior knowledge. Confidence is tentative, likely or strong, not a
claim of objective truth. Commands and speech about payments are not confirmed
transactions. Invalid evidence references/quotes produce feedback without saving
the claim. Conflicting claims remain visible; an explicit `supersedes` revision
retires a claim about the same subject/predicate while preserving its history.

**Speaker labels do not establish name knowledge.** Seeing `Marco says, "Hello"`
records a labelled utterance, without automatically creating a `known_name`
belief. The character may already know Marco from prior knowledge or an earlier
encounter, so absent name evidence means **unestablished**, not “definitely a
stranger.” Identity beliefs require a quotation containing the name in an utterance
or prior knowledge; speaker prefixes, rosters and emote attribution are insufficient.
Other recognition clues can be stored as tentative fact beliefs. An
introduction is testimony and could be an alias or lie. Evidence validation checks
provenance and quotations; interpreting their meaning remains fallible model work.

Successful observed walks create directed route records with their command and
room evidence. `/route` finds paths through these travelled edges, including named
exits, without guessing a reverse exit. Looking at another room and teleporting do
not establish a walked route. Reconnection first observes the current room again.
Routes describe past travel; locks, renamed exits and changed worlds may invalidate
them, and the bot must respond to new observations.

Each model turn retrieves relevant journal records within `memory.recall_bytes`.
`/ask` searches using the **question**, so older purchases or conversations can be
found after leaving recent terminal history. Autonomous decisions can use
`action: "recall"` with `query` to search again, inspect `#record-id`, or request an
exact place name and its route. These calls consume the ordinary model turn
budget. `/recall` and `/route` are local, read-only controls with no model charge.
Retrieval is lexical and bounded, not exhaustive or semantic embedding search;
use names, distinctive terms, or record IDs when narrowing a search.

The journal is flushed to disk as observations settle and before model requests.
It is bounded by `memory.max_bytes` (64 MB by default); reaching the limit stops
the run rather than silently losing evidence. An incomplete or corrupt journal
is rejected without overwriting it. Reports and `/state` show its path and counts.

## Knowledge and behavior

The binary starts with the help topics and a source-derived primer for movement,
inspection, communication, inventory and world creation. It also executes `help`
and `look` at startup. Live help is saved as reference material; the agent can read command and
topic help through the same interface as a player.

Add Markdown/text files or directories to `knowledge` for setting documents, wiki
exports or a game-specific player guide. Paths are relative to the config file;
symlinks are skipped. Loading is bounded to 1 MB per file and 16 MB overall.
The bot splits documents into chunks and retrieves up to six relevant chunks each
turn. This is lexical retrieval; export web/wiki pages to `.md` or `.txt` first.
Knowledge supplies information, not hidden world state or extra powers.

Each model turn receives the role, goal, character profile, runner instructions,
recent speech, a bounded terminal history, relevant reference text, working notes
and retrieved experiences. It returns one command, a local memory recall, a bounded
wait, completion, or a blocker. The prompt
requires evidence for completion and treats game text and documents as untrusted
data. Runtime validation permits only one terminal line with no control characters;
server-side account permissions remain the authority over game actions.

A flower-shop goal exercises both the bot and the world's implementation. RustMUSH
currently has core `get`, `drop`, `give`, and `inventory`, but no generic core `buy`
command. The bot must discover the game's shop/NPC mechanics and report a blocker if
the world cannot sell a rose. Picking one up or posing a purchase is not a completed
purchase. The integration fixture tests finding a shop and **acquiring** a rose.

SSH line output has no request IDs or universal end-of-response marker. Ordinary
turns use a configurable quiet interval and observation cap; delayed actions may
need explicit model waits or a longer interval. Ambient events continue to be shown
and added to history even while paused or during model calls. No reconnection or
automatic command replay occurs on a disconnect.

## Building with a capable character

Give the bot a builder character and a creation goal. There is no separate bot-side
building toggle: `@dig`, `@gen` and their related commands go through the same SSH
command path, and the game checks permissions on every action. `@dig` needs the
`BUILDER` flag; opening exits also needs control of the source room. `@gen` needs
`BUILDER_LLM`, control of the target room, and the server's LLM features enabled with
available budget. The bot's model configuration and the server's generation model
configuration are independent.

```text
mush-bot --config builder-bot.toml --role "A world builder following the game's setting and house style" --goal "Build a Rose Garden north of this room with a south exit back. Use @gen to describe it, review and accept the draft, then verify the description and both exits."
```

The bot knows that `@dig` creates topology and `@gen` drafts descriptions. It is
instructed to record created dbrefs, wait for asynchronous drafts, review their
contents, retry or edit when needed, accept suitable drafts itself, and inspect
the resulting world. A creation goal authorizes this review-and-accept workflow;
a request for drafts only leaves them uncommitted. `@gen/building` additionally
requires the rooms and exits to exist first, then a brief review followed by room
reviews. The bot follows permission denials and reports disabled generation or
budget limits as blockers, preserving any partial progress in its memory.

An ordinary play goal such as buying a rose still means playing in the existing
world. Give an explicit building role, goal or instruction when you want the bot
to create missing content.

## Repeatable JSON scripts

A bot script is a source-controlled record of exact game commands, their order,
expected output and timeouts. `--script` accepts `.json` files;
`--scenario` remains an alias. Scripts run headlessly without a bot model or API
key. They include builder commands such as `@dig` and `@gen` when the character
has permission. Server-side `@gen`
still uses the game's configured generation model and budget.

Repository examples:

- [smoke.json](../bots/scripts/smoke.json): repeatable, read-only command-help
  checks. No world reset or builder permissions are needed, including for `help @dig`.
- [rose-garden.json](../bots/scripts/rose-garden.json): dig, generate, wait for the
  draft, accept and walk both exits.

For the garden script, start in a room named Market Square that the character
controls, with no north exit or existing Rose Garden. The character needs BUILDER
and BUILDER_LLM, and server generation must be enabled with available budget.
Reset this disposable fixture before replaying. The generation step waits for the
actual draft, with a 120-second timeout, before the script accepts its description.
Generated wording can vary; this fixed recipe checks output but does not ask the
bot model to review prose. Use an autonomous building goal for model review and revision.

A minimal script looks like this:

```json
{
  "version": 1,
  "name": "Inventory help",
  "steps": [
    {
      "command": "help inventory",
      "expect": ["List everything you are carrying"],
      "reject": ["No help available"],
      "timeout_ms": 10000
    }
  ]
}
```

`version` defaults to `1`, preserving older JSON scenario records. `reject`
defaults to empty and `timeout_ms` defaults to 10000. Use arrays for multiple
`expect` or `reject` substrings. Strings use normal JSON escaping; decoded commands
are sent literally, so `#dbrefs`, colons, exit aliases and `@gen` prose retain their
meaning. Each command must fit on one terminal line without control characters.
Malformed JSON, duplicate or unknown fields, invalid commands and missing positive
expectations fail validation before any SSH connection. Scripts are limited to 1 MB.
Keep explanatory documentation in Markdown alongside the JSON source records.

Validate and inspect the normalized record without config, credentials or a server:

```text
mush-bot --script bots/scripts/rose-garden.json --check-script
```

Run the same record repeatedly, using new artifact filenames per run:

```text
mush-bot --config bot-test.toml --script bots/scripts/smoke.json --transcript run-001.jsonl --report run-001.json
mush-bot --config bot-test.toml --script bots/scripts/smoke.json --transcript run-002.jsonl --report run-002.json
```

The report's `script` field and the transcript's `script` event contain the full
normalized input record, including defaulted values. They identify exactly which
commands and checks produced the result even if the source file later changes.
The connection config supplies host/authentication and run/quiet-period limits.
Script mode skips `agent.bootstrap`: include any desired `help`, `look` or setup
commands in the script itself. It sends exactly the commands listed, with no model
decisions, goal-based additions or automatic retries.

Scripts repeat actions; outcomes still depend on initial world state, other players
and generation responses. Reset the fixture before rerunning a creation script:
`@dig` is not idempotent. An `@gen/accept` step commits the draft after the listed
assertions, without model review. Use an autonomous role and goal when you want the
bot to decide what to do, review prose and adapt. A model's `done` is a model-reported
result; script assertions provide explicit checks on player-visible output.

Script assertions search only the output collected after that step begins,
excluding lines that exactly echo its command. All `expect` substrings must appear;
any `reject` substring fails the step. A step advances after a quiet interval once
expectations match, or at its deadline if they match. A later unsolicited event
belongs to the next observation. An omitted/null `command` waits for new output.
At least one nonempty positive expectation is required per step; there is no
negative-only "nothing happened" assertion. Steps begin after collecting login output.

`max_steps` bounds model requests (including questions, waits and cancelled calls)
or script steps. `max_seconds` bounds each goal's wall time, including pauses.
A limit or model/provider error stops autonomous action. `/goal` resets the goal
budget while preserving discoveries; `/resume` does not replenish it.

Exit codes are **0** for completion (or interactive quit), **1** for errors and
failed assertions, and **2** for a headless blocker/incomplete run or exhausted
budget. Reports include status, outcome, counters, memory, recent output and elapsed
time. JSONL events record observations, commands, controls, decisions, provider
usage when available, assertions and the final result. Artifact paths must be new
files. Transcripts contain session/game text and should be handled accordingly.

The package tests launch the actual bot executable against real production SSH,
session and world wiring on an ephemeral port, with ordinary-player and scoped-builder
fixtures in memory. A local fake model endpoint makes bot turns deterministic and
asserts that actions follow observations. Builder tests exercise real `@dig`, the
asynchronous `@gen` draft/retry/accept path with a mock server-side provider, and
verification of descriptions and exits. Permission, ownership and disabled-generation
checks remain enforced. No external model, Redis or live game is needed:

```text
cargo test -p mush-bot
```

## Local terminal UI

```text
cargo run -p mush-bot -- --config mush-bot.local.toml --tui --paused
```

The local UI displays the live SSH game session, runner events, active model and
session spend. The inspector has state, costs, working memory/attention, prompts,
context, debug settings, session budgets and local query results. The input box accepts the same
controls and player commands as the REPL. The bot still uses SSH line mode; this
interface is independent of the game's own TUI.

| Key | Action |
| --- | --- |
| F1 | Help in the result inspector |
| F2 | Pause/resume |
| F3 | Select a configured model with arrows and Enter |
| F4 | Edit the prompt profile as JSON |
| Shift+F4 / Ctrl+F4 | Edit decision/question system text directly |
| F5 | Produce a model decision and hold it |
| F6 / F7 | Edit context/debug settings as JSON |
| F8 | Commit a held decision |
| F9 | Cycle inspector panels |
| F10 | Edit session request and spending limits as JSON |
| PgUp/PgDn | Scroll game output |
| Ctrl+PgUp/PgDn | Scroll the inspector |
| Ctrl+S / Esc | Apply an editor's contents / cancel editing |
| Ctrl+Q | Disconnect and exit |

Opening an editor pauses autonomous play. Applying validated settings retains the
goal, experience journal and usage counters. Changes are live session settings;
use `/prompt save new.json`, `/context save new.json`, `/debug save new.json` or
`/budget save new.json`
to create source records. These commands never overwrite an existing file. JSON
editors accept multiline paste; multiline game-command paste is rejected. The
terminal is restored when the UI closes or returns an error. Pipes should use the
REPL, `--json-events`, or headless mode instead of `--tui`.

## Prompts, captures and decision replay

Set `prompts = "path.json"` in the connection config or pass `--prompts path.json`.
See [careful-observer.json](../bots/prompts/careful-observer.json). A version 1
profile has a `name`, optional `decision_system` and `question_system` replacements,
and a `supplement` appended to both modes. Null replacements select the bundled
instructions. Replacements change the complete base system message, including
the usual protocol/personality/memory guidance; command validation and actual
server permissions still apply.

`/prompt` shows both effective system messages and their SHA-256 revisions.
`/prompt load path.json` hot-loads a profile; `/prompt set {"name":"brief","supplement":"Keep replies short."}`
applies an inline JSON profile. Invalid settings leave the current profile active.
Loading valid settings cancels an unsent decision and pauses. Full system text
can also be edited directly in the TUI without manually escaping JSON newlines.

Every model request now records `model-input`: the exact HTTP JSON body, prompt
revision, full request revision and question/decision mode. It contains the
supplied character, terminal context, speech, retrieval results and instructions.
The corresponding `model-reply` contains response text and any usable-response
error. It excludes provider-private reasoning fields and authorization headers.
Request IDs join these records to decisions, cancellations and cost receipts;
`command_sequence` joins sent commands to their observations and settled
`command-result` records. The latter exclude exact PTY command echoes.

```text
/capture decision-001.json
mush-bot --config mush-bot.local.toml --replay decision-001.json --report replay-001.json
mush-bot --config mush-bot.local.toml --replay decision-001.json --model careful --report replay-002.json
```

`/capture` saves the last request, including one still running or subsequently
cancelled. Replay opens no SSH connection and never applies the returned command
or memory. Without `--model`, it sends the captured body unchanged to the active
configured endpoint. With `--model`, it substitutes that profile's model ID,
token limit and temperature while retaining all captured messages. Credentials
always come from the current configuration/environment. The replay report includes
both revisions, parsed decision or error, response, zero commands sent, and costs
under `decision_replay`. A modified capture fails hash validation before a request.

Transcripts and captures contain the context actually supplied to the model,
including game conversations and runner-authored text. Request capture is kept in
memory unless exported or recorded with `--transcript`; harness runs always record
it. Existing cost journals remain separate from these larger debugging artifacts.

## Breakpoints, watches and context experiments

`--debug path.json`, config `debug`, and `/debug load|save|set` use this shape:

```json
{"version":1,"after_model":false,"before_commands":["@dig","@gen/accept"],"observations":["Permission denied"]}
```

Patterns are case-sensitive literal substrings. `after_model` holds every action
decision; `before_commands` holds matching model-generated commands. A held
decision has not yet applied its memory annotations or sent its command.
`observations` watches incoming game output, including fragmented matches, and
pauses/cancels any unsent decision. Manual commands and deterministic scripts do
not pass through model-command breakpoints.

`/step` retains its original behavior: one model turn, including its action.
`/step model` instead holds the resulting decision. `/commit` or `/step command`
applies it, provided game output has not changed and the run's time budget remains.
Commands, waits and recalls leave the bot paused; done/blocked decisions retain
their usual outcome handling (including configured idle listening after done).
Fresh output invalidates a held decision; `/resume` discards
it and replans. `/step observation` pauses and waits for an already sent command's
output to settle. SSH observations keep arriving while paused; the world itself
does not pause.

`--context path.json`, config `context`, and `/context load|save|set` use
[full.json](../bots/context/full.json) or
[no-retrieval.json](../bots/context/no-retrieval.json). They independently control:

- `history_bytes`: terminal history supplied to the model, up to the configured
  agent history ceiling.
- `speech_limit` / `speech_bytes`: newest utterances included in the separate
  speech field, up to the attention collector's limits.
- `recall_bytes`: retrieved record budget, capped by `memory.recall_bytes`.
- `retrieval`, `routes`, `references`, `working_memory`, `character`: include or
  remove the corresponding context fields.

These settings filter the supplied view; they never erase the journal. Each
channel is independent: disabling retrieval leaves working notes and recent
history available, disabling the character field does not remove biography
already seeded as prior knowledge, and disabling retrieved references leaves
the bundled system command primer and any help in recent history. Attention
collection/detection continues even if speech is hidden from the model. Combine
settings explicitly for broader ablations. Every request captures its settings.

## Coordinated casts and experiment matrices

```text
mush-bot --harness bots/harness/conversation.json --check-harness
mush-bot --harness my-cast.json --output new-cast-results
```

[conversation.json](../bots/harness/conversation.json) is a four-run interruption
example. Before running it, point its actors at your connection configuration and
create/login the two game accounts/characters in the same room. It uses local
model tapes, so it tests scheduling and conversation delivery without a model API.
Replace tapes with real model profiles for behavioral experiments. Offline
`--check-harness` validates all referenced inputs and matrix combinations without
connecting or loading `.env`.

A harness manifest contains `version: 1`, `name`, `actors`, optional sequential
`steps`, an `assertions` contract, `timeout_ms`, and optional `matrix` and `server`.
Paths are relative to the manifest. Each actor names its own connection `config`
and may override `login` (`account+character`), `goal`, `role`, `character`, `model`
profile, `prompts`, `context`, `debug`, `model_tape` or command `script`.

Each actor runs a separate bot executable and authenticated SSH session. It gets
a new memory directory inside its run artifacts; authored prior knowledge is
still seeded normally. Costs continue to use its configured durable ledger.
Actors using the same ledger owner concurrently must use distinct configured cost
directories; ordinary distinct character logins naturally have separate owners.
There is no automatic world reset for runs against an existing server.

Non-script actors start paused. The harness waits for all actors' bootstrap/login
output to settle, then sends each actor's optional `start` control, such as
`/resume` or `/listen`. Script actors execute their scripts headlessly and cannot
receive controls. `expected_status` defaults to `done`; use `paused` for a driven
stimulus actor, or `blocked`/`error` for an intentional failure case. A control
error fails the run. Final actor reports and process exit codes are checked.
For `expected_status: "listening"`, the harness records the listening state before
shutdown and accepts the ordinary paused state after `/quit`. Script actors can
expect done, blocked (exit 2), or error (exit 1).

```json
{
  "actor":"marco",
  "control":"/send say Rosebot, are you there?",
  "expect":[{"actor":"rose","kind":"send","contains":"say Hello"}],
  "reject":[{"actor":"rose","kind":"send","contains":"stale-action"}],
  "timeout_ms":10000
}
```

Steps send an optional local control and wait for all their expectations.
Omitting targets/control creates an event barrier. For targeted steps, a runner
acknowledgment marks the start before effects and confirms the control applied;
queued events preceding that actor's start marker cannot satisfy the step.
A control-only step waits for acknowledgments. Use an outcome expectation when
game or model completion must precede the next control. This lets another
player speak while a model request is pending, a builder change a location, or a
runner change a goal/profile at a reproducible event boundary.

The run-wide contract has `eventually`, `ordered`, and `forbidden` matcher arrays.
Matchers select `kind`, optional `actor`, optional literal `contains`, and/or a
`pointer` into `event.data` paired with a typed `equals` value (including null).
Observation substring matches span SSH fragments but never cross actors. Ordered
matches require successive events in the specified order. Forbidden matches stay
active through normal shutdown. Reports retain the matching actor and event
sequence, remaining requirements, and failure evidence.

Assert `observation` or settled `command-result` evidence for world outcomes;
assert `send` to forbid or require actions. A model's `done` status alone is not
independent evidence. The harness test suite includes a model claiming it reached
a shop without visiting it, and requires the harness to fail that case.

`matrix` names one target `actor`, arrays of `models`, `prompts`, `characters`,
`contexts`, and `repeats`. It runs the Cartesian product sequentially; omitted or
empty dimensions retain the actor's settings. Limits are 16 actors, 256 runs and
100 repeats. Each run saves its effective cast, joined JSONL trace, per-actor
trace/report/stderr, and `run.json`. `summary.json` compares status, command/model
counts and session costs with links to full run reports. A failed run retains its
artifacts and later cases continue; Ctrl+C stops the matrix. The harness asks bots
to persist their final state and cancellations before terminating unresponsive
owned processes. The local TUI currently displays one bot; the cast harness is
headless, ready for a later multi-pane workspace.

## Failure injection and optional server startup

`--model-tape path.json` (or config/actor `model_tape`) substitutes deterministic
local responses for provider calls. A tape has `version: 1` and ordered `replies`:
each has optional `delay_ms` and exactly one of `content` or `error`. Content is
the raw response text, so it can deliberately contain invalid decision JSON.
Cancellation consumes the delayed entry before waiting; the next request sees
the next entry. Exhaustion produces a model error. Synthetic returned receipts
are marked `local-test-tape` with zero cost; cancelled requests without a returned
receipt retain the ordinary unresolved accounting status. Reports identify a
simulated model, and traces include the tape input. Runtime model switches keep
the tape active for that session.

`--test-controls` enables `/fault disconnect`, which closes the bot's SSH transport
and exercises normal connection-loss handling. A harness actor opts in with
`"test_controls": true`. Public game actions, speech, goal replacement and model
changes need no test-only controls. Generation delays and closed shops can be
provided by the server fixture or builder actors through ordinary commands.

An optional `server` object starts a foreground server-under-test process for
each run, before starting any player:

```json
{
  "program":"C:/path/to/mushd.exe",
  "args":["--config","fixture.toml"],
  "working_directory":"C:/path/to/fixture",
  "ready_output":"your fixture's ready message on stdout",
  "timeout_ms":60000
}
```

The harness passes an argument vector directly, inherits the runner environment,
waits for the specified stdout substring, captures stdout/stderr, and stops that
owned process after the run. Launch the server executable in the foreground;
background descendants are not managed. A fresh process does not itself reset a
persistent game database: the fixture must arrange its world state and accounts.
Omit `server` to attach to an already running game. The bot and harness never use
NPC memory or privileged engine APIs; comparisons with in-game NPCs use the same
public observations and assertions.

## Composed prompts and inspection

Enable the component composer with a JSON prompt profile:

```text
cargo run -p mush-bot -- --tui --paused --prompts bots/prompts/composed-observer.json
```

`--prompts <json> --check-prompts` validates and prints the normalized profile
without loading credentials or opening SSH. Existing prompt profiles without a
`composition` field keep the original system/context/question arrangement.

`composition: {}` enables the default components. Each component has an `id`,
either a context JSON `pointer` or runner-authored `text`, `enabled`, `required`,
`order`, `priority`, `max_bytes`, and `purposes`. Purposes are `goal`,
`conversation`, and `runner_question`; an empty list means all. If supplied,
`components` replaces the default list. Inspect the normalized example before
editing so desired context is not omitted accidentally.

Required components are allocated first, followed by descending priority; final
message order uses ascending `order`, with authored order breaking ties. Pointer
components are JSON data in user messages; literal text components are system
instructions. This version uses fixed placements, not arbitrary chat-depth or
executable extension hooks. Raw terminal history can be shortened at a UTF-8
boundary. Structured speech and evidence are atomic. An oversized required
component blocks preparation and leaves the REPL available to adjust settings.

`max_input_bytes` bounds actual UTF-8 message content. Displayed token usage is an
estimate using bytes divided by three, excluding provider framing. It is not an
exact tokenizer or assurance that the provider's context window fits. Exact
provider usage and costs remain attached to the request receipts.

The default composition protects the current objective, runner instructions and
recent speech before optional older context. It includes group membership,
character, temporary condition, working notes, evidence, routes, references and
control metadata when available. Existing context ablations still apply first.

`/inspect` shows the last dispatched request and whether its inputs have become
stale. `/inspect preview` prepares the current decision input without a provider
call; `/inspect preview <question>` previews a runner question. Previews do not
consume turns, alter memory or replace the last dispatched capture. In the local
TUI, F9 cycles to **Assembly**, which contains component sources, inclusion and
exclusion reasons, byte sizes, hashes and actual messages. **Groups** shows the
current actor's affiliations.

Composed captures use version 2 and retain their provenance with an integrity
hash. `/capture <new.json>` and `--replay` support both legacy and composed
captures. Model overrides preserve messages. The inspection concerns the prepared
API body; external providers may perform their own formatting. Replay reproduces
the request, not guaranteed model output.

### Lore eligibility

The composition's `lore` array accepts entries like:

```json
{
  "id": "marco-acquaintance",
  "text": "Marco taught me bookbinding years ago.",
  "known_by": ["Rosebot"],
  "keywords": ["Marco"],
  "purposes": ["conversation", "runner_question"],
  "order": 500,
  "priority": 10
}
```

Eligibility is checked before relevance: either `common_knowledge: true` or an
explicit `known_by` character name is required. With neither grant, the text is
excluded even if its keyword matches. Empty keywords activate eligible entries
unconditionally. Matching is case-insensitive literal matching against goal,
runner question and recent utterance content; speaker labels, OOC speech and self
echoes do not activate entries. This initial implementation has no regex,
recursion or automatic learned grants. A lore match selects context; it does not
schedule a reply. Author-only information must also stay out of supplied reference
directories, whose established retrieval behavior is unchanged.

### Temporary character state and voice selection

```text
/character-state set {"mood":"wary","intent":"Check the directions before leaving.","concerns":["The guide sounded unsure."],"remaining_turns":3}
/character-state
/character-state clear
/context set {"voice_examples":3}
```

Changing a condition pauses and invalidates pending decisions. Applied decisions
consume its lifetime; questions, previews, cancelled work and held decisions do
not. Expiration clears it and emits `character-state-expired`. Omitting
`remaining_turns` keeps the condition until cleared or session end. It is separate
from permanent biography and recorded in snapshots/reports. Learned automatic
mood updates are future work.

`voice_examples` limits the character's examples by lexical relevance of their
situation to the goal and speech, retaining authored order after selection. Zero
removes them; null retains all. The source sheet is never rewritten. The example
profile is `bots/context/selected-voice.json`.

## Organized casts and group phases

`bots/harness/organized-party.json` provides a two-member example. Set its logins
and config path for your game. `groups` define an `id`, display `name`, free-form
`kind`, `mission`, optional `values` and `shared_knowledge`, and `members`. Each
member identifies an actor, role, responsibilities and optional private briefing.
The same schema can describe a Garou pack, Kindred coterie, mage chantry,
mercenary faction or adventuring party. These names confer no game powers.

Each member receives the mission, public roster/roles, explicit shared starting
knowledge, and only their own responsibilities and briefing. Other members'
private instructions and memories are not merged into their context. Shared
knowledge is seeded as authored prior evidence. Harness actor IDs are not assumed
to be in-character names. Overlapping group memberships are supported.

The harness writes each member's derived `affiliations.json` alongside their
trace. A standalone session can load that format with `--affiliations <json>` or
the `affiliations` config path; `/group` inspects it. A grouped harness actor's
config must not also specify affiliations, so no configuration is silently
overridden. Each run gets fresh isolated memories as before.

```json
{
  "name": "Regroup after scouting",
  "group": "lanterns",
  "dispatch": "serial",
  "control": "/send west",
  "expect": [{"kind": "command-result", "text_pointer": "/output", "contains": "Market Square(#"}],
  "timeout_ms": 15000
}
```

Choose one target form: `actor`, `actors`, or `group`. For group/actors targets,
expectations without an actor are expanded to every selected member. Explicit
matcher actors must belong to that selection. A named phase completes after all
members acknowledge the control and meet their expectations. Omitting `control`
creates a targeted event barrier with a read-only state fence. Script-mode actors
can participate in run-wide/global assertions but cannot receive targeted controls.

`dispatch` defaults to `parallel`: send to all members before waiting. `serial`
waits for each member's acknowledgment and its expected outcome before sending to
the next, in roster/selection order. Use serial phases for deterministic fixture
actions when the world temporarily rejects concurrent commits; use parallel phases
to test contention or simultaneous engagement. Neither mode automatically retries
a failed game command. Failed runs retain pending members and missing evidence.

`joined.jsonl` includes phase start/completion records and each runner's control
start/applied acknowledgment. `_harness` is reserved for coordinator trace records.
Run reports add exact session cost breakdowns for the cast and each group, by
model, profile and prompt purpose. The cast counts each actor once. Group totals
overlap when memberships overlap; do not add them together. Missing actor reports
are listed explicitly, and unreported provider charges remain unknown.

### Cast terminal monitor

```text
cargo run -p mush-bot -- --harness bots/harness/organized-party.json --output .mush-bot/party-001 --tui
```

The cast monitor shows the roster, current phase and selected bot's live terminal.
Tab switches actors; F9 switches state/attention, prompt assembly, session costs,
group, memory and debug details. PgUp/PgDn scroll the terminal; Ctrl+PgUp/PgDn scroll details.
Q requests graceful cancellation of the owned cast, saving reports; after a run
finishes, Q closes the monitor. Omit `--tui` for CI.

F2 requests a hold before the next phase; F5 dispatches one phase while held.
The current phase finishes normally, and bot/world activity and failure checks
remain live. Add `--harness-debug bots/debug/phase-gates.json` to start held and
set phase breakpoints. See [phase debugging](bot-cast-debug.md) for controls,
deadlines and the distinction between wall and active time.

The JSON manifest remains the source of actor controls. The single-bot TUI retains
its editors and REPL. Both UIs have layout tests; native terminal interaction has
not been automated. Ordinary display events are bounded and can skip updates
under load; debugger state uses a reliable latest-state channel, and files retain
the complete traces. Interactive casts are marked as debug runs and are
inconclusive in regression gates; use unattended runs for CI acceptance.

Pending architectural choices are recorded in [bot-decisions.md](bot-decisions.md).

### Reopening saved traces

```powershell
cargo run -p mush-bot -- --inspect-trace .mush-bot/party-001/run-0001/joined.jsonl --tui
```

The offline trace inspector browses recorded events by actor, kind and text,
with explicit request/command associations and the selected event's original
data. It also accepts individual bot transcripts. Omit `--tui` for a paginated
JSON metadata report; add `--trace-query <query.json>` for repeatable filters and
`--include-content` for full selected records. Inspection loads no credentials
and starts no bots or model requests. See [trace inspection](bot-traces.md).

## Planning and comparing experiments

Expand a cast's matrix and group phases before launching it:

```powershell
cargo run -p mush-bot -- --harness bots/harness/recall-experiment.json --plan-harness --report recall-plan.json
```

The plan validates actor configurations and JSON profiles, lists every run and
its selectors, and expands phase targets and scoped assertions. Serial targets
retain dispatch order. It shows which phases require acknowledgments, their
timeouts, and whether the harness would own a server process. It starts no
processes and resolves no credentials. Private briefings and character contents
are not copied into the plan. `--check-harness` retains its normalized-manifest
output; `--plan-harness` explains the expanded execution.

After running an experiment, analyze its saved summary and export CI results:

```powershell
cargo run -p mush-bot -- --analyze-harness party-001 --report party-analysis.json --junit party-tests.xml
```

Analysis groups repeated cast specifications and reports pass/fail/unknown
counts, recorded timing, command counts, session request counts and exact cost
breakdowns. Missing counters and receipts retain explicit coverage gaps. It reads
only `summary.json`; paths recorded inside the summary are not opened. Therefore
copied artifacts can be analyzed after the original game or workspace is gone.
JUnit contains one test case per run. Exit status is 0 when every recorded run
passed, and 1 for failed/unknown outcomes or invalid artifacts. Both exports omit
prompt bodies, private briefings and raw conversation. See
[offline analysis](bot-analysis.md) for grouping and missing-data semantics.

Compare two saved model requests:

```powershell
cargo run -p mush-bot -- --compare-captures before.json after.json --report prompt-diff.json
```

Comparison validates both captures before reporting exact message, role, order
and model-setting differences. JSON formatting outside strings is immaterial;
whitespace inside prompt content remains significant. Composed captures also
report added, removed, moved and changed component IDs, separately from changes
to provenance or the composition profile. Thus a change to excluded lore can
change profile metadata while leaving the actual model request identical.

By default the report includes hashes, byte counts and change descriptions.
`--include-content` adds exact changed messages and model-setting values. It
cannot recover text excluded from both original captures. Both legacy and
composed captures work; missing legacy provenance is marked unavailable. A valid
comparison exits 0 even when differences exist. These commands make no model or
SSH calls, do not load dotenv, and require new output files. Omit `--report` to
print the JSON result.

## Comparing memory retrieval policies

Context JSON now accepts a `recall` object. `lexical` preserves the established
default, `recency` favors newer evidence, and `relevance_recency` combines query
matches with a deterministic bonus for recent journal records. `max_records`,
`recent_episodes`, `recency_window` and `recency_weight` control the experiment.
Automatic model context and `/recall` use the same policy. The returned evidence
includes selection reasons, scores, budgets and missing source-record IDs. These
scores describe selection relevance, not confidence in a claim. Whole evidence
records are preserved, and context changes do not rewrite the journal.

See [local recall experiments](bot-recall.md) for the JSON format and limits.
The [recall matrix](../bots/harness/recall-experiment.json) compares three context
profiles with two repetitions each, keeping all context settings except policy
identical. Set its logins/config for your game and place
both characters in the same room first. It sends two controlled statements and
probes the listener's recall of a price and whether a purchase is confirmed.
The [probe rubric](../bots/probes/price-claim.json) checks the answer fields and
quoted episode citations. Each run has isolated bot memory. Recent speech and terminal history
remain available in this example; ablate those separately to isolate journal
retrieval. A passing probe means its explicit structural checks passed; quotation
validity does not establish truth or semantic entailment. Inspect answers and
captures before drawing broader conclusions about model quality.

## Structured answer probes

Use a JSON probe to check what a bot can answer from its supplied evidence:

```text
/probe bots/probes/price-claim.json
```

This pauses autonomous play and makes one model request. The model sees the
question, field descriptions and response protocol. Expected values and citation
constraints remain in the local evaluator. The response must contain typed
`answers` and exact episode `evidence`; the evaluator checks values, quoted text,
record IDs and any required source types against that request's captured memory.
It cannot retrieve omitted records or consult another character's journal.

`probe-result` contains the grade, rubric revision and request revision. The
single-bot TUI opens the result; the cast monitor includes the last grade in its
state view. Transcript events retain the separate probe definition and original
model reply. Costs use the `structured_probe` purpose. Probes consume the model
request budget, preserve temporary-character turn lifetimes, and apply no game
commands or belief updates. Pausing cancels a pending probe; it does not invent a
grade for the cancelled request.

In a harness step, choose `probe` instead of `control` and supply a JSON path
relative to the manifest. Actor, actors and group targets all work. Each targeted
actor must acknowledge and pass the probe; a failed grade fails the run. Serial
dispatch waits for each grade before proceeding to the next member. Rubrics are
copied into each run directory before any actor or owned server starts, so every
member in a run uses the same definition even if the source file is edited.
Raw `/probe` controls and starts are rejected in harness manifests: use the
typed step so acknowledgment alone cannot finish the phase before its grade.

Analysis and JUnit include probe failures. JSON analysis also groups results by
probe ID and definition revision, with actor and variant breakdowns. Missing,
cancelled and omitted results remain distinct from passing grades. Session
reports retain the last 128 grades and an omission count; each harness run retains
up to 4096 evaluations or 2 MB, with further omissions counted. Traces retain the
full event stream.

Validate or regrade saved input without SSH, dotenv or model calls:

```powershell
cargo run -p mush-bot -- --check-probe bots/probes/price-claim.json
cargo run -p mush-bot -- --grade-probe rubric.json capture.json answer.json --report grade.json
```

The answer file is the model's response JSON object. Offline grading uses the
same strict parser as online grading, including duplicate-key rejection. A grade
exits 0 on success or 1 on failure; malformed inputs also exit 1. New rubrics can
regrade old captures, with both revisions recorded. See [probe definitions and
evidence limits](bot-probes.md) for the schema and examples.

## Precise observation assertions

`contains` historically searches all of `event.data`, including a structured
command result's command text. Set `text_pointer: "/output"` with
`kind: "command-result"` to require the substring in the actual game response.
The selected value must be a string. Existing `pointer` plus `equals` checks can
be combined with it, or used alone for typed comparisons.

Typed null comparisons require an explicit data value; absent data does not
count as null. JSON pointer escapes must be `~0` or `~1`. Observation substring
matching retains continuous text per actor across unrelated local events, while
a fragment rejected by a typed predicate breaks that match's continuity.
Ordered assertions require successive matching events, including repeated
identical assertions.

## Regression gates for CI

For daily plans, route invalidation, guitar lessons and learned character growth,
see [activities and learning](bot-life.md). The bot and cast TUI expose the same
actor-local state as `/activity`, `/map`, `/learning` and `/development`.

After running baseline and candidate experiments, compare their saved summaries:

```powershell
cargo run -p mush-bot -- --check-regression bots/regression/recall-quality.json
cargo run -p mush-bot -- --compare-runs bots/regression/recall-quality.json baseline-run candidate-run --report regression.json --junit regression.xml
```

The [JSON policy](../bots/regression/recall-quality.json) gates run and probe pass
rates, citation validity, elapsed time, session request counts and exact provider
charges. Each check can set an absolute candidate bound, an allowed change from
baseline, and a minimum sample count. Variant selectors let a check compare one
matrix configuration per side.

Checks use only the saved summaries. Missing evidence, incompatible rubric
revisions and unknown charges are inconclusive, which fails CI rather than
inventing a successful comparison. Reports include exact fractions, observed
sample counts and content revisions. These thresholds measure the recorded
samples; they do not establish statistical significance. See the
[regression guide](bot-regression.md) for units, coverage rules and examples.
