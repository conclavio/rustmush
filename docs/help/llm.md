# LLM (staff)

The server's LLM layer powers worldgen (`@gen`, `@gen/building`), NPC agents,
and the weather flavour top-up (`@weather/generate`). Every call is **metered,
budgeted, and audited**, and no generated text is ever committed automatically —
drafts and flavour lines are always reviewed first. This topic covers the staff
surface for watching and controlling spend; the operator setup (providers, API
keys, routes, budgets) lives in `docs/ops.md`.

The whole layer can be off (`[llm].enabled = false`): the commands still exist,
but a generation just says "LLM features are disabled." and no call is made.

## Watching spend

  @llm/budget             Today's global token/cost totals + the kill-switch state.
  @llm/dashboard          The fuller cost dashboard (terminal view of the web
                          LLM panel): budgets vs their caps, the top builders and
                          NPCs by spend today, and the most recent calls.
  @llm/log [<feature>]    The recent audit rows, optionally one feature route
                          (worldgen_room, worldgen_brief, ambience, npc_dialogue_*,
                          npc_summarize).

`@llm/dashboard` and the web **LLM** panel are **Wizard+** (the whole `@llm/*`
staff surface is Wizard, and both surfaces of the dashboard share that floor).
Both show **aggregates only** — feature, provider, model, token/cost totals,
latency, and which builder or NPC spent — and **never any prompt or response
text**. Prompts are audited by hash and a short summary only; they are never
stored in full and never surfaced here.

The web LLM panel is the same data live: staff see an **LLM** tab that hydrates
from the `llm.dashboard` feed. It is advertised to every web client but gated by
role on the server — a non-staff viewer's request is refused, never served.

## Budgets

Spend is capped at several layers, checked *before* a call is made; a call that
would exceed any applicable cap is refused up front (a "budget trip"):

  global        server-wide per day (tokens and/or USD)
  per-feature   e.g. a cap on all worldgen_room calls
  per-builder   each `BUILDER_LLM` holder's daily quota
  per-NPC       any one agent's daily ceiling

Budgets roll over at UTC midnight. A batch `@gen/building` that trips mid-run
stops cleanly, marking the remaining rooms "needs attention" (nothing is
auto-committed). Caps are set in `mushd.toml` (`[llm.budgets]`).

## The kill switch

  @llm/disable            Engage the global kill switch — stops ALL LLM calls now.
  @llm/enable             Lift it.
  @llm/kill <npc>         Park one NPC agent (no wakes, no tokens).
  @llm/revive <npc>       Wake a parked NPC agent.

Use `@llm/disable` if cost or output ever runs away; in-flight drafts finish
harmlessly and everything else falls back to its deterministic behaviour (hand-
written weather lines, NPC fallback lines).

## Spending LLM budget

The commands that actually spend are gated on the `BUILDER_LLM` power (Wizards
hold it implicitly; grant it to a trusted builder with `@set <builder>=BUILDER_LLM`):

  @gen / @gen/building    Worldgen room + whole-building drafts — see `help worldgen`.
  @weather/generate       Weather flavour top-up — see `help weather`.

NPC agents spend on their own turns; see `help npc`.
