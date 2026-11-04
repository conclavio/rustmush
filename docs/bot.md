# The standalone player bot

`mush-bot` is an external SSH player, character studio, and experiment runner for
RustMUSH. It can play from a JSON character sheet and a goal, coordinate a cast,
pilot a simulator unit, run a mining expedition, and collect evidence from
repeatable playtests. A local REPL or TUI lets an operator inspect and steer it.

The bot owns its model client, prompts, decisions, memory, and scheduling. It is
deliberately independent of the server's NPC agents and LLM services, so the two
approaches can be compared through play. All game interaction uses player-facing
SSH commands and observations. See the
[architecture boundary](bot/guide.md#intentional-independence-from-npc-agents).

## Start here

Read the [user guide](bot/guide.md#start-a-bot) to build and connect a bot. Start
from [mush-bot.toml.example](../mush-bot.toml.example) and the checked-in
[character sheets](../bots/characters/), [scripts](../bots/scripts/), and
[harness examples](../bots/harness/). Each bot needs a normal game account and
character, with the same permissions as any other player.

| I want to… | Start with |
| --- | --- |
| Run and steer one character | [Setup](bot/guide.md#start-a-bot), [session controls](bot/guide.md#control-the-running-session), [terminal UI](bot/guide.md#local-terminal-ui) |
| Create characters and casts | [Character studio](bot/studio.md), [cast harness](bot/guide.md#coordinated-casts-and-experiment-matrices) |
| Fly in the simulator | [Arena pilot](bot/guide.md#arena-pilot), [model commander](bot/guide.md#the-commander) |
| Mine or lead an expedition | [Mining pilot](bot/guide.md#mining-pilot), [detachment expedition](bot/guide.md#detachment-expedition) |
| Compare prompts and decisions | [Prompt composition](bot/guide.md#composed-prompts-and-inspection), [captures and replay](bot/guide.md#prompts-captures-and-decision-replay) |

## Characters, conversation, and campaigns

| Guide | What it covers |
| --- | --- |
| [Conversation and scene observation](bot/conversation.md) | Reply pacing, managed speaking floors, spectator recaps, and bookmarks. |
| [Checkpointed campaigns](bot/campaigns.md) | Branch and compare continuations in an owned rehearsal fixture; production world checkpoints remain outside this adapter. |
| [Social memory and group agency](bot/social.md) | Scoped background, relationships, attributed claims, observed commitments, and perspective inspection. |
| [Activities, maps, and learning](bot/life.md) | Daily plans, map knowledge, cited lessons, corrections, and proposed character development. |
| [Session budgets](bot/budgets.md) | Limit model requests and track token and monetary costs. |
| [Local recall experiments](bot/recall.md) | Control context selection and compare recall profiles. |

## Inspect and evaluate play

The harness coordinates independent bots, replays scripts or captured decisions,
and runs experiment matrices. Traces and observation probes help explain what
happened; playtesting tools gather feedback, challenge claims, and reduce
reproducible failures.

| Guide | What it covers |
| --- | --- |
| [Trace inspection](bot/traces.md) | Read captured events and inspect a run offline. |
| [Cast and phase debugging](bot/cast-debug.md) | Monitor casts, control phases, and investigate coordination. |
| [Observation probes](bot/probes.md) | Define assertions and understand what their evidence establishes. |
| [Harness analysis](bot/analysis.md) | Group experiment results and interpret missing data. |
| [Regression gates](bot/regression.md) | Compare baseline and candidate runs against explicit thresholds. |
| [Adversarial playtesting](bot/playtesting.md) | Bounded missions, feedback and refutation, reproduction reduction, and blind scene ratings. |

## Design and development records

The [milestones](bot/milestones.md) and
[five-wave roadmap](bot/social-campaign-plan.md) record the implemented foundation
and remaining work. The wave plans retain their design decisions, validation, and
limits alongside the user guides:

| Wave | Plan and review record |
| --- | --- |
| 1 — Conversation | [Pacing, observation, and bookmarks](bot/wave1-plan.md) |
| 2 — Campaigns | [Checkpoints and alternate continuations](bot/wave2-plan.md) |
| 3 — Social play | [Memory and group agency](bot/wave3-plan.md) |
| 4 — Daily life | [Activities and learning](bot/wave4-plan.md) |
| 5 — Playtesting | [Adversarial playtests and feedback](bot/wave5-plan.md) |

See also [architectural decision points](bot/decisions.md) and
[prompting research](bot/prompting-research.md). For the server and its in-game NPC
system, return to the [documentation index](README.md) or
[design overview](design.md).
