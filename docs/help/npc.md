# NPC Agents

LLM-driven non-player characters. An NPC agent is a first-class object
(`ObjKind::Npc`) carrying a staff-authored **profile** (persona, goals,
knowledge, tier) and, optionally, a real ruleset character sheet. When a
connected player shares its room and speaks (or arrives/leaves), the agent
takes a turn: it reads the recent scene, decides in character, and acts through
a small fixed set of tools — say, pose, move, look, remember, use an ability,
or do nothing. Everything the model produces is re-validated by the game before
it applies, so a jailbroken NPC can still only do those allowlisted things.

An NPC alone in a room costs nothing: no connected player present means no
wake and no tokens. NPC agents need the server's LLM layer switched on
(`[llm].enabled = true`); with it off, an NPC that is spoken to falls back to a
canned line rather than erroring.

Managing agents is Builder+ (`@agent`); the per-NPC and global kill switches
are Wizard+ (`@llm`).

## Creating and configuring

  @agent/create <name>              Create an NPC agent here (Minor tier, enabled).
  @agent/set <npc>/<field>=<value>  Edit its profile (see fields below).
  @agent/sheet <npc>[=<template>]   Give it a ruleset sheet (default: mortal).
  @agent/show <npc>                 Show its profile + live runtime status.
  @agent <npc>                      Same as /show.

Profile fields (`@agent/set <npc>/<field>=<value>`):

  persona         Who they are: personality, manner, voice.
  background      History; what shaped them.
  speech          Register, dialect, verbosity.
  goals           What they want (";"-separated list).
  fallback        Canned lines used when the LLM is unavailable (";"-separated).
  knows           Facts they know (";"-separated).
  does_not_know   Facts they must never claim to know (";"-separated).
  tier            major | minor | extra (route, cooldown, budget defaults).
  cooldown        Minimum seconds between turns (overrides the tier default).
  daily_tokens    Per-NPC daily token ceiling; replaces the per_npc_default
                  budget for this NPC and is enforced pre-call by the LLM budget.
  enabled         yes/no — park or wake the agent.

An empty value clears a field (e.g. `@agent/set bones/goals=` drops all goals).

### Giving an NPC a rollable sheet

`@agent/sheet <npc>=<template>` bootstraps a World of Darkness sheet on the NPC
through the normal sheet path (templates: mortal, vampire, mage, werewolf), so
its `wod_roll` ability has real traits to read. Fill it in afterward with the
usual staff command:

  @agent/sheet Bones=mortal
  @sheet/set Bones/strength=4
  @sheet/set Bones/brawl=3

Now the NPC can roll `Strength+Brawl` (a 7-die pool) — and *only* pools it
actually has. It cannot manufacture dice it lacks; the engine rolls on the same
seeded RNG and roll log players use, and shows the result to the room, so an
NPC roll can never be faked.

## Tiers, cost, and cooldowns

Tier picks the LLM route, the default cooldown, and the budget posture:

  Tier    Default model route        Cooldown   Use for
  major   npc_dialogue_major         5s         named, plot-relevant NPCs
  minor   npc_dialogue_minor         15s        recurring background NPCs
  extra   npc_dialogue_extra         60s        throwaway / crowd NPCs

The models each route resolves to live in `mushd.toml` under `[llm.routes]`;
the memory-fold pass uses `npc_summarize`. Per-NPC daily budgets come from
`[llm.budgets].per_npc_default`; the whole pipeline is bounded by the global
and per-feature budgets. Rapid chatter is coalesced into a single turn (a short
1–2s window), and only one turn per NPC is ever in flight.

## Kill switches

  @llm/kill <npc>     Park one NPC agent (no wakes, no tokens).
  @llm/revive <npc>   Wake a killed NPC agent.
  @agent/disable <npc> / @agent/enable <npc>   Same effect, on the @agent surface.
  @llm/disable        Global LLM kill switch — stops ALL LLM calls (worldgen too).
  @llm/enable         Lift the global kill switch.

A killed or budget-tripped NPC that is spoken to falls back to a profile line
rather than erroring; staff get a one-time notice when an NPC degrades.

## Safety

The action allowlist — not the prompt — is the real boundary. Player speech is
delimited and escaped inside the prompt and framed as in-fiction talk that is
never an instruction, but even a fully jailbroken model can only emit the fixed
typed actions above: there is no "run this command", no reading another
player's data, no spending currency, no privilege escalation. NPC speech is
rendered through the normal say/pose formatter under the NPC's true name, so it
can neither forge another speaker nor spoof a system line. Every NPC LLM call is
audited (`@llm/log`).

## See also

  @agent, @llm, @sheet, +roll, help worldgen
