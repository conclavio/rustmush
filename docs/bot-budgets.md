# Session request and spending limits

Set an independent session budget alongside a bot's goal:

```powershell
cargo run -p mush-bot -- --config mush-bot.local.toml --budget bots/budgets/session.json --goal "Find a flower shop and buy a rose" --tui
```

The policy gates the next model request against the session's cost ledger. It
applies to goal decisions, conversations, `/ask`, `/probe`, and model-only replay.
Changing `/goal` resets its existing turn/time allowance but does not reset this
session's request count or spending. Switching model profiles, resuming, editing
limits and changing memory settings also retain the ledger totals.

## JSON policy

```json
{
  "version": 1,
  "max_requests": 20,
  "max_cost_by_currency": {
    "USD": "0.50"
  },
  "unknown_cost": "block"
}
```

`max_requests` permits at most that many started model requests in this session.
Zero prevents all model requests; omit it or use null for no session request cap.
The maximum configurable value is 1,000,000. A denied attempt never creates a
ledger request, consumes a model turn, or replaces the last captured request.
Pending requests, cancelled requests, provider errors and unusable responses all
count once they have started.

`max_cost_by_currency` contains exact decimal strings, with up to 18 fractional
places. A request is denied when **any configured currency total has reached or
exceeded its limit**. A zero cost limit therefore blocks the first request, too.
Currency keys are three uppercase ASCII letters; at most 16 are supported.
Currencies are never converted or added together. Prior known charges in a
currency without a configured limit remain reported but have no threshold.

When cost limits are configured, the upcoming model must have a known currency
with an entry in the policy. The bot's configuration recognizes its OpenRouter
endpoint as USD. Other endpoints can set `cost_currency` in their model profile.
A model switch to an unknown or uncovered currency cannot bypass the policy.

`unknown_cost` controls requests whose cost or currency was not received or could
not be represented. With `block`, any such request in this session blocks further
model requests when cost limits are configured. With `allow`, only known charges
are compared to thresholds, and unknown charges remain visible in the report.
This setting never bypasses the next model's currency requirement or a reached
request/cost limit. Provider-reported zero is known, rather than unknown.
A request cancelled before its receipt arrived is reported separately as
`cancelled_unpriced_requests`: it is unpriced and may have cost money, but it
does not count as an unknown cost. The runner cancels a pending decision by
itself whenever new addressed speech arrives, so treating each cancellation as
unknown would let one conversation freeze a cost-limited session.

An empty `max_cost_by_currency` disables spending checks, including the
unknown-cost check. For a provider-independent request-only limit, use
`{"version":1,"max_requests":20}`. The default `{}` imposes no new limits and
preserves existing behavior.

### What the spending limit measures

A provider supplies the actual charge after a request. The request that crosses
a spending threshold can therefore exceed it; the bot blocks the next request.
There is no token-price estimate, reservation or provider billing cap here.
Cancellation may prevent a receipt arriving even if the provider charges for the
work. Lowering a policy does not reverse charges or cancel a command already sent.

Limits cover the external bot's own model requests. Manual game commands,
bootstrap commands, JSON command scripts, local recall and prompt previews remain
usable without consuming this budget. Committing an already produced staged
decision does not make another request. Server-side `@gen` charges and other
programs using the same API key are outside this ledger.

Each new bot process starts a new session budget. Lifetime costs remain visible
in `/cost`; these limits do not cap an account or all sessions. Each model-only
replay invocation is likewise its own session. Harness limits are per actor and
per run, rather than a pooled cast-wide allowance.

## Local editing and inspection

| Control | Effect |
| --- | --- |
| `/budget` or `/budget show` | Inspect settings, revision, usage, remaining amounts and blocking reasons |
| `/budget load policy.json` | Validate and apply a policy, then remain paused |
| `/budget set {"version":1,"max_requests":40}` | Replace the whole policy from inline JSON, then remain paused |
| `/budget save new-policy.json` | Save normalized policy JSON to a new file |
| F10 in the single-bot TUI | Pause and edit the policy; Ctrl+S validates/applies, Esc closes |

Applying a valid policy cancels an outstanding model decision using the normal
pause behavior. That request and any received charge remain counted. Invalid
edits leave the current policy intact. Raising a limit can permit more requests;
use `/resume`, `/step` or another local question to continue. Budget denial sets
the bot to `blocked`, so repeated ticks and incoming speech do not automatically
retry a denied request.

The Budget panel shows the next-request check. During an in-flight request,
remaining request count already reflects that start, while its receipt may still
be unknown. This does not discard a response that is already in flight.

The selected actor's Costs panel in the cast TUI includes its budget status.
Cast phase controls remain separate from actor budgets.

## Configuration and reproducibility

Set `budget = "policy.json"` at the top level of a bot TOML config to load a
policy relative to that config. `--budget` overrides it, relative to the current
working directory. A harness actor can set `"budget":"../budgets/session.json"`
relative to its manifest; omitted actor settings inherit the bot configuration.
Every expanded actor is checked before the harness launches processes. Matrices
keep that actor's budget policy unless the authored actor configuration changes.

Validate a policy without loading dotenv, credentials, bot configuration or a
runtime:

```powershell
cargo run -p mush-bot -- --check-budget bots/budgets/session.json --report checked-budget.json
```

JSON policies reject unknown fields, duplicate currencies, negative limits and
numeric currency amounts. The normalized policy revision covers its settings,
including exact amounts; equivalent decimal spellings have the same revision.
Policy files are limited to 16 KiB.

Snapshots and final reports include `budget.settings`, `budget.revision` and
`budget.status`. The status contains usage, remaining request/currency amounts
and typed reasons for denial. Remaining amounts are based on known charges and
are not a promise about future or unreceived costs.

Traces record startup/edited `budget-settings`, an allowed `budget-check` under
its new request ID, and `budget-blocked` for denied attempts. Settings remain
runner controls and are not inserted into character memory or model prompts.
Model-only replay includes the pre-request check and final status in its report;
a denied replay has no request ID and sends no provider request. An inspection or
successful policy validation exits 0; denied headless play exits 2, and denied
replay exits 1. Interactive bots stay available for local controls.

Use [cost accounting](bot.md#session-and-lifetime-costs),
[trace inspection](bot-traces.md) and [regression gates](bot-regression.md) to
inspect the resulting experiments.
