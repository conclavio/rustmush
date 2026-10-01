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

The simulator pilot (cut AJ) holds to the same line. It reads only the text a
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
the pilot starts from Limbo, the Welcome Hall or the simulator centre.`

**What it types.** A closed list and nothing else: `look`, `ooc`, `simulator`,
`red`/`blue`, `SIM STATUS`, `SIM TEAM red|blue`, `SIM START`, `BOARD`,
`STARTUP` and `SIM LEAVE` in the centre, and `CONTACTS`, `STATUS`,
`WEAPONSTATUS`, `RANGE`, `BEARING B`, `HEADING`, `SPEED`, `LOCK` and `FIRE` in
the pod. It never types `SIM AI`, `SIM UNIT`, `SIM MODE`, `SIM RANGE`,
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
wait (`STATUS`, the regular `WEAPONSTATUS` and `BEARING B`). Those keep a slot
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
`ORDER`; 16c Q-AJ2 and Q-AJ3). A result a human lead calls (an abort, a
concession) is reported as what it is: not a win.

**Live-box etiquette.** As lead it waits `[arena] lead_start_grace_ms` (three
seconds) after its claim before starting, and it starts that wait again
whenever anybody else claims or gives a slot up, so a person joining gets the
time to. When it stops before a match has started (a blocker, `/arena stop`,
the end of the run) and still holds a slot, it gives the slot up with one
`SIM LEAVE` (typed again only when the server answers it as busy, and only
within the three seconds it waits for the answer), so it never leaves a centre
locked behind it; it never types it while a match is boarding or running.
Every centre room hears every match's announcements, so it moves only on the
lines of the match it claimed in, and it waits out another match that is
boarding or running before it claims.

**Taking the tank back.** When a dropped connection or a server restart hands
its tank to the machines (`… is flying on doctrine; BOARD takes it back.`), or
a cockpit verb answers `You are not at the station of a deployed unit.`, it
types `BOARD` every three seconds until the tank is its own again
(`Back aboard {callsign} after {duration} away.`), then types `STARTUP` if the
tank went dark. The report counts each reclaim and the time away. A tank the
server says was left for good (`… it cannot be taken back.`) ends that match
for the pilot. A run whose own process ended (its SSH connection closed) does
not come back by itself: start it again for the same side and it walks to the
centre, reads its own row on the board as `flown by doctrine`, follows that
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
into it again). When a lost section's line ends `its weapons are gone`, every
gun is held and a `WEAPONSTATUS` goes at once; nothing is fired until it
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
for 091°.` and their kin, and the `arena-picture` record carries its hunt, its
fall back and its stall, the turn into a hit (its bearing and the seconds
left), the live turn with a track gone (`cw` or `ccw`), how many marks it holds
and what its hold is doing (`stopped`, `ahead` or `astern`), each gun's cycle
and slack, the guns struck and whether every gun is held. Its first `SPEED`
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

`/pause` stops the pilot typing and `/resume` starts it again; `/arena stop`
ends it and hands you the tank. It will not fly under a mission contract, a
script, a managed floor or a campaign session.

**What it never does.** It never types a line outside its closed list, and
never more than four in a second. It never starts a match it does not lead,
and never aborts, concedes, reconfigures (`SIM AI`, `SIM UNIT`, `SIM MODE`,
`SIM RANGE`) or leaves a match (`EXIT`), nor gives `ORDER`s to machines. It
never reads the server's state, logs or the machines' controller: everything
it knows is what its own session was sent, so it holds nothing about a unit its
own tank has not detected. It never locks a friend or an unidentified contact.
While it flies, no goal turn runs: the only model it may ask is its
commander, and a commander's answer changes only the doctrine, never a line.

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
fewer than ten matches finish: the owner's target is seven in ten, and the lane
reports the figure rather than failing on it. CI runs it only by hand (the `bot-win-rate` job,
`workflow_dispatch`); read the figure off the job's log.

**It always reports.** Set `MUSH_BOT_WIN_RATE_KEEP` to a directory and the
run's transcript and report are kept there (`win-rate.jsonl`, `win-rate.json`)
before anything is printed, however the run ended. When the bot left no
report, the figure is counted from the lobby's own `Match {n} is over (…)`
lines, and `NO REPORT: the figure is from the lobby's match-end lines.`
follows it. A run that stopped early says so after the match lines:
`EARLY END: {why} after {n} match(es).`, where the reason is `the bot ended`,
`no match end inside 600 s` or `the 100-minute deadline`. A refused pose
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
the pilot won.

**The live commander lane.** A second ignored lane,
`crates/mush-bot/tests/arena_live_commander.rs`, flies the same rig, poses and
seven-minute cap for five matches with `bots/arena/commander.toml`'s commander
and its real model. It prints `LIVE COMMANDER: {w} of 5 for red against one
machine at 1 v 1 (…)`, one line a match with the commander's calls and what
became of them (applied, late, overruled, invalid, errors, and why it stopped
if it did), and a `COST:` line from the cost ledger. **It spends real money**:
the lead's live smoke of 2026-09-29 made 370 calls over five matches for 0.0706
US dollars, about 1.4 US cents a match, about 410 ms a call. So CI never runs
it and never sets its variables; the lead runs it by hand:

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
back, since the panel first said the power was up), the own tank (callsign,
**its own side**, power, speed, heading, position, each section's condition
and plate, each weapon's readiness, rounds and chance), the six nearest
contacts (designator, side, bearing, range, plot age, and course and speed
when the plots show it moving), the lock, the doctrine in force (without its
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

**Hunt.** `hunt` drives straight at the target until its band and then
holds it there, bow on, but only while its plot is fresh (three seconds); with no
hostile in sight it hunts the last one lost at once, as the pilot does on its
own (its last plot, its course, one sweep), and with nothing to hunt it
searches. A `null` target and the lost contact's own designator both mean
that one.

**The pilot's rule on a withdraw.** The model adjudicates, the pilot enforces.
In the lead's live smoke of 2026-09-29 the commander answered `withdraw` for
238 of 367 doctrines, most of them on a lost contact and some on a hostile it
had read as a friend, and only 33 on a hull at half or less. So a `withdraw`
answered while the tank's hull is sound (its internals above half) is flown as
`hunt` when no hostile with a fresh plot is held and as `orbit` when one is,
keeping its target, band and sense; the transcript records it as
`commander-overruled` (`{"from": "withdraw", "to": "hunt", "why": "the hull is
sound"}`) and the report counts it. A withdraw on a failing hull is kept, as
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
`commander_overruled` (the withdraws the pilot's rule flew as a hunt or an
orbit), and names `commander_off` when it stopped before the match's end.

**Why reasoning must be off.** Measured before this was built, on a picture of
about 1.2 KB and a five-field answer: `deepseek/deepseek-v4.1-flash` through
OpenRouter with reasoning off answered in a median 1 342 ms on default routing
(313 to 1 597 ms, eight of eight valid) and 333 ms pinned to Together (295 to
479 ms, ten of ten valid). With `reasoning = { effort = "low" }` the median was
10.6 s, the slowest 56.8 s, and five answers in eight spent the whole 1 500-token
budget thinking and returned nothing. A commander answer is useful only inside
the picture's own refresh, so send `reasoning = { enabled = false }` (and pin a
fast provider when the latency matters). `bots/arena/commander.toml` does both;
pinned to Together at `commander_ms = 1500`, the lead's live smoke of
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
