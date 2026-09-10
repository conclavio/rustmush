# Offline regression gates

Compare two saved harness experiments against source-controlled JSON checks. Use
this after changing a model, prompt, character sheet or recall policy to catch
unacceptable changes in observed quality and resource use.

```powershell
cargo run -p mush-bot -- --check-regression bots/regression/recall-quality.json
cargo run -p mush-bot -- --compare-runs bots/regression/recall-quality.json baseline-run candidate-run --report comparison.json --junit regression.xml
```

Each input is a harness artifact directory or its `summary.json`, in baseline,
then candidate order. Inspection reads these summaries and the policy only. It
does not read dotenv, runtime configuration, referenced artifacts or journals,
start processes, or contact the game or provider. New report paths protect earlier
artifacts. Exit 0 means every gate passed; a failed or inconclusive gate exits 1.
Invalid inputs also exit 1, before creating comparison reports.

## Authoring checks

```json
{
  "version": 1,
  "checks": [
    {
      "id": "recall-quality",
      "metric": "probe_pass_rate",
      "probe_id": "price-claim",
      "minimum": "0.9",
      "max_drop": "0.05",
      "min_samples": 10
    },
    {
      "id": "charge-growth",
      "metric": "provider_cost_mean",
      "currency": "USD",
      "max_increase": "0.02",
      "min_samples": 10
    }
  ]
}
```

The first check requires at least ten eligible probe evaluations in each input,
a candidate pass rate of at least 0.9, and a drop of no more than 0.05 from the
baseline rate. The second limits growth in mean provider charges to 0.02 USD per
actor session. These thresholds are illustrative; set them for the experiment.

| Metric | Value and sample unit |
| --- | --- |
| `run_pass_rate` | Passed runs / runs with known outcomes |
| `probe_pass_rate` | Passed grades / eligible probe evaluations |
| `probe_valid_response_rate` | Valid responses / eligible probe evaluations |
| `elapsed_ms_mean` | Total run elapsed milliseconds / runs |
| `model_requests_mean` | Total model requests / actor sessions |
| `provider_cost_mean` | Total provider charges in `currency` / actor sessions |

`minimum` and `maximum` bound the candidate value. `max_drop` bounds
baseline minus candidate; `max_increase` bounds candidate minus baseline.
Bounds are inclusive. A decrease satisfies any nonnegative increase limit, and
an improvement satisfies any nonnegative drop limit. Rate values use 0–1, so
`"0.05"` is five percentage points, not a five-percent relative change.

Every check needs at least one bound. Thresholds are decimal **strings** with no
sign or exponent, at most 20 whole digits and at most 18 fractional places.
Arithmetic is exact: reports
retain totals and rational numerator/denominator strings instead of rounding
currency or repeating fractions through floating point. If the bounded arithmetic
cannot represent an operation, the check is inconclusive. `min_samples` defaults
to one, is bounded to 1–1,000,000, and applies independently to both inputs.

Use `probe_id` only with probe metrics. Without it, all recorded probe grades are
included. A provider-cost check must specify its currency; conversion between
currencies is never inferred; currency codes use three uppercase letters.
Policy version 1 permits 1–64 checks with unique
IDs of 1–64 ASCII letters, digits, hyphens or underscores. Unknown fields,
incompatible selectors and invalid limits are rejected.

## Selecting experiment variants

By default a check compares the aggregate of each supplied summary. To select one
matrix variant per side, add `baseline_variant` and/or `candidate_variant` with
the full SHA-256 ID from that side's `--analyze-harness` output. The selectors are
independent: changing a model or prompt normally changes the variant ID. An
unmatched selector is inconclusive, rather than falling back to all runs.

Aggregating a matrix deliberately compares its combined results. Different mixes
of actors, variants or probe frequencies can change that aggregate; select
comparable experiments when drawing conclusions. Variant IDs identify recorded
actor specifications and path selectors. They do not prove that referenced file
contents or the game world stayed unchanged. See [analysis](bot-analysis.md) and
[capture comparison](bot.md#planning-and-comparing-experiments) for those limits.

## Missing evidence and comparability

Acceptance checks require an explicit unattended-run marker (`debug.enabled:
false`) in each selected run. Interactive cast debugging is inconclusive,
including debugger sessions that never held a phase. Legacy summaries missing
this marker are also inconclusive; rerun unattended to establish a baseline.
See [phase debugging](bot-cast-debug.md) for how those runs are recorded.

Missing data never becomes a zero value or a passed gate. Inconclusive checks
include insufficient samples, unknown outcomes, incomplete relevant metrics,
missing or omitted probe coverage, ungraded probes, incompatible rubric
revisions, missing or duplicate probe event identities, and incomplete or mixed
simulation evidence. Even with a `probe_id`
filter, an ungraded entry cannot be safely assigned to an unrelated rubric and
discarded.

Probe samples need a positive event sequence, unique for that actor within its
run. Repeating the same recorded event cannot satisfy a larger sample minimum.
Distinct invocations can have identical request hashes, so the hash alone is not
used to deduplicate events. Repeated invocations are still not assumed to be
statistically independent samples.

Probe comparisons require the same set of `(probe_id, definition_revision)`
identities on both sides. Rewording a question or changing expected answers
changes the rubric revision; rerun both experiments with the same rubric for a
comparable pair of summaries. A valid citation establishes that an excerpt exists in the
supplied capture. It does not prove the truth of that excerpt or that it supports
the answer semantically. See [probe evidence rules](bot-probes.md).

Cost means use each actor's session ledger once. Group totals can overlap, and
lifetime ledgers span other experiments, so neither is added. All requests need
known charges in the requested currency; an unresolved cancellation or a missing
currency is inconclusive. A provider-reported zero is known. A cancelled request
whose receipt was already received can still have a known charge. Request means
use session request counts; replacing a goal does not reset this count.

Recorded simulation modes must be known and comparable. Two simulated experiments
can be compared as simulated; simulated and nonsimulated data cannot pass as a
single comparison. A nonsimulated marker means the bot did not use its response
tape. It does not establish that an HTTP endpoint was a live paid provider.

## Reports and CI

Each check reports `pass`, `fail` or `inconclusive`, with bounded reason codes,
the selected scope, observed totals, sample and missing counts, simulation mode,
and probe rubric identities when applicable. The normalized policy is included
with its currency, thresholds and selectors. Revisions identify the normalized
policy and analyzed baseline/candidate content. They are content hashes, not
signatures or provenance verification. Comparison reports omit raw conversations,
prompts, private briefings, answers and provider error bodies.

JUnit writes one case per check. Failed thresholds are failures; inconclusive
checks are errors. Gate success is determined by the authored thresholds, even
when some source runs failed. Include a `run_pass_rate` minimum of `"1"` if every
candidate run must succeed. This permits comparing imperfect models without
silently relaxing a test's explicit requirements.

These are deterministic acceptance checks on observed samples, not statistical
significance tests. Repeat runs and inspect traces when a difference matters.
Elapsed time includes harness and world activity; it is not model latency alone.
