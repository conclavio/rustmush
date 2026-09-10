# Offline harness analysis

Analyze an existing cast or experiment matrix without starting bots, reading bot
configuration or contacting a model provider:

```powershell
cargo run -p mush-bot -- --analyze-harness party-001 --report party-analysis.json --junit party-tests.xml
```

The input can be a harness artifact directory or its `summary.json`. The optional
JSON and JUnit destinations must be new files. Analysis uses the compact records
inside the summary; it does not follow the recorded `run.json`, configuration,
character, prompt or memory paths. A copied artifact therefore remains usable
after the original workspace or game server disappears.

Use [regression gates](bot-regression.md) to compare baseline and candidate
summaries against JSON thresholds for quality, validity, time, requests and cost.
They add sample minimums and explicit failure/inconclusive results for CI.
Use the [offline trace inspector](bot-traces.md) to investigate individual events
and their recorded request/command associations after identifying a failing run.

The version-1 analysis report contains:

- Overall and per-variant run counts: passed, failed and unknown.
- Recorded run duration totals, minima and maxima, with missing-sample counts.
- Commands sent and model requests across actor sessions, with coverage for each
  counter. `current_goal_model_calls` is separate because changing `/goal` resets
  that runner counter; session request accounting does not reset with the goal.
- Exact decimal session charges and the existing model, profile and prompt-type
  breakdowns. Currency totals remain separate strings. Lifetime and overlapping
  group totals are not added to cast spend.
- Recorded final model/profile selections and explicit simulation markers. Older
  summaries may lack the simulation marker; those sessions remain unknown.
- Per-run failed phases, missing expectations, outstanding control
  acknowledgments and forbidden observations. Diagnostics identify actor, event
  kind and position without copying matcher literals or observed conversation.
- Structured probe evaluations, response validity and check outcomes, grouped by
  both probe ID and rubric revision, with per-actor totals and coverage.
- Debugged, unattended and unknown-debug run counts, plus active and held time
  alongside wall time. Missing legacy markers and timing fields stay unknown.

`known_total: null` means no value was reported. When some values are available,
`known_total` is the sum of those values and `missing` describes the uncovered
samples. A missing actor cost report does not become a zero-cost session; the
ledger's `unknown_cost_requests` also remains visible for sessions with
unreported charges. Provider token counts retain the ledger's existing semantics
and are not reconstructed from missing receipt fields.

Repeated runs group by the entire recorded actor specification, after sorting
the cast by actor name. The exported variant description shows the authored
model, prompt, context and character selectors plus specification hashes. Explicit
actor budget selectors also appear; inherited/unset budgets omit that field. Goals,
roles and initial controls contribute to the hash but are not copied into the
analysis. Actor order and repetition number do not split otherwise equal casts.

These are comparisons of the recorded inputs, not proof that a path's file
contents stayed unchanged. Null selectors mean inherited or unspecified settings.
The final selected model can differ from the initial selector after runtime
switching; session cost breakdowns preserve the models used for requests. A
simulation marker records use of the bot's simulator, not whether a nonsimulated
endpoint was a paid provider or a local test stub.

JUnit contains one case per run. Failed outcomes produce failures, unknown
outcomes produce errors, and missing duration does not become a zero-second
measurement. Each case's diagnostic text is bounded to 8,000 characters, with XML
escaping and invalid XML characters replaced. The JSON report retains up to 128
diagnostics per run and an omitted count. Neither export includes prompt text,
private member briefings, matcher values or raw provider error bodies. Human
investigation can use the original artifact when more detail is needed.

Analysis accepts version-1 summaries up to 64 MB and 4,096 runs. Explicitly
contradictory success records are rejected: a run cannot claim success while
recording failed assertions or an incomplete phase, and an overall success flag
must agree with the run outcomes when all outcomes are known. Missing assertion
status is reported as missing data. Analysis trusts recorded outcomes and does
not rerun assertions against joined traces.

`debugged_runs`, `unattended_runs` and `unknown_debug_runs` reflect the explicit
run debugger marker. Debugging does not itself turn a successful run into a failed
analysis outcome. Regression acceptance checks separately require unattended
classification. `active_elapsed_ms` and `debug_held_ms` retain their own coverage;
analysis does not subtract or invent missing durations from legacy records.

## Structured probe results

`metrics.probes` appears in both the overall report and every experiment variant.
Its `totals` contains recorded evaluations, graded and ungraded evaluations,
passed and failed grades, valid and invalid responses, and passed and failed
checks. Each invocation counts separately, including repeated evaluations of the
same probe against the same captured request.

`by_probe` separates results by the pair of `probe_id` and
`definition_revision`. Changing a rubric while keeping its readable ID creates
a separate comparison group. Each group also contains `by_actor` counts. The
aggregate `by_actor` includes ungraded attempts whose rubric could not be
identified. Counters report observed evaluations, rather than inferring the
number of probes intended by a script.

Probe coverage has three independent dimensions:

- `runs_reported` and `runs_missing` distinguish an explicit probe array from
  older or incomplete summaries. An empty array records no probe evaluations;
  an absent or null array leaves coverage unknown.
- A recorded attempt with a null or missing `grade`, including a cancelled or
  failed request, increments `ungraded`. It does not become a passed or failed
  grade and cannot contribute check outcomes or a rubric identity.
- `omitted_evaluations` retains the harness's `probes_omitted` count as a covered
  metric. Its known total counts evaluations excluded by the summary's storage
  limit, separately from the listed evaluations. A missing omission count stays
  unknown. Omitted evaluations cannot be assigned an actor, rubric or outcome
  from the compact summary, even when every retained grade passed.

Per-run `probes` retain only actor, optional event sequence, rubric/request
revisions, grade validity/success and check IDs with boolean outcomes. Probe
reasons, expected answers, citations and response text are not copied into JSON
or JUnit exports. Failed grades and checks contribute bounded failure diagnostics
with the probe ID and revision prefix. A run claiming success while recording a
failed probe is rejected as inconsistent. A non-null version-1 grade must contain
1–64 uniquely named checks and consistent boolean outcomes; malformed grades are
rejected instead of being coerced into a result.

Analysis accepts at most 4,096 recorded probe evaluations per run. It consumes
the harness's event collection, which can span the whole session, rather than
adding each actor report's bounded recent-grade history again.

The standalone library entry points are `analysis::load(&Path)`,
`analysis::analyze(&serde_json::Value)` and `analysis::junit(&Report)`.
