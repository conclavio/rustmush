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

The simulator pilot holds to the same line. It reads only the text a
seated line-mode pilot is sent — the answers to its own `CONTACTS`,
`WEAPONSTATUS` and `STATUS`, the cockpit's notices and the centre's
announcements — and builds its picture of the match from nothing else. Its
format-contract test takes the game's own formatters as a **development**
dependency, so a changed line breaks a test and not a match. Nothing of the
machines' own controller is read, imported or mirrored: the pilot's tactics are
written from the player help (`help simulator`, `help cockpit`) alone.

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

A profile (and `[model]`) may also carry two objects that are passed through
verbatim into every request body: `reasoning` and `provider`. They are how an
OpenRouter request switches a reasoning model's thinking off and pins its
routing, for example in TOML

```toml
reasoning = { enabled = false }
provider = { order = ["Together"], allow_fallbacks = false }
```

or in a JSON profile `"reasoning": {"enabled": false}, "provider": {"order":
["Together"], "allow_fallbacks": false}`. Each must be a JSON object of at most
2048 bytes, nested at most four levels deep, or the profile is refused. They
are part of every request's recorded revision (the SHA-256 the cost ledger,
`/capture`, `--replay` and `--compare-captures` read), `--replay` records the
ones the captured request carried, and replaying under another profile
(`--replay` with `--model <profile>`) swaps them for that profile's. Unset, nothing about a
request or a record changes.

Catalog files are JSON only, at most 128 KB, with 1..64 profiles. Profile names
use letters, digits, `_` and `-`; `configured` is reserved for the TOML model
and `arena_commander` for the arena commander's `[arena.commander_model]`.
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
| `/arena red` or `/arena blue 3` | Fly the simulator for that side: claim, board and fight one (or `n`) matches; see [Arena pilot](#arena-pilot) |
| `/arena stop` | Stop the pilot (giving the slot up if no match has started) and pause; the tank is yours |
| `/arena` | Show the pilot's picture: phase, own tank, lock, weapons, contacts, spots, tally |
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

## Arena pilot

The bot can fly a tank in the simulator centre (`help simulator`,
`help cockpit`) over the same line-mode SSH session a person would use, with no
model at all. There are three ways to start it:

- **`--arena red`** (or `--arena blue`) on the command line flies that side at
  once. `--matches 3` (or `[arena] matches` in the config) plays three before
  it reports. `--headless --arena red` is a complete run: it needs no
  `[model]` and no goal.
- **`/arena red`** (or `/arena blue 3`) at the prompt of an interactive
  session starts the same pilot; bare `/arena` shows its picture, and
  `/arena stop` ends it and hands you the tank.
- **A goal's `arena` action**: a plain `--goal` whose model decides the goal is
  a fight hands the character to the pilot (below, "From a goal").

```powershell
$env:MUSH_BOT_PASSWORD = "your-game-password"
cargo run -p mush-bot -- --config mush-bot.local.toml --arena red --matches 3 --transcript arena.jsonl
```

**The example configs.** `bots/arena/` holds two configs to copy or run as
they are once `[ssh]` names your server and character:

- `bots/arena/pilot.toml` — the pilot alone, no model: `[agent] max_seconds =
  900` (each match's own limit), no bootstrap lines, `[memory] persistent =
  false` under the namespace `arena` (a pilot needs nothing across runs; its
  matches are in the report and the transcript), a persistent cost ledger, and
  every `[arena]` key written out at its default with what it does.
- `bots/arena/commander.toml` — the same plus a model commander
  (`[arena.commander_model]`, `deepseek/deepseek-v4.1-flash` through OpenRouter
  with reasoning off and pinned to Together; see "The commander").

```powershell
$env:MUSH_BOT_PASSWORD = "your-game-password"
cargo run -p mush-bot -- --config bots/arena/pilot.toml --arena red
cargo run -p mush-bot -- --config bots/arena/pilot.toml --arena red --matches 3 --headless --report arena.json
# The commander also needs OPENROUTER_API_KEY in the environment or the nearest .env.
cargo run -p mush-bot -- --config bots/arena/commander.toml --arena blue --transcript arena.jsonl
```

**From where.** The pilot starts from Limbo, the Welcome Hall or any room of the
simulator centre: it types `look`, then `ooc` and `simulator` as the room needs,
reads the board with `SIM STATUS`, and waits (reading it again every ten
seconds, up to `[arena] wait_start_s`) while another match is boarding or
running. Anywhere else it stops with `The simulator is not reachable from here:
the pilot starts from Limbo, the Welcome Hall or the simulator centre.` The
walk reads only its own room's heading and lines: a late look of the Welcome
Hall or of a room of the simulator centre that the walk is not going to (its
heading, then its description and exits, each within two seconds of the last)
is read as that room's and is no answer to the walk, so it never stops the
walk; a busy answer, a refusal or chat still answers it. The pilot knows no
other room by its heading, so a late look of any other room, Limbo's among
them, still reads as an answer and can stop the walk.

**What it types.** A closed list and nothing else: `look`, `ooc`, `simulator`,
`red`/`blue`, `SIM STATUS`, `SIM TEAM red|blue`, `SIM START`, `BOARD`,
`STARTUP` and `SIM LEAVE` in the centre, and `CONTACTS`, `STATUS`,
`WEAPONSTATUS`, `RANGE`, `BEARING B`, `VIEW BRIEF`, `HEADING`, `SPEED`, `LOCK`
and `FIRE` in the pod. `VIEW BRIEF` goes at the muster to read the ground for
the opening (below), and once more after each move in for room behind it: at
most four a match. It never types `SIM AI`, `SIM UNIT`, `SIM MODE`, `SIM RANGE`,
`SIM CONCEDE`, `SIM ABORT`, `ORDER` or `EXIT`. It types **at most four lines a
second** (`[arena] max_lines_per_s`, 1 to 4), every purpose together, well under
the server's flood limit and the cockpit's eight pending orders. A line-mode
pilot is sent no running picture, so it polls: `CONTACTS` once a second while it
holds anything (every five seconds otherwise, and a quarter-second after any
contact notice), `STATUS` every two seconds, `WEAPONSTATUS` half a second after
each shot until it knows that gun's cycle, then every few seconds — and every
eight seconds while every gun has its cycle and a chance to hit read against
the lock it holds, since it counts each gun's rounds down by its own shots —
the lock's `RANGE` after every
second `CONTACTS` (a lock reads finer than the scope), and `BEARING B` while it
searches. When the four slots are short, the order is: the first `STATUS` and
`WEAPONSTATUS` after the power comes up (nothing can be aimed before they are
read), then every gun that is due, main gun first, then the lock, `CONTACTS`,
the heading and the speed and the lock's `RANGE`, and then the polls that can
wait (`STATUS`, the regular `WEAPONSTATUS`, `BEARING B` and `VIEW BRIEF`).
Those keep a slot
free for any gun it would fire that is due inside the next second (not one that
reads `0%`, and none while it fires nothing), and stop waiting for the guns once
they are two seconds late; everything ahead of them in the order still goes
first.

**Lead or wingman.** The centre's own rule (`help simulator`) is that whoever
claims a side first leads it, and only a side's lead (or staff) may start the
match. The pilot follows it exactly. It keeps a claim window for each side,
filled in order by every `{name} takes {side} slot {i}.` line it hears and
emptied by every `{name} gives up {side} slot {i}.`, opened afresh by anything
that clears the board and by a `SIM STATUS` read of the side's row. It **leads
only while nobody stands ahead of it** in its side's window, and only then
types `SIM START`. Anything it cannot read only ever withholds `SIM START`, and
the server's `Only a side's lead or staff can start the match.` closes the
window. Under a human lead it is a **wingman**: it claims its slot, waits (up
to `[arena] wait_start_s`) for the lead to start, boards when the match boards,
powers up and fights its own tank. It never starts, aborts or concedes the
match, never adds or changes machines, and never gives orders to its side's
machines (`SIM START`, `SIM ABORT`, `SIM CONCEDE`, `SIM AI`, `SIM UNIT`,
`ORDER`). A result a human lead calls (an abort, a
concession) is reported as what it is: not a win.

**Live-box etiquette.** As lead it waits `[arena] lead_start_grace_ms` (three
seconds) after its claim before starting, and it starts that wait again
whenever anybody else claims or gives a slot up, so a person joining gets the
time to. When it stops before a match has started (a blocker, `/arena stop`,
the end of the run) and still holds a slot, it gives the slot up with one
`SIM LEAVE` (typed again only when the server answers it as busy, and only
within the three seconds it waits for the answer), so it never leaves a centre
locked behind it. It never types it while a match is boarding or running,
except the one it owes when it stops with a claim or a start of its own typed
and not yet answered: that one goes once the answer comes (whatever the
centre is doing by then; none when the answer is that it holds a slot on the
other side), or after at most `[arena] step_timeout_ms` without one.
Every centre room hears every match's announcements, so it moves only on the
lines of the match it claimed in, and it waits out another match that is
boarding or running before it claims.

**Taking the tank back.** When a dropped connection or a server restart hands
its tank to the machines (`… is flying on its mind; BOARD takes it back.`), or
a cockpit verb answers `You are not at the station of a deployed unit.`, it
types `BOARD` every three seconds until the tank is its own again
(`Back aboard {callsign} after {duration} away.`), then types `STARTUP` if the
tank went dark. The report counts each reclaim and the time away. A tank the
server says was left for good (`… it cannot be taken back.`) ends that match
for the pilot. A `STARTUP` that goes unanswered or is refused in a running
match is typed again, and so is one the server keeps answering as busy: after
forty busy answers in a running match it says ``The server kept answering
`STARTUP` as busy; typing it again in 15 s.`` and types it again after
`[arena] step_timeout_ms`. If the tank is destroyed or a wreck, the pilot waits for
the match to end. If the tank is no longer in the match, it records the match
as left, and waits for the end only when another match follows. It never gives
a running match up because the tank would not start, and records it with the
centre's own word. A run whose own process ended (its SSH connection closed)
does not come back by itself: start it again for the same side and it walks to the
centre, reads its own row on the board as `flown by its mind`, follows that
match, and boards back from its team room as above.

**How it fights.** Written from the player help alone — nothing of the machines'
own controller is read, imported or mirrored. It locks the nearest identified
hostile with a fresh plot (never an unidentified contact, never a friend) and
keeps that lock unless another is at least 100 m nearer; it fires each gun,
main gun first, when the lock's plot is fresh and the gun is ready **on the
gun's own clock**: the shot's acknowledgement starts it, one cycle and a slack
long. The cycle is the whole seconds a `cycling (N s)` row prints when it is
read inside the first second after the shot (the seconds the cockpit prints are
rounded up, and a row read later can print a second less). The slack covers
the arena's step after the acknowledgement and is never more than half a second
(one engaged pulse: "you and the arena are a pulse apart"); it starts at a
quarter-second, grows when the cockpit still says cycling to its own `FIRE`,
and shrinks a little with every shot that leaves. A `FIRE` refused as still
cycling is tried again a quarter-second into the last second it was told
(`1 s to go` a quarter-second on, `3 s to go` two seconds and a quarter on),
inside the second it was refused for. A gun is never fired again while its last
`FIRE` is unanswered.

**Its guns on the arena's lattice.** A line-mode gun fires no faster than one
cycle and one quarter-second step apart: the arena's own notices — every hit,
miss, contact notice and `Held:` — arrive on a 250 ms lattice within a match,
and a shot goes on the first lattice point after its acknowledgement. The
acknowledgement answers the `FIRE` within a few milliseconds, so it says
nothing of where that lattice lies; the notices do. So by default (`[arena]
lattice = true`) the pilot reads **the lattice's phase from the arena's own
notices** (the last 24 it read in a running match; at least 8 of them, and
three in four, in one 50 ms bin; never an acknowledgement, a refusal or a
reply), and **the round trip** from each acknowledgement less the moment its
`FIRE` went (the median of the last eight). Once a gun's cycle is learned, it
types that gun's next `FIRE` a **guard** of 275 ms past the instant the gun is
ready — the next lattice point after the acknowledgement and one cycle on —
**less the round trip**: a lattice point it reads is late by the wire's down
leg, and its `FIRE` takes the up leg to arrive, so the guard is measured where
the arena stands. **Why 275.** The bot's own transcripts on a local server
(a round trip of 1 to 2 ms), read against the lattice it learned, put the
arena's edge about a whole lattice step past that instant: of the `FIRE`s
sent 150 to 199 ms past it, 42 in 98 were accepted; 200 to 249 ms past it, 33
in 64; 250 to 299 ms past it, 55 in 61. An earlier guard of 175 ms was
refused about half the time. With the pilot's 50 ms tick, a guard of 275 ms
lands each `FIRE` 275 to 325 ms past that instant, past the edge. (Anyone
reading a transcript from a distant server must read it net of its round
trip.) **What to expect.** A `FIRE` that waits for the edge lands a lattice
step later, so the shot goes a cycle and two lattice steps after the last —
about 1.5 s apart for the machine gun, no faster than the clock above flies
live; no line-mode clock is expected to beat that. What the lattice clock can
buy at that gap is **fewer refused lines**, and fewer benches (the reading
counts them as `benched`, below). A `ready` row read before a lattice clock's
instant is the cockpit's frame, not the arena's acceptance, so it never pulls
that clock earlier. The pooled runs kept the lattice on by default: it
refused 6.9 per cent of its `FIRE` lines against 15.9 per cent with it off,
at the same shot gap ("From the muster, pooled", below). Two refusals as still
cycling of one gun's own `FIRE`s inside 30 seconds put that gun back on the
clock above for 30 seconds (`The machine_gun is off the lattice clock for 30
s: refused twice.`). It says `Firing on the arena's lattice: phase 5 ms, round
trip 40 ms.` once a match, the first time it uses it. **On a slow wire** it
changes nothing: above a round trip of 200 ms the lattice is off and the clock
above flies, and above about 250 ms no cycle is learned at all, so on a
distant enough server the pilot types exactly what it types with the lattice
off. On the offline lattice rig (an arena firing on its quarter-second pulse
that accepts a `FIRE` only 250 ms past the ready instant, the edge the
transcripts read) the machine gun's shots went a median of 1 500 ms apart with
the lattice on or off, and the lattice clock had no `FIRE` line refused (0 of
80 for the machine gun, 0 of 18 for the cannon) against 10 of 88 and 4 of 22
with it off. On the same rig accepting a `FIRE` only 350 ms past the ready
instant (past the guard) the guns were benched, the machine gun's shots still
went a median of 1 500 ms apart, and the lattice clock cost more refused
lines: 23 of 96 against 17 of 92. On the same rig with a steady wire of about
210 ms round trip, both cycles are learned and no gun ever goes on the
lattice.

**It fights nose-on.** `help cockpit` says the facet you show decides what a
hit costs: a hit from within 45 degrees of the bow strikes the front, a side is
twice the target of the bow, and the stock hull carries 400 in front against
250 a side and 150 behind. So by default the pilot **holds**: it stops at
`[arena] band_m` (150 to 450 m; 450 m by default: just outside the 420 m a
machine closes to, inside the 480 m a stopped tank is seen at, and the widest
band it ever flies) with its bow on the hostile, within 30 degrees of it, and
changes the range only along the line of fire — ahead to close, in reverse to
open. It moves only once the range is more than 75 m off the band, and stops
again once back within 50 m of it, so a machine at the scope's 50 m rounding
(420 m read as `400 m`) never sets it reversing and back. A commander's
`close` drives in bow on and holds at 200 m, and `hunt` drives at a fresh
hostile and holds at the
band; every withdrawal — its own fall back and a commander's kept `withdraw` —
backs away in reverse with the bow on the target. A hit on a side or the rear
turns the bow toward it, stopped, for four seconds: toward the nearest hostile
whose own contact row puts it in that face's arc, or, with only the held target
to blame, nowhere new (the hold already faces it), or, with nothing to blame,
to the middle of that face (the hit line says nothing of who fired); a newer
hit moves the turn only when its shooter is well apart from the last, and one
with nothing to blame does not move it while the turn is still under way and
unread by `STATUS`. A held stop, the back-off and every withdrawal outrank the
turn, and it says `Turning into the shot` only when it makes the turn; a hit
read four seconds late, or taken while the machine flew the tank, turns
nothing, and with a track gone a hit on the dead side turns nothing either
(the live way round would take longer than the turn lasts). The
orbit — circling the target at the band, toward its rear when its plots show
it moving, else reversing every twenty seconds — is now only a stance a
commander or `[arena] engagement = "orbit"` asks for (`hold` by default; no
other word is accepted); under it a hit turns nothing.

**With a track gone** ("Lose one track and the tank is slowed and will not turn
that way") every turn the pilot makes of its own accord goes the live way, an
orbit circles the way that turns the live way, and a heading that would turn it
toward the dead side by 45 degrees or less is not ordered at all, so the bow
may stand that far off the target, still inside the front's arc, rather than
the tank going round a whole circle for a small correction. A larger turn that
way is ordered once, and then no heading at all until `STATUS` reads it or
twelve seconds (a whole circle at the speed a one-track tank turns) have
passed. It does not reverse to turn.

A gun the tank no longer carries is struck from the fire schedule for the rest
of the match at the first `That unit carries no weapon called …`, and an empty
one at `FIRE … refused. Empty: …` (until a `WEAPONSTATUS` row counts rounds
into it again). When a lost section's line says `its weapons are gone`
(anywhere in it: the shipped turret's line goes on `; its core is gone`, or
`; its core is degraded` on the heavy), every gun is held and a `WEAPONSTATUS` goes at once; nothing is fired until it
answers. Each is said in an `arena` event: `No cannon aboard any more: struck
from the fire schedule.`, `The turret's weapons are gone: no FIRE until
WEAPONSTATUS says what is left.`, `The machine_gun is empty: struck from the
fire schedule.`

It falls back only on its own hull: its internals at half, or a hull plate at a
quarter or gone (a turret or track plate does not count). It backs away from
the target in reverse, its bow on it, for ten seconds, or until it has opened
to its widened band if that is sooner. The widened band is 150 m wider than its
band, never narrower than it and never wider than 450 m, so at the default
band the fall back is a short step back inside the sight. After that it flies
its doctrine again at the widened band, a `close` flown as a hold there.

When it loses a hostile it hunts it rather than waiting: it drives to the
contact's last plot, then along its estimated course (the plots' course when
they showed one, else the way the range was opening) while that plot is under
90 seconds old, then sweeps once in a box of four 300 m legs (at most a
minute). It never drives past the proving ground's edge: the course leg ends
where its next 100 m would cross it, or once the tank is that far past the
estimate. A fresh hostile ends the hunt at once. A hostile the cockpit said was
destroyed is never hunted, targeted or shot at, and its row, still listed until
it expires, neither holds off nor ends a hunt for another.
Only after the hunt, with no identified hostile held for five seconds (an
unidentified contact does not count), does it drive for The Crossroads (`B`),
800 m past it, and back.

**It stops before it is found.** `help cockpit` says a tank on the move is seen
from further away than one standing still — on grass a stopped tank is seen
out to 480 m against the stock sight's 600 m, a moving one out to all of it,
and no ground hides a mover better than a stopped tank — so, while no
identified hostile is held, the pilot stops wherever it is (these stops read
no ground; only the opening, below, reads its own brief). A **return not yet told apart**, once a contact row has given its
bearing, is faced, stopped, while it resolves: for at most six seconds from
when it was first seen (the help's three seconds to tell one apart, twice
over), with a rest as long between two such stops and a handful of them (four)
a match at most, so a stream of returns cannot hold the tank still; a return
known only from its `New contact` notice does not stop it, and one lost and
found again (the help: it "reads `X2` again") is a new return, first seen
then. A **hunted machine
the tank has not hurt and that is not going away**, expected back inside
700 m, is waited for, stopped and facing where it is estimated to be, for
twenty seconds, once a trail: the help's machine "drives at where it last saw
you", and a tank that arrives moving is seen first. A machine it **has** hurt
(the cockpit said `BA's turret is destroyed.` or any section of it) is chased
at once, because the help's hurt machine breaks off for home and on the way
fights only what is close; so is one whose plots showed it going away or
across when the hunt began (judged there, once: a tank that has reached or
passed the plot has the bearing back to it turned round). The hunt's legs stand still through a wait, and **any hit ends a wait**
for good: the tank has been found. A fresh hostile ends it, and a hit on a side
or the rear still turns the bow first.

**It opens from the muster.** `help cockpit` says that on grass a stopped tank
is seen out to 480 m against the stock sight's 600 m, a moving one out to all
of it, and that a road hides nothing either way; `help simulator` says a
machine that sees you closes to about 420 m, stops, turns onto you and shoots,
and that one that has seen nobody for a minute and a half crosses to the other
side's entry zone to look. So the pilot no longer drives for The Crossroads
first. It opens this way **only in a skirmish it saw start**: not in a hold
match or on a range (the board's `mode:` line says which), not in a posed duel
where its first `CONTACTS` after `Power online.` already holds something other
than a teammate (a contact not yet told apart more than 100 m off included) or
the cockpit announced one, and not after joining a match already
running (it must have read this match's own `Power online.` and its first
`STATUS` within five seconds of it). Its **own entry** is where that first
`STATUS` put it. The **other side's entry** is worked out, not read: both sides
fly identical stock tanks from opposite ends of the ground, so the pilot
mirrors its own muster across the ground's middle, on the ground's size from
its brief's header (`… of 256 by 256; … 10 m a cell.`) or, without one, the
help's 2.56 km square; no contact's bearing is used. It then types `VIEW
BRIEF` and reads the ground round it.

**It holds with room behind it.** When it later backs away from a machine
(below), the longest back-off it orders runs from about 420 m of the machine,
where the help's machines stop, out to 650 m of where it last saw it; the
scope reads 420 m as `400 m`, and a back-off keeps 30 m of ground behind it,
so with a margin it wants about **290 m of ground it can cross** behind the
place it holds. Ground it can cross is road, grass, bare ground, rubble, light
woods and plain water (the ford); deep water, heavy woods and any other ground
stop a tank. It reads the room from its own brief: the distance behind it,
straight away from the other side's entry, to the edge of the ground, cut
short where the brief's line behind it first meets ground that stops a tank.
It does this only when it would ever back away (the re-approach or the follow
is on). From a muster with less room, it first **moves in** toward the other
side's entry, at full ahead, along a line its brief shows passable (stopping
20 m short of anything that stops a tank), by the room it lacks; a shortfall
of 30 m or less is room enough, and a move in shorter than 40 m is not
driven. Then it **stops**, types `VIEW BRIEF` again and reads the ground
where it stands, at most three times a match. With no passable line in, it
says so and opens where it stands. From red's first pod that is about 235 m
in along the road, and the brief there shows room enough.

Then, standing on a road (red's first pod musters on the east-west road), it
**steps off the road** onto ground that
hides a stopped tank — grass, bare ground, rubble or light woods; never heavy
woods, which stop a tank — on the nearest such ground in any direction more
than 45° from the other side's entry, never toward the machines: past the
road's true edge by 20 m (the brief rounds its reaches to 10 m and counts them
from the middle of the tank's own cell, so the pilot allows half a cell on a
straight line and a cell and a half on a diagonal), at 15 kph, 80 m at most.
From red's first pod, after its move in, that is about 35 m north onto grass. Already standing
on such ground (six of the eight ground lines around it begin on it), or with
no brief (none in five seconds, or a dark window), it stays where it is. Then
it **holds**, stopped, its bow toward the other side's entry, until a hostile
is in sight, a hit is taken, or six minutes have passed (then it searches as
before). A contact not yet told apart does not end the wait: it is faced while
it resolves, as ever. A hostile first seen inside the stock sight while it
waits is **fired on from the halt**: the tank stays stopped with its bow on it,
and every gun fires on its own clock, because the machine comes on to 420 m and
stops, and a tank closing on it would be seen sooner. The halt ends when the
hostile stops coming on (six seconds in which its `CONTACTS` row reads it no
nearer and its plots show no course), when it reaches the band, when the tank
is hit, or after a minute, and then the pilot fights as before. A hostile row from beyond the sight is a teammate's
report: it ends the wait with no halt, and the pilot fights or hunts as before.
A wingman under a human lead opens the same way beside a teammate at the
start: at its first `CONTACTS`, a contact the cockpit names as its own side,
or one not yet told apart within 100 m (the two sides muster at opposite ends
of the ground), is a teammate and ends nothing.

**It opens the range on a machine that out-hits it** (only with `[arena]
reapproach = true`; off by default, see "From the muster, pooled" below).
`help simulator` says
"Open the range past its sight and it comes after you again; let the ground
hide you and it drives at where it last saw you". When the machine it fights
has **stopped** (its plots show no course), is inside the stock sight, and has
hit the tank at least eight times in the last twenty seconds and four more
times than the tank has hit it, the pilot **backs away bow on**, steering on
the machine while it can see it and on its last plot after: between 480 and
600 m of a stopped machine the pilot cannot see it, but it can see the pilot
moving. At 650 m from that plot (past the machine's sight of a mover) it stops,
facing the plot, and **waits** up to 45 seconds for the machine to come on; a
hostile that comes on inside the stock sight is fired on from the halt (a
teammate's report from beyond it ends nothing), and a hit ends the wait. A
tank reverses only along its bow line, so where the ground behind runs short
the pilot **slants its bow** off the machine by the smallest angle, 10, 20 or
30 degrees, that leaves 30 m of ground behind it, keeping the machine inside
its front arc; when no such slant does, when the cockpit says it is held
while it reverses, or when ground it remembers as held stops the reverse, it
**stops there and waits** as at 650 m (`No room behind: waiting 410 m from
BA's last plot.`) rather than backing into the edge. The backing off gives up
after a minute: `help cockpit` gives the stock tank 27 kph astern, and grass
caps it to about 20 kph, so 250 m of backing off takes about 46 seconds. It
does this at most twice a match, with half a minute's rest between, never
while falling back or under an
`orbit` or `withdraw` doctrine, and no hunt starts while it does it; after the
wait the pilot hunts the machine only if its last plot is still young enough,
else it searches.

**It follows a broken machine home.** `help simulator` says that a machine
"Hurt badly enough … breaks off for its own entry zone", that on the way home
it fights only what is close (inside about 200 m) while it has armour left,
and that once home it shoots back at whatever comes for it and, after twenty
quiet seconds with nothing hostile in sight, comes back in, once. So a
hostile the cockpit said lost a section (`BA's turret is destroyed.`), or one
the pilot has hit eight times this match, whose plots show it **driving away**
is **followed**. Driving away is a course within 60 degrees of the bearing to
it; a course faster than 20 m/s (the stock tank makes 15) is noise and shows
nothing. A machine whose plots show no course at all — at 500 to 600 m the
scope's 50 m rows seldom carry one for a tank making 10 m/s — is driving away
when its row's range from where the pilot stands now has opened by 100 m
inside ten seconds. It is followed bow on,
held 350 m behind it — outside the 200 m it turns to fight at, inside the
480 m a stopped tank is seen at — and fired at from behind. It is **never
followed into its entry zone**. The help names the entry zone but not its
shape, so the pilot keeps out of a band across the whole approach: a line
through the other side's entry, square to the line from its own muster to
it. Once the machine is within 400 m of that line, or the pilot within 750 m,
the pilot **backs out of its sight**, in reverse with its bow on the machine
(on its last plot once it is out of sight), to 650 m from that plot (past the
600 m a moving tank is seen at; at most 65 seconds, the same 20 kph over
about 300 m), then **waits** there, stopped and facing it, up to 90 seconds
for its one return. A machine seen farther off moves that plot farther; one
coming back never moves it nearer, so the pilot still stops where it meant
to and lets the machine come on. Like the re-approach's, this back-off slants
or stops where the ground behind runs out (`No room behind for the return:
waiting 520 m from BA's last plot.`). A machine already that close to its
entry line when it breaks off is not followed at all: the pilot says only
that it will not follow it into its entry zone, and backs out of its sight.
A hostile that comes on inside the sight is
fired on from the halt (the machine waited for only once its row reads nearer
than that plot: still in sight and driving home, it is let go); a hit ends the
wait; at its end the pilot drops the
machine's trail and searches, because a hunt would drive at its last plot,
near its entry line, and on into the entry zone. A followed machine lost goes
the same way, to the backing off; one whose plots stop showing it moving for
five seconds has stopped, and the pilot fights it as before; one the pilot
turns from to fight another hostile is no longer followed. A followed
machine's row is flown up to six seconds old under any stance the commander
sets, the same limit at which it counts as lost. A machine that
breaks off while the pilot is opening the range on it or firing at it from
the halt is followed at once, and that rule ends. No hunt starts, and no re-approach
begins, while it follows or waits. It follows only from a muster it saw: a pilot that
joined a running match has no entry line, and fights a broken machine as
before.

**Its keys.** `[arena] opening = "wait"` (the default) opens as above;
`"search"` drives for The Crossroads after five quiet seconds, as the pilot did
before (any other word is refused). `[arena] reapproach = true` opens the
range as above; `false` (**the default**, chosen on the pooled runs below:
the re-approach reached its wait in only 11 of 31 back-offs) never does.
`[arena] follow_broken = true`
(the default) follows a broken machine home as above; `false` holds and hunts
it as before. `[arena] lattice = true` (the default) fires on the arena's
lattice as above; `false` keeps the clock that adds a learned slack to each
acknowledgement. All four are written out in `bots/arena/pilot.toml`,
`bots/arena/commander.toml` and `mush-bot.toml.example`.

Held by the ground, it turns 45° for three seconds and orders its speed again.
Still held at 0 kph, it turns further from the heading it stopped on (90°, then
135°, then 180°, each judged four seconds after the last), and at last backs
off for three seconds; a tank it stopped or reverses itself is never counted as
held up. It **remembers held ground**: a stop on a slope too steep, on ground
that will not take the tank or at the arena's edge — never `Held: something is
in the way.`, which is another unit that moves — marks the point 20 m ahead of
where it stopped, and while it drives ahead it steers 45 degrees aside (up to
three times) round any mark its next 120 m would pass within 30 m of, unless the
mark is within 50 m of the tank (its own stop is the held turn's business). A
mark is a point, not a whole bank: a detour can meet the same bank further
along, and that is a new stop and a new mark. Stopped while reversing, it does
not drive ahead to turn: it stops with its bow kept, marks the ground behind,
and does not reverse into it again.

It says what it does in `arena` events: `Hunting BA: its last plot, 380 m
away.`, `Falling back to 450 m: the hull is at half.`, `Held at 0 kph: turning
to 180°.`, `Turning into the shot: hit on the hull's left facet; facing BB.`,
`The right_track is gone: turning counter-clockwise only.`, `Held ground marked
at 1420 m east, 1049 m north, heading 000°.`, `Steering round held ground: 136°
for 091°.`, `Stopping for X1: facing 087° while it is told apart.`, `Stopping
before BA finds us: its estimate is 590 m off at 012°.`, `The wait for BA is
over; hunting on.`, `Opening: moving in 235 m toward the other side's entry:
65 m of ground behind us, 290 m wanted.`, `Opening: stepping off the road to
the north (35 m).`, `Opening: holding near our entry, facing 091° toward the
other side's entry.`,
`Opening over: BA is in sight after 95 s.`, `Firing from the halt at BA, 590 m
off.`, `The halt is over: BA has stopped.`, `Opening the range on BA: 10 hits
taken to 4 scored in 20 s; backing off bow on.`, `Out of BA's sight at 650 m
from its last plot; waiting for it to come on.`, `No room behind: waiting
410 m from BA's last plot.`, `No approach from BA in 45 s; moving on.`,
`Following BA home: it is hurt and driving away; holding 350 m behind it.`,
`Not following BA into its entry zone; backing off out of its sight.`,
`Waiting for BA to come back, 650 m from its last plot.`, `No room behind for
the return: waiting 520 m from BA's last plot.`, `No return
from BA in 90 s; searching.`, `Firing on the arena's lattice: phase 5 ms,
round trip 40 ms.` and their kin, and the `arena-picture` record carries its
hunt, its fall back and its stall, the turn into a hit (its bearing and the
seconds left), the live turn with a track gone (`cw` or `ccw`), how many marks
it holds, what its hold is doing (`stopped`, `ahead` or `astern`), the wait it
is in (whom it is for and the seconds left, or `null`), the rule of its own in
force (`rule`: `opening`, `halt`, `reapproach`, `approach_wait`, `follow`,
`return_back`, `return_wait` or `null`), the hits taken and scored in the
last twenty seconds (`exchange`), where the opening is (`opening`: `fresh`,
`brief`, `inward`, `step`, `hold` or `done`), the start it kept (`start_m`), the
lattice's phase and round trip as it reads them (`lattice`: `phase`,
`rtt_ms`, each a figure or `null`), each gun's cycle and slack and whether
its last shot set the lattice clock (`lattice`), the guns struck and whether
every gun is held. Its first `SPEED`
order is `999`, which makes the tank say its own top speeds.

**The report and the exit code.** The JSON report gains an `arena` block: the
side, the matches asked for, one entry a match (the match number, the callsign,
whether it led, the centre's own result word, shots that left, orders refused,
hits, misses, kills, hits taken and their split by the facet struck
(`hits_taken_by_facet`, keyed like `hull left` or `right_track right`), sections
lost, whether the tank was destroyed,
reclaims and time away, the match's length, and the commander's calls, applied,
late, invalid, failed and overruled answers and why it stopped) and the wins, losses,
draws, timeouts and others. The run is `done`, and a headless run exits 0, **only when
every match it flew ended on its own side's word**; anything else — a loss, a
draw, a timeout, an abort, a blocker — is `blocked` and exits 2. The outcome
reads `Won match 3 for red as RA: 7 shot(s), 4 hit(s), 1 kill(s); took 2 hit(s).`
or `Match 3 ended (timeout); the bot flew RA for red: …`, and for several
matches `Won 2 of 3 matches for red (lost 1, drew 0, timed out 0, other 0).`
A run that blocks after some matches says the blocker, then how many of the
asked matches it flew and their tally: `The red side is full. Stopped after 1
of 2 match(es) for red: won 1, lost 0, drew 0, timed out 0, other 0.`

**What it remembers.** The transcript keeps every byte the game sent, and each
line the pilot typed as an `arena-send` record with the lane's own clock
(`lane_ms`, the clock the four-a-second limit runs on); `arena-state` records its
phase, lead and seat on every change, `arena-picture` its picture every five
seconds while a match runs, and `arena` events say what it did in sentences. Its
memory, attention and the recent window see only the lines worth remembering:
the poll replies, the acknowledgements of its own orders and the echo of its own
typing are left out. With `[memory] persistent = false` (as in
`bots/arena/`) nothing of that outlives the run but the transcript and the
report; with a persistent memory the journal lines are kept like any other
observation, under the config's namespace.

**On a slow wire.** Flown against a distant server, a reply reaches the pilot in
pieces, and it is still read whole however it arrives. Its rows count as they
come; what a reply says by leaving something out (a contact gone, a gun gone,
the lock gone, a section's state) is taken at the first game line after it,
never from a pause in the stream. A line of chat in the middle of a contact or
weapon list is read as itself, and so is any line that begins with `[`. A line
that arrives straight after a full-width channel line, where the game wraps it,
is read as the rest of that line, never as a game sentence, unless it answers
something the pilot just typed (a reply or an order's answer, the lock line
heading a weapon list, or anything at all while a line typed in the centre
awaits its answer). A line whose end arrives late is whole for up to two
seconds. A `FIRE` answered `please retry in a moment` is typed again on the
gun's own clock. A gun's cycle is learned only from a row read within a second
of typing its `FIRE`: on a round trip over about a quarter of a second the
cycle stays unlearned and the gun fires from its rows, rather than a second
early.

**At the prompt while it flies.** `/pause` stops the pilot typing and nothing
else: it keeps reading and its picture settles, so the picture stays current,
and a tank left sitting in a match is your choice. While paused its commander
asks nothing, so a paused run spends nothing. `/send`, a bare line, and any
command that pauses the run pause it the same way before they go. `/resume`,
or anything that sets the run running again, says `Resumed after {s} s; the
lobby's waits go on where they stood.`: the lobby's waits — a lobby step's
answer, the start and boarding bounds, a rematch due while you were paused —
are moved on by the pause's length, so each goes on where it stood (a boarding
announced while paused keeps the window the centre gave it), while the
pilot's own clocks (its opening, its back-offs' and returns' waits) ran on
through the pause. A pause never leaves the run blocked. **A pause through a
match's boarding and its start loses that match**: the game drops a slot
whose pilot never boarded, so the `BOARD` typed after the resume is answered
`You hold no slot in the simulator.`, and the pilot says `Match 3 started
without us; waiting for it to end.`, records the match as left and claims the
next once it ends; after the last match asked for (`Match 3 started without
us.`) the run ends there. After a resume the pilot reads its tank again and
sends its heading and speed again first. Your own command's output never holds
the pilot up once it runs again. The run's time limit (`[agent] max_seconds`)
keeps counting while paused. `/arena stop` ends it and hands you the tank. A
claim or a start it has typed and not yet heard back about is waited for (at
most `[arena] step_timeout_ms`), and then the slot is given up with one
`SIM LEAVE`. It will not fly under a mission contract, a script, a managed
floor or a campaign session.

**What it never does.** It never types a line outside its closed list, and
never more than four in a second. It never starts a match it does not lead,
and never aborts, concedes, reconfigures (`SIM AI`, `SIM UNIT`, `SIM MODE`,
`SIM RANGE`) or leaves a match (`EXIT`), nor gives `ORDER`s to machines. It
never reads the server's state, logs or the machines' controller: everything
it knows is what its own session was sent, so it holds nothing about a unit its
own tank has not detected. It never locks a friend or an unidentified contact.
While it flies, no goal turn runs unless you take one with `/step` (which asks
the goal model); on its own the only model it asks is its commander, and a
commander's answer changes only the doctrine, never a line.

**How well it flies against the machine.** Measured by an ignored lane,
`cargo test -p mush-bot --test arena_win_rate -- --ignored --nocapture`: ten
1 v 1 matches in one run (`--arena red --matches 10`, no model) against one
blue machine, both tanks posed 500 m apart on the road before every match,
**seven minutes (420 s) a match at most**. The cap is that long so a finished
fight is not cut off: on a three-minute cap half the matches timed out, every
one of them one-sided. A timeout is its own column, never a loss. A run takes
about seventy-five minutes when every match runs long (the runs of 2026-09-30
took 24 to 32 minutes, most matches ending well inside the cap), and the whole
flight has one deadline of 100 minutes. It prints `WIN RATE: {w} of 10 for red
against one machine at 1 v 1 (…)` and one line a match, and fails only when
fewer than ten matches finish: the target is seven in ten, and the lane
reports the figure rather than failing on it. CI runs it only by hand (the
`bot-win-rate` job, `workflow_dispatch`); read the figure off the job's log.

**The engagement it flies.** `MUSH_BOT_WIN_RATE_ENGAGEMENT` picks the pilot's
own engagement, written into the bot's config as `[arena] engagement`: unset or
`hold` flies the hold (the default), `orbit` the orbit, and anything else stops
the lane before it starts (`MUSH_BOT_WIN_RATE_ENGAGEMENT is hold or orbit, not
…`). Nothing in the code changes between the two.

```sh
MUSH_BOT_WIN_RATE_ENGAGEMENT=orbit cargo test -p mush-bot --test arena_win_rate -- --ignored --nocapture
```

**The knobs it flies.** `MUSH_BOT_WIN_RATE_OFF` names the pilot's own rules to
fly off for a comparison run, on either win-rate lane: a comma list of
`opening`, `reapproach`, `follow`, `lattice` or `all` (all four), written into
the bot's config as `[arena] opening = "search"`, `reapproach = false`,
`follow_broken = false` and `lattice = false`; unset, each flies its shipped
default. A knob shipped off is flown off whether it is named or not (naming
it is accepted and changes nothing); no variable turns a knob shipped off
back on. Anything else stops the lane
before it starts (`MUSH_BOT_WIN_RATE_OFF names opening, reapproach, follow,
lattice or all, not …`). Both lanes print `KNOBS: opening {wait|search},
reapproach {on|off}, follow {on|off}, lattice {on|off}` straight after the
`ENGAGEMENT` line: the value **flown**, the shipped default less what the
variable names. Every lane's reading ends with `PILOT RULES: opening
{a}, off the road {b}, halt {c}, reapproach {d}, approach wait {e}, follow
{f}, return wait {g}, lattice {h}, inward {i}, room holds {k}, return room
holds {r}, gave up {u}, benched {n}`: how many times the pilot held near its
entry, stepped off the road, fired from the halt, opened the range, waited for
the machine to come on, followed a broken machine, waited for its return,
(once a match at most) put its guns on the lattice, moved in for room behind
its opening, stopped a re-approach's back-off and a return's back-off where
the ground behind ran out, gave a re-approach up at its time, and took a gun
off the lattice clock after two refusals (`… is off the lattice clock …`),
counted from its own `arena` sentences.

```sh
MUSH_BOT_WIN_RATE_OFF=lattice cargo test -p mush-bot --test arena_muster_rate -- --ignored --nocapture
```

Neither lane can fly a knob shipped off on (as the re-approach now ships):
set it in a config (`[arena] reapproach = true`) and run the bot by hand.

**What it prints.** First `ENGAGEMENT: {hold|orbit}; flown hold 164 of 164
picture records` (the stances the transcript's picture records actually flew,
most first), then the `WIN RATE` line, straight after it `WIN RATE INTERVAL:
{w} of {n}, 95 per cent Wilson {lo} to {hi} per cent` (`n` the matches
counted; `WIN RATE INTERVAL: no matches` with none), and one line a match,
then a
reading of the transcript and the report: `FIRE GAP {gun}: p10 … / p50 … / p90
… ms over {n} gaps` (the gaps between one gun's `FIRE` lines, each under 20 s),
`SHOT GAP {gun}: …` (the same between the shots that left, `Firing {gun}.`, so a
refused line tried again a quarter-second later does not count as a short
gap), `STILL CYCLING {gun}: {k} of {n} FIRE lines`, and `HITS TAKEN BY FACET:
hull front 205, turret front 150, …` (summed over the matches; `none`, or `no
report` when the bot left none). The live commander lane prints the same lines
after its match lines.

**The interval.** Ten matches are a small sample: a 95 per cent Wilson
interval on 7 of 10 runs from 40 to 89 per cent, and on 5 of 10 from 24 to
76, so every win-rate figure is printed with its interval and no difference
is called while two intervals overlap.

**It always reports.** Set `MUSH_BOT_WIN_RATE_KEEP` to a directory and the
run's transcript and report are kept there (`win-rate.jsonl`, `win-rate.json`)
before anything is printed, however the run ended, with its `KNOBS` line and a
newline in `win-rate.knobs` beside them. When the bot left no
report, the figure is counted from the lobby's own `Match {n} is over (…)`
lines, and `NO REPORT: the figure is from the lobby's match-end lines.`
follows it. A run that stopped early says so after the match lines:
`EARLY END: {why} after {n} match(es).`, where the reason is `the bot ended`,
`no match end inside 600 s`, `the 100-minute deadline` or `the watching
session closed` (the lane's own staff session dropped). A refused pose
answers at once, and the match it was for (the next one by the centre's own
match number) flew from the 2 430 m muster: its line ends ` (unposed)` and
`UNPOSED: {u} of {n} match(es) flew from muster.` counts them.

**What it measured.** The first run, on 2026-09-29 on the three-minute cap
(before the pilot hunted, fell back inside the sight, turned out of held
ground and fired on each gun's own clock), read `WIN RATE: 4 of 10 for red
against one machine at 1 v 1 (lost 1, drew 0, timed out 5, other 0)`: every
timeout ended with the machine, its turret gone, out of sight, and nearly half
the pilot's `FIRE` lines were refused as still cycling. It was re-measured on
the seven-minute cap on 2026-09-30, three runs, each changing one hunting
constant from the one before:

- with the constants the hunt first shipped with (a 45 s trail, a 60 s sweep):
  `WIN RATE: 2 of 10 for red against one machine at 1 v 1 (lost 5, drew 0,
  timed out 3, other 0)`;
- with the trail at 90 s (`HUNT_TRAIL_MS`), the sweep still 60 s:
  `WIN RATE: 4 of 10 for red against one machine at 1 v 1 (lost 5, drew 0,
  timed out 1, other 0)`, **selected from 3 runs**, and the constants the
  pilot ships with;
- with the trail at 90 s and the sweep at 30 s (`HUNT_SWEEP_MS`):
  `WIN RATE: 4 of 10 for red against one machine at 1 v 1 (lost 4, drew 0,
  timed out 2, other 0)`.

In all three, 0 to 10 per cent of the `FIRE` lines in a match were refused as
still cycling, and every held stop was cleared inside its match (the longest
held stretch 18 s). The losses are the exchange of fire: the machine hit about
once a second from first contact, and in every lost match the pilot took more
hits than it scored. Every timeout was a tank with a track shot away: two
could not move at all, and the rest hunted at a crawl, the plot about 600 m
off taking 88 to 211 s to reach, so the course leg never ran and the sweep
barely moved; twice the search after the sweep found the machine again and
the pilot won. All three flew the orbit.

**Nose-on, measured.** On 2026-09-30 again, with the hold as the default, the
guns on their own clocks and the stops before being found, three runs on the
hold and one forced onto the orbit (every other constant as the pilot ships;
flown before a return lost and found again was stopped for afresh, and before
a trail's going away was judged once, when its hunt began):

| run | fire slack: start, rise | result | shot gap p50 (cannon, machine gun) | still cycling (cannon, machine gun) | facet hits taken on a front facet |
|---|---|---|---|---|---|
| hold 1 | 250, 150 ms | `WIN RATE: 5 of 10 … (lost 5, drew 0, timed out 0, other 0)` | 6 550, 1 498 ms | 18 of 107, 62 of 412 | 355 of 355 |
| hold 2 | 250, 200 ms | `WIN RATE: 5 of 10 … (lost 5, drew 0, timed out 0, other 0)` | 6 544, 1 499 ms | 17 of 108, 75 of 434 | 404 of 404 |
| hold 3 | 300, 150 ms (as shipped) | `WIN RATE: 4 of 10 … (lost 6, drew 0, timed out 0, other 0)` | 6 497, 1 497 ms | 12 of 104, 58 of 423 | 401 of 401 |
| orbit | 300, 150 ms (as shipped) | `WIN RATE: 4 of 10 … (lost 5, drew 0, timed out 1, other 0)` | 6 612, 1 502 ms | 15 of 125, 75 of 497 | 47 of 429 |

The pilot ships with the third (**selected from 3 runs**, on the guns'
refusals: a starting slack of 250 ms saw 20 to 28 per cent of each gun's first
five `FIRE` lines in a match refused, 300 ms 14 and 18; a win more or less in
ten is noise). On the hold **every hit that named a facet struck a front
one** (the rest, 17, 28 and 28 a run, struck a turret already destroyed, which
names no facet), no track was shot away and nothing held it, against one facet hit in nine on the
front under the orbit, which lost a track in six matches of ten. The win rate
did not move with it: the machine still hit about twice every three seconds
from first contact, and the tank, fighting at the band, still took about
as many hits as it scored. A return not yet told apart stopped the tank once a
match, at its first sighting (a machine found again after a hunt was not
stopped for then; it is now); every machine it hunted was one whose turret it had destroyed, so the
wait before being found never ran. A line-mode machine gun fires no faster than
about once every 1.25 s (it is ready one cycle after the arena's next
quarter-second step following its acknowledgement), and these runs measured a
median of 1.5 s between its shots, a seventh of its `FIRE` lines still refused
as cycling: the slack may not grow past half a second, so that share stays
above one in ten.

**The live commander lane.** A second ignored lane,
`crates/mush-bot/tests/arena_live_commander.rs`, flies the same rig, poses and
seven-minute cap for five matches with `bots/arena/commander.toml`'s commander
and its real model (or, with `MUSH_BOT_LIVE_FROM_MUSTER=1`, from the muster:
see below). It prints first which it flew, `FLOWN FROM: posed 500 m apart
(420 s cap)` or `FLOWN FROM: the muster (600 s cap)`, then `LIVE COMMANDER:
{w} of 5 for red against one machine at 1 v 1 (…)`, one line a match with the
commander's calls and what became of them (applied, late, overruled, invalid,
errors, and why it stopped if it did), the reading's lines, and a `COST:` line
from the cost ledger. **It spends real money**:
a live smoke on 2026-09-29 made 370 calls over five matches for 0.0706
US dollars, about 1.4 US cents a match, about 410 ms a call. So CI never runs
it and never sets its variables; it is run by hand:

```sh
export OPENROUTER_API_KEY=...
MUSH_BOT_LIVE_COMMANDER=1 cargo test -p mush-bot --test arena_live_commander -- --ignored --nocapture
```

The key must be **exported into the shell**: the test reads its own process
environment, not `.env`, and hands the key to the bot it starts and nothing
else. Opted in without the key, the lane fails rather than skips; without
`MUSH_BOT_LIVE_COMMANDER=1` it prints a skip line and passes. An offline test
in the same file, run with the bot's other tests, proves that the config it
writes loads, validates and resolves its commander from the table.

**From the muster.** The posed lane is a duel from its first second: both
tanks start inside each other's sight. The match a person plays starts at the
muster, about 2.4 km apart, and a third ignored lane flies exactly that: ten
matches, no pose, from the muster, **ten minutes (600 s) a match at most**,
with no model.

```sh
cargo test -p mush-bot --test arena_muster_rate -- --ignored --nocapture
```

Its `WIN RATE FROM THE MUSTER: {w} of 10 for red against one machine at 1 v 1
(…)` is **the figure of record**; the posed lane stays as the duel benchmark.
CI runs it only by hand too (the `bot-muster-rate` job). A run takes up to
about two hours, under one deadline of 140 minutes; `MUSH_BOT_WIN_RATE_KEEP`
keeps `win-rate-muster.jsonl`, `win-rate-muster.json` and
`win-rate-muster.knobs`, `MUSH_BOT_WIN_RATE_ENGAGEMENT` picks the engagement
and `MUSH_BOT_WIN_RATE_OFF` the knobs flown off, as for the posed lane, and it
prints its `WIN RATE INTERVAL` straight after its figure.
Each match line ends with the match's first contact, first hit taken and
first hit scored, in whole seconds after its `Power online.` (or `none`).
The report carries the same three times for each match (`first_contact_ms`,
`first_hit_taken_ms`, `first_hit_scored_ms`), in milliseconds after that
match's `Power online.`; they carry no designator and name nothing of the
other side. Every lane now ends its reading with three more lines:

- `SEEN FIRST: contact before hit {a}, hit before contact {b}, neither {c} of
  {n} matches; lead p10 … / p50 … / p90 … s over {k}; shot first {s}, shot at
  first {t}`. The bot cannot know when the machine saw it. The machine closes
  to about 420 m before it shoots, and both sights reach past that, so the bot
  almost always has a contact before its first hit taken, and that count alone
  says little. What it can measure is **the lead**: the seconds from its own
  first contact to its first hit taken. A long lead is the bot seeing first; a
  short one is the machine finding it first. `shot first` counts the matches
  where the bot scored before it was hit, `shot at first` the others that saw
  a hit taken.
- `OPENING: first contact p10 … / p50 … / p90 … s after power online over {k}
  matches; first hit taken p50 … s over {j}`: how long the opening took
  (`OPENING: no contact in {n} matches` when nothing was ever seen).
- `FRONT SHARE: {f} of {h} facet hits taken on a front facet`: how much of the
  punishment the front took.

**The pool.** Kept runs are read together by a fourth ignored lane in the
same target, `arena_win_rate_pooled`. `MUSH_BOT_WIN_RATE_POOL` names the kept
directories as a path list (separated by `;` on Windows, `:` elsewhere), and
`MUSH_BOT_WIN_RATE_POOL_NAME` the files' stem (`win-rate-muster` by default;
`win-rate` pools posed runs). It flies nothing. On Windows give the
directories as Windows paths (`C:/…`): Git Bash does not translate a `/c/…`
path inside a list.

```sh
MUSH_BOT_WIN_RATE_POOL="runs/d1;runs/d2;runs/d3" cargo test -p mush-bot --test arena_muster_rate arena_win_rate_pooled -- --ignored --nocapture
```

It prints `POOLED: {k} runs`; for each run `RUN {i}: {w} of {n}, 95 per cent
Wilson {lo} to {hi} per cent` (or `RUN {i}: no report`) and `RUN {i}: KNOBS:
…` (or `RUN {i}: KNOBS: unknown` when the run kept no knobs file); then
`WIN RATE FROM THE MUSTER, POOLED: {w} of {n} …` over the summed results (`WIN
RATE, POOLED` for posed runs), its `WIN RATE INTERVAL`, and one reading of all
the runs from `FIRE GAP` on (no gap is counted across two runs). It stops,
before printing anything, when a directory holds neither the transcript nor
the report, or when two runs' known `KNOBS` lines differ: runs flown with
different knobs are never pooled. Unset, it prints `POOL:
MUSH_BOT_WIN_RATE_POOL is unset; nothing to pool.` and passes, so the
`bot-muster-rate` job runs it harmlessly.

The live commander lane flies from the muster too with
`MUSH_BOT_LIVE_FROM_MUSTER=1` beside `MUSH_BOT_LIVE_COMMANDER=1` (`0` or unset
flies it posed; anything else stops it before it starts), its first line then
`FLOWN FROM: the muster (600 s cap)`:

```sh
MUSH_BOT_LIVE_COMMANDER=1 MUSH_BOT_LIVE_FROM_MUSTER=1 cargo test -p mush-bot --test arena_live_commander -- --ignored --nocapture
```

The commander is asked about every two seconds whatever the tank can see, so a
muster match of up to ten minutes, much of it with nothing in sight, costs up
to about 300 calls (under the 400 a match `commander_max_calls` allows), and
five matches up to about 1 500.

Ten matches are a small sample: three runs of one unchanged bot on the posed
lane read 5, 5 and 4 of 10, so a difference of one or two wins between runs is
not a finding.

**From the muster, measured.** The first run, on 2026-10-01, with the pilot as
it shipped then (the hold, nothing changed for the muster), took 28 minutes
and read `WIN RATE FROM THE MUSTER: 4 of 10 for red against one machine at 1 v
1 (lost 6, drew 0, timed out 0, other 0)`. Every match opened the same way:
the pilot drove east along the road at full speed, and the first contact came
83 to 86 s after power online, about 550 to 600 m off dead ahead. The first hit
taken followed 7 to 12 s later, 7 to 9 s in nine matches of ten (`SEEN FIRST:
contact before hit 10, hit before contact 0, neither 0 of 10 matches; lead p10
7 / p50 8 / p90 9 s over 10; shot first 10, shot at first 0`; the p90 by its
index rule leaves out the longest of ten). The pilot scored first in every
match, and every hit that named a facet struck a front one (`FRONT SHARE: 408
of 408 facet hits taken on a front facet`); the matches lasted 114 to 229 s,
none near the cap.

**From the muster, measured with its own rules.** On 2026-10-01, with the
opening, the halt, the re-approach, the follow of a broken machine and the
lattice clock all on (the shipped defaults): one run as shipped, then two
runs each moving one constant and keeping it only if the line it was meant to
move moved that way and the wins did not fall; neither move was kept, so the
pilot ships with the values of the first run. Then one run with the
re-approach off, because the third run lost more matches than the run before
the opening (below) and re-approached in it; and one posed duel. These runs
were flown before a broken machine could end a re-approach or a halt in force
(see the follow below).

| run | result | `SEEN FIRST` | `OPENING` | `FRONT SHARE` | machine gun `SHOT GAP` |
|---|---|---|---|---|---|
| before the opening (the first measured run) | `WIN RATE FROM THE MUSTER: 4 of 10 … (lost 6, drew 0, timed out 0, other 0)` | `contact before hit 10, hit before contact 0, neither 0 of 10 matches; lead p10 7 / p50 8 / p90 9 s over 10; shot first 10, shot at first 0` | `first contact p10 83 / p50 85 / p90 86 s after power online over 10 matches; first hit taken p50 94 s over 10` | `408 of 408` | `p10 1444 / p50 1500 / p90 1887 ms over 305 gaps` |
| as shipped | `WIN RATE FROM THE MUSTER: 7 of 10 … (lost 3, drew 0, timed out 0, other 0)` | `… lead p10 12 / p50 13 / p90 14 s over 10; shot first 10, shot at first 0` | `first contact p10 156 / p50 158 / p90 158 s …; first hit taken p50 171 s over 10` | `382 of 382` | `p10 1352 / p50 1497 / p90 1807 ms over 406 gaps` |
| lattice guard 200 ms (not kept) | `WIN RATE FROM THE MUSTER: 5 of 10 … (lost 5, drew 0, timed out 0, other 0)` | `… lead p10 12 / p50 13 / p90 14 s over 10; shot first 10, shot at first 0` | `first contact p10 156 / p50 158 / p90 159 s …; first hit taken p50 172 s over 10` | `362 of 362` | `p10 1390 / p50 1499 / p90 1793 ms over 343 gaps` |
| re-approach margin 8 (not kept) | `WIN RATE FROM THE MUSTER: 2 of 10 … (lost 8, drew 0, timed out 0, other 0)` | `… lead p10 12 / p50 12 / p90 13 s over 10; shot first 10, shot at first 0` | `first contact p10 157 / p50 158 / p90 159 s …; first hit taken p50 171 s over 10` | `391 of 391` | `p10 1407 / p50 1499 / p90 1895 ms over 300 gaps` |
| as shipped, re-approach off | `WIN RATE FROM THE MUSTER: 8 of 10 … (lost 2, drew 0, timed out 0, other 0)` | `… lead p10 12 / p50 13 / p90 14 s over 10; shot first 10, shot at first 0` | `first contact p10 156 / p50 158 / p90 158 s …; first hit taken p50 171 s over 10` | `384 of 384` | `p10 1428 / p50 1499 / p90 1785 ms over 433 gaps` |
| posed duel, as shipped | `WIN RATE: 4 of 10 … (lost 6, drew 0, timed out 0, other 0)` | `… lead p10 12 / p50 13 / p90 14 s over 10; shot first 10, shot at first 0` | `first contact p10 0 / p50 0 / p90 0 s …; first hit taken p50 13 s over 10` | `353 of 353` | `p10 1393 / p50 1500 / p90 1961 ms over 386 gaps` |

What the lines show. **The opening** worked every match: the pilot stepped
off the road, held, and saw the machine first — the machine came to it about
158 s after power online, the pilot scored first in every match, and its first
hit taken came 12 to 14 s after its first contact against 7 to 9 before. Every
facet hit still struck a front one. **The re-approach** began six to nine
times a run on an exchange of about 17 hits taken to 13 scored and never
reached its wait: every one that ended said `Could not open the range on BA in
45 s; holding.`, and the run with it off won 8 of 10. The machine did not
re-close on it: the tank backed at most some 65 m from where it held, to the
arena's west edge, and stopped there (`Held ground marked at 0 m east, …`)
with the machine's row still 400 to 450 m off; held that near its own entry,
it has no room behind it to back out to 650 m, and the range opened only when
the machine itself left. (The second tuning move, a wider margin before a
re-approach begins, was made on the reading that the machine re-closed.)
Those runs are why the opening now holds with room behind it, why a back-off
slants or stops where the ground behind runs out, and why the backing off is
given a minute; the figures above were read before those changes.
**The follow** ran none to three times a run, and the return's wait as often
except in the first run, whose one return was still backing off when the
match ended. In six matches of the first two runs a re-approach was in force
when the machine broke off for home, and once lost it was hunted at its last
plot into its entry zone: in one the machine was seen driving away and the
follow could not begin while the re-approach was in force (it now ends the
re-approach); in the other five, and once in the run with the re-approach
off, the machine published no track as it went, so it was never judged
broken. **The lattice clock**
was used every match and benched 22 to 36 times a run: read against the
lattice the pilot learned, a `FIRE` arriving 175 to 250 ms past the instant the
gun was ready was refused about as often as it was accepted, and one arriving
250 ms or more past it almost always accepted — the arena's edge on that day
lay past the guard's 200 ms ceiling, so the machine gun's shots stayed a
median of 1.5 s apart and about a fifth of its `FIRE` lines were refused. The
guard has since been widened to 275 ms, past that edge ("Its guns on the
arena's lattice", above). Ten
matches are a small sample (one unchanged bot read 5, 5 and 4 of 10 on the
posed lane), so these figures are read beside the mechanism lines, never
alone.

**From the muster, pooled.** On 2026-10-02, with the opening's move in for
room behind it, the back-offs that slant or stop at the edge and the lattice
guard at 275 ms, the defaults were chosen on pooled runs. **The rule:** a
behaviour stays on by default only when its own reading shows it doing its
job and its pooled figure is not below its figure off; no difference is
called while the intervals overlap, and at thirty matches against ten only a
gap of about 35 points can be called. So the default rests mainly on each
behaviour's own reading. Three runs of one build at the defaults of the day
(every knob on) were pooled to thirty matches; the re-approach and the
lattice clock were each flown once off; then one run at the defaults that
resulted, and one posed duel.

| run | `KNOBS` | result | 95 per cent Wilson interval | machine gun `STILL CYCLING` | machine gun `SHOT GAP` p50 | `PILOT RULES` (reapproach, approach wait, room holds, gave up, benched) |
|---|---|---|---|---|---|---|
| default 1 | all on | `2 of 10` (lost 8) | 6 to 51 per cent | 25 of 325 | 1 500 ms | 12, 4, 2, 0, 2 |
| default 2 | all on | `4 of 10` (lost 6) | 17 to 69 per cent | 23 of 354 | 1 500 ms | 11, 5, 2, 0, 2 |
| default 3 | all on | `6 of 10` (lost 4) | 31 to 83 per cent | 20 of 391 | 1 500 ms | 8, 2, 0, 0, 0 |
| **the three pooled** | all on | **`12 of 30` (lost 18)** | **25 to 58 per cent** | 68 of 1 070 | 1 500 ms | 31, 11, 4, 0, 4 |
| re-approach off | `reapproach off` | `3 of 10` (lost 7) | 11 to 60 per cent | 26 of 363 | 1 500 ms | 0, 0, 0, 0, 3 |
| lattice off | `lattice off` | `3 of 10` (lost 7) | 11 to 60 per cent | 78 of 474 | 1 500 ms | 11, 5, 0, 0, 0 |
| as now shipped (re-approach off) | `reapproach off` | `3 of 10` (lost 7) | 11 to 60 per cent | 20 of 337 | 1 500 ms | 0, 0, 0, 0, 2 |
| posed duel, as now shipped | `reapproach off` | `8 of 10` (lost 2) | 49 to 94 per cent | 24 of 510 | 1 500 ms | 0, 0, 0, 0, 2 |

No run's interval lies wholly above or below another's, so no win-rate
difference is called. What the readings showed:

- **The re-approach** began 31 times in the thirty pooled matches and reached
  its wait at 650 m 11 times: under the half its rule asks for. Four more
  stopped where the ground behind ran out (`No room behind: …`) and none gave
  up at its minute, so the raised bound covers the drive; of the other 16, 15
  were still in force when the match ended (6 won, 9 lost) and 1 ended when
  the machine drove away and was followed. It is **off by default** (`[arena] reapproach = false`) and
  stays a key.
- **The lattice clock** had 92 of its 1 342 `FIRE` lines (both guns) refused
  as still cycling, 6.9 per cent, against 94 of 590, 15.9 per cent, with it
  off, at the same machine-gun shot gap (a median of 1 500 ms both ways). It
  was benched 4 times in thirty matches, against 22 to 36 a run with the
  175 ms guard. It stays **on by default**.
- **The move in** ran in every match (`inward` 10 a run). The pilot still saw
  the machine first in every match (`SEEN FIRST: contact before hit 30 … of 30
  matches; lead p10 11 / p50 12 / p90 13 s`), but the machine now came into
  sight about 137 s after power online (`OPENING: first contact p10 136 / p50
  137 / p90 138 s … over 30 matches; first hit taken p50 150 s`), about 21 s
  sooner than from the hold near the entry (158 s), because the hold is now
  235 m nearer the other side's entry. Every facet hit still
  struck a front one (`FRONT SHARE: 1253 of 1253`).

The defaults that result: `opening = "wait"`, `reapproach = false`,
`follow_broken = true`, `lattice = true`.

**From a goal.** A plain `--goal` can hand the character to the pilot: the goal
model may answer with the action `arena` and a `side` (`red` or `blue`, in any
case), for example
`{"action":"arena","side":"red","summary":"Fight for red in the simulator."}`.
The runner then flies exactly as `--arena` does, for `--matches` (or
`[arena] matches`) matches, from Limbo, the Welcome Hall or the simulator
centre, and the lane's result ends the goal the same way: `done` only when
every match ended on the bot's own side's word. An `arena` action without a
side, or a `side` on any other action, is an invalid decision; an `arena`
action with a `command` is refused like any other non-command action with one.
Under a mission contract it blocks the goal with the same sentence `--arena`
refuses with, and in a managed-floor or campaign session with that refusal. A
conversational turn never starts the pilot: an `arena` answer there is a
declined turn, like `done`. Nor does an `arena` answer replace a pilot that is
still flying or still leaving its slot (reached by stepping the goal model with
`/step` mid-match): the bot says `the arena pilot is already flying; /arena stop
first`, as `/arena` does, and the pilot flies on.

### The commander

An optional model commander sets the pilot's **doctrine** — where it goes and
what it hunts — and nothing else. The deterministic pilot keeps the lock, the
fire cadence and the steering; the commander never types a line, never fires
and never aims. Without a commander the pilot flies its own default doctrine
(hold the nearest hostile at `[arena] band_m`, bow on, or orbit it under
`[arena] engagement = "orbit"`).

**Its model.** `[arena.commander_model]`, a model table with the same keys as
`[model]`, when it is given; else the session's own selected model, but **only
with `commander_from_session = true`**; else there is no commander. The session
model is off by default because a `--goal` user's `[model]` may be slow or a
reasoning model, and would be paid for up to `commander_max_calls` late answers
a match that the pilot throws away. As the lane starts, the bot says which:
`Commander: {name} from [arena.commander_model].`,
`Commander: {name}, the session's model.` or
`No commander; the pilot flies on its defaults.` It shares the session's model
tape when one is configured.

```toml
[arena]
commander = true            # a commander flies when a model resolves
commander_ms = 1500         # asks again this long after each answer or error (1000..10000)
commander_max_calls = 400   # the most calls in one match (1..10000)
commander_from_session = false # the session's model commands only when true

[arena.commander_model]
endpoint = "https://openrouter.ai/api/v1/chat/completions"
name = "deepseek/deepseek-v4.1-flash"
api_key_env = "OPENROUTER_API_KEY"
timeout_secs = 5            # the commander's timeout is at most 10 s whatever this says
max_tokens = 200
reasoning = { enabled = false }
provider = { order = ["Together"], allow_fallbacks = false }
```

**What it is sent and what it answers.** One system message (the commander's
own instructions) and one user message: the pilot's picture as compact JSON of
at most 2 KB — seconds since the power came on (or, after taking a powered tank
back, since the panel first said the power was up), the pilot's own rule in
force (`rule`: `opening`, `halt`, `reapproach`, `approach_wait`, `follow`,
`return_back`, `return_wait` or `null`; below), the own tank (callsign,
**its own side**, power, speed, heading, position, each section's condition
and plate, each weapon's readiness, rounds and chance), the six nearest
contacts (designator, side, bearing, `rel` — the contact row's own bearing from
the tank's bow, `null` for a contact known only from its notice — range, plot
age, and course and speed when the plots show it moving), the lock, the
doctrine in force (without its
note: the model's own last reason, sent back to it, only made it repeat
itself) and the six latest events. It holds nothing the pilot's own screen did
not say. The answer is one JSON object,
`{"target": "BA" | null, "stance": "orbit" | "close" | "hold" | "hunt" |
"withdraw", "band_m": 150..450, "sense": "cw" | "ccw" | "auto", "hold_s":
5..30, "note": "…"}`; every field but `hold_s` and `note` is required and
nothing else is allowed (a stance of `stand_off`, the hold's old name, still
reads as `hold`). The band is clamped into 150..450 (450 m is the
widest band the pilot flies: a band past it loses a machine that stops to
shoot), a withdraw's `hold_s` into 5..30 (10 when it gives none; any other
stance's is dropped), the note cut to 160 characters, and a target that is not
1 to 4 of `A-Z0-9` reads as none (the pilot ignores a target it does not hold
as hostile anyway). An answer that does not read keeps the doctrine and is
recorded as `commander-invalid`; one that does is recorded as read
(`arena-doctrine`) and flown from the next tick, by the pilot's rule below.

**Hold.** `hold` stops at the band with the bow on the target and changes the
range only along the line of fire, ahead to close and in reverse to open (a
model's `stand_off` reads as it); `close` drives in bow on to 200 m and holds
there; `orbit` circles at the band and shows the target a side the whole time.

**What the instructions tell it.** Besides the stances and the rules below,
the system message gives the model the help's own figures: a round from within
45 degrees of the bow strikes the front (400 on the hull, 350 on the turret), a
side 250 or 220 and is twice the target, three side hits in ten land on that
side's track, and a round from behind strikes the thinnest plate and can reach
the engine — so **turn into the shot**; circling did not stop the machines'
cannon and machine guns hitting (in a live run on the orbit it still
chose the orbit a quarter of the time, and of 494 hits the tank took on a named
facet only 32 struck a front one); a stopped tank on grass is seen at 480 m
against a mover's 600 m, so the tank that stops before it is found sees first,
and a hunt first waits, stopped, for a machine it has not hurt; and each
contact's `rel` is its bearing from the bow. It calls neither the hold nor the
orbit the pilot's own default: `[arena] engagement` decides that. It also
tells the model that the pilot keeps **four rules of its own** that no stance
moves while they are in force, and that `rule` in the picture names the one in
force: in a skirmish flown from the muster, the wait near its own entry, off
the road and bow toward the other side's entry, until a hostile is in sight; a
hostile first seen in that wait fired on from the halt until it stops, comes
to `band_m` or finds the tank; the backing away from a stopped machine that
out-hits it, past the machine's sight of where it last saw it, then a wait for
it to come on; and a hurt machine driving away followed outside 200 m and
inside the sight and fired at from behind, never into its entry zone, then a
wait out of its sight for its one return.

**Hunt.** `hunt` drives straight at the target until its band and then
holds it there, bow on, but only while its plot is fresh (three seconds); with no
hostile in sight it hunts the last one lost at once, as the pilot does on its
own (its last plot, its course, one sweep), and with nothing to hunt it
searches. A `null` target and the lost contact's own designator both mean
that one.

**The pilot's rule on a withdraw.** The model adjudicates, the pilot enforces.
In a live smoke on 2026-09-29 the commander answered `withdraw` for
238 of 367 doctrines, most of them on a lost contact and some on a hostile it
had read as a friend, and only 33 on a hull at half or less. So a `withdraw`
answered while the tank's hull is sound (its internals above half) is flown as
`hunt` when no hostile with a fresh plot is held and, when one is, as the
pilot's own engagement (`hold` by default, `orbit` under `[arena] engagement =
"orbit"`), keeping its target, band and sense; the transcript records it as
`commander-overruled` (`{"from": "withdraw", "to": "hunt", "why": "the hull is
sound"}`, or `"to": "hold"`) and the report counts it. A withdraw on a failing hull is kept, as
**one bounded hold**: `hold_s` seconds counted from the first withdraw of the
episode, however often the commander answers withdraw again (a later answer
may move its target or band, never its end). Then the pilot's own doctrine
flies again (`doctrine-lapsed`, `"the hold ran out"`), and for the next 30
seconds any withdraw is flown as the sound hull flies it (`"why": "the hold is
spent"`), so the commander keeps the tank away at most half of any minute. An
episode ends at its time whatever stands then: if the commander has answered
another stance meanwhile, nothing lapses, but the rest follows all the same. A
kept withdraw outranks the pilot's own wider band after it has fallen back, so
it is what backs the tank away, bow on, for its hold.

**When it asks.** Never more than one call at a time. The first at
`Power online.`; the next `commander_ms` after the last answer or error; and at
once — or as soon as the call in flight returns — when a contact is identified
to the other side, when one of the tank's own sections is destroyed, and when
the lock breaks and is not held again for three seconds.

**Late answers.** An answer that arrives more than **twice the contact refresh
period in force when it was asked** is dropped (`commander-stale`): two seconds
while the pilot holds a contact (it reads `CONTACTS` every second), ten seconds
while it searches (every five). A line-mode pilot's picture is refreshed by its
own polls, never by the arena's pulse, so that is what "two refreshes of the
picture" means here. A call still unanswered at that bound is **given up**
there: counted late, recorded `commander-stale` with `"abandoned": true`, and
aborted, so a provider that hangs never holds the one call for longer; an
answer that still comes is ignored. And the commander's own HTTP timeout is at
most ten seconds (the longest bound), whatever `timeout_secs` its model says: a
model table's default is 90 seconds, and the session's model, under
`commander_from_session`, keeps its own for the goal loop.

**When it stops** (the pilot flies on, on the last doctrine it applied, except
a withdraw, which gives way to the pilot's own doctrine with `doctrine-lapsed`,
`"the commander stopped"`): at the match's end; when the tank is destroyed;
after `commander_max_calls` in the match; after five errors in a row; after
**five late answers in a row** (`answers arrive too late to use`; a timely
answer, readable or not, resets the count, and a call given up counts); when
the pilot stops; and when the session budget blocks a call. Each writes one
`commander-off` record with its reason. The next match starts it
again, except after the budget, which holds for the rest of the session: every
later match of that pilot, and every pilot started later in the run (by
`/arena` or a goal's `arena` action), flies without a commander and names the
budget as the reason. A failed call records its cause as `commander-error`
(the transport's error, the provider's status, or a request that could not be
built).

**What it costs.** Every call is recorded like any other model request
(`model-input`, `model-request`, `model-reply`, `usage`, `cost-receipt`), filed
in the cost ledger under the purpose `arena_commander`, and under the profile
`arena_commander` when it came from `[arena.commander_model]` (a profile file
may not take that name). It is gated by the session budget **in the commander
model's own currency** — a budget that names only EUR refuses a USD commander,
and a commander-only run with no `[model]` is not refused for want of a
currency — and a block is recorded once as `budget-blocked`. The budget is
checked before an ask is taken, so a refused ask is never counted as a call.
Each call's records carry its own ledger request id as their `request_id`,
never the goal decision's. Commander calls
are not goal turns: they never count against `[agent] max_steps`. Each match in
the report's `arena` block counts `commander_calls` (the requests the cost
ledger holds), `commander_applied`,
`commander_stale`, `commander_invalid`, `commander_errors` and
`commander_overruled` (the withdraws the pilot's rule flew as a hunt or as the
pilot's own engagement), and names `commander_off` when it stopped before the
match's end.

**Why reasoning must be off.** Measured before this was built, on a picture of
about 1.2 KB and a five-field answer: `deepseek/deepseek-v4.1-flash` through
OpenRouter with reasoning off answered in a median 1 342 ms on default routing
(313 to 1 597 ms, eight of eight valid) and 333 ms pinned to Together (295 to
479 ms, ten of ten valid). With `reasoning = { effort = "low" }` the median was
10.6 s, the slowest 56.8 s, and five answers in eight spent the whole 1 500-token
budget thinking and returned nothing. A commander answer is useful only inside
the picture's own refresh, so send `reasoning = { enabled = false }` (and pin a
fast provider when the latency matters). `bots/arena/commander.toml` does both;
pinned to Together at `commander_ms = 1500`, a live smoke on
2026-09-29 made 370 calls over five matches for 0.0706 US dollars (about 1.4
US cents a match, about 410 ms a call). On default routing the median answer
(1.3 s, the slowest measured 1.6 s) is inside the two-second bound with little
room to spare, and five late answers in a row switch the commander off for the
match.

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
