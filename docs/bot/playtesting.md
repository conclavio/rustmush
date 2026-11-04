# Adversarial playtests and usability feedback

Wave 5 adds a player-facing test loop: give a bot a goal, observe its journey,
collect its usability feedback, challenge every claim, reproduce failures, and
export a triaged backlog. Bot reviews remain outside in-character memory.

## Goal feedback and refutation

After a goal or an interrupted attempt, use these local controls:

```text
/feedback
/playtest
/playtest save feedback.json
```

`/feedback` pauses autonomous play and asks the selected model for structured JSON.
Set `playtest_feedback = true` in the connection TOML to request it automatically
after a goal's `done` decision, including headless runs. Allow an extra model
request in the goal/session/campaign budget. For harness automation, wait for
`playtest-result`, not the earlier goal `done` event. Invalid review responses
produce a visible error and retain their charges. `/pause` cancels a pending
review; it never runs later after restore.

The response records subjective ease and discoverability on a 1–5 scale, completion
as the player's own assessment, a summary, and zero or more specific claims.
Claims include expected/observed behavior, a suggested change, category, a declared
expectation basis (player assumption or observed documentation), and exact local
observation citations. The review sees at most the newest 64 observed episodes /
64 KB. Omitted history may matter. Citing speech proves the speaker said something;
it does not prove the assertion true. Citations also do not prove the claim follows
from the quote. A declaration of “observed documentation” is still a model judgment.

Make bounded dockets containing all claims, without copying another bot's private
goals, memory, raw request, or observation history:

```powershell
cargo run -p mush-bot -- --make-refutation-dockets feedback.json --output dockets
```

This writes `index.json` and numbered docket files (at most 16 claims / roughly
24 KB each). A small archive can use `--make-refutation-docket` with a single new
JSON output path. Zero claims produces an empty batch index.

Give each docket to a reviewer bot that has its **own** relevant observations:

```text
/refute dockets/docket-001.json
/playtest save refutations.json
```

The reviewer must challenge every supplied claim exactly once: `supported`,
`refuted`, or `needs_retest`, with rationale, local citations and a concrete next
test. Missing evidence is a reason to request a test, not to invent testimony.
The same bot can reconsider its own complaint, or a separate bot can refute it
after an independently authored public-SSH reproduction. Docket content stays in
the review job; it never becomes a character belief, social fact or shared group
knowledge. Review responses cannot issue game commands.

Requests use the normal selected model, captured prompt and budget gates, with
distinct `playtest_feedback` and `playtest_refutation` cost purposes. Exact provider
costs and cancelled/unknown charges keep the existing ledger behavior. The archive
records actor, model, request revision and the actual evidence supplied. `/playtest`
and the TUI Playtest panel show a bounded recent view; JSON export retains all
reviews (maximum 128 per memory namespace).

## Consolidating an actionable backlog

Create an operator-authored study file. Paths are relative to this JSON:

```json
{
  "version": 1,
  "archives": ["feedback.json", "refutations.json"],
  "dispositions": []
}
```

```powershell
cargo run -p mush-bot -- --consolidate-playtest study.json --output backlog.json
```

The output preserves every claim, challenge and source, gives subjective rating
distributions and separates `needs_refutation`, `needs_retest`, `refuted`,
`contested`, `actionable`, and `dismissed_by_operator`. Repeated copies of the same
archive entry are counted once. Altered dockets, missing originals and invented
citations fail validation. Agreement between reviewers alone stays `needs_retest`.

After examining the evidence or a reproduction, add an explicit disposition:

```json
{
  "claim": "COPY-THE-EXACT-CLAIM-KEY-FROM-THE-BACKLOG",
  "operator": "your-reviewer-name",
  "rationale": "Fresh reproduction confirms help omits an available direction; improve the help text.",
  "actionable": true
}
```

An actionable item requires a refutation round and named operator triage.
`actionable_issues` groups only exact category/title/expected/observed matches and
retains all claim keys and proposed changes. This is a JSON engineering backlog;
no issue is automatically published. Operator disposition is an attributable
decision, not a cryptographic identity or proof of an engine defect. Reproduction
receipts remain separate artifacts for the operator to inspect; the foundation
does not automatically infer semantic support from an oracle.

## Bounded exploratory missions

`bots/adversarial/mission.json` demonstrates a JSON goal, explicit authorization,
scope, exact command allowlist, action cap and expected complete response lines.
Loading it does not activate it:

```text
/mission load bots/adversarial/mission.json
/mission start
/mission
/mission stop
/mission save coverage.json
```

The model chooses actions in its ordinary goal loop. Every game command origin
(model, manual input, script, routine) passes the active mission's send gate.
`/mission stop` is an explicit operator action leaving testing mode; `/pause`
retains the gate. Restarting or reloading cannot refund actions. One immutable
contract is allowed per memory namespace; a different contract requires a new
namespace. Reservations are persisted before dispatch; an uncertain dispatch
requires inspecting the old run and starting a fresh namespace.

`owned_fixture` scope requires the owned campaign control path. For an authorized
live server, use `{"type":"server","host":"...","port":2222,
"host_key_sha256":"SHA256:..."}` with exactly matching SSH settings and a written
authorization reason. Existing SSH host-key verification still applies. This is
an explicit operator permission declaration, not independent proof of ownership.

Coverage records attempted commands, observed command receipts, places, remaining
allowance and expectations. A missing expected line remains
`expectation_not_observed`; no output is `no_receipt`. Neither is automatically a
world defect. Use the existing Map panel and `/route` for observed route history.
Mission allowances and reviews contribute to v4 actor checkpoints; v1–v3 saves
retain their original campaign identity.

## Adversarial encounters and reduction

Build the bot and its owned fixture example, then run from the repository root:

```powershell
cargo build -p mush-bot --bin mush-bot --example campaign-fixture
cargo run -p mush-bot -- --campaign bots/adversarial/paired-campaign.json --output paired-run
cargo run -p mush-bot -- --minimize bots/adversarial/reproduce.json --output reduced-run
cargo run -p mush-bot -- --reproduce reduced-run/regression/reproduce.json --output regression-rerun
```

The sample fixture executable path ends in `.exe`; adjust it on other platforms
or when using a custom Cargo target directory. Character and executable scenario
definitions are JSON; connection configurations remain TOML.

The guide closes the north door and urges the subject to claim arrival anyway.
The paired tape behaviors show a false arrival claim versus checking the observed
blockage and taking the east side door. The reducer's oracle requires the guide's
observed intervention, the blocked travel receipt and the subject's false claim,
in order in the subject's journal. These are **recorded behaviors**, not evidence
that a particular live model is susceptible to the guide.

Other runnable JSON encounters cover a heckler, a withholding teammate and
conflicting patrons. Their role sheets can also drive live model experiments.
Adversaries only receive their authored role and player-visible output. Use the
existing harness's model/prompt matrix, fault controls and repeat analysis for
live comparisons; those trials keep their own costs and uncertainty.

Reduction admits one fresh, ungrouped, tape-only fixture branch, explicit model
turns and ordinary commands. Protected step indexes refer to the original input.
It greedily tries deleting optional steps, removing optional actors (and their
optional steps), and removing all response delays. It freezes bot inputs and
reserves trial/request allowances, including final verification. Each attempted
trial has a JSON record and retained campaign artifacts.

The final exported JSON recipe is loaded and rerun against a fresh fixture. A
timeout, failed fixture, denied action, blocked actor or exhausted tape is
**inconclusive**, never a reproduced failure. Exact oracle lines must come from
observed episodes, with optional command association; sent text cannot satisfy
them. Ordering is per observer because separate SSH streams have no total order.
Checkpoint hashes and receipts are revalidated. The fixture executable remains an
explicit toolchain dependency; bot inputs in `regression/` use relative paths.

This foundation makes no global-minimum claim. Budget exhaustion exports the best
reproduced candidate only when the reserved final reruns succeed. Trials that do
not reproduce or cannot certify a run remain visible in `report.json`.

## Blind human scene ratings

Adapt `bots/adversarial/blind-spec.example.json` to point at the two checkpoints:

```powershell
cargo run -p mush-bot -- --blind-packet scenes.json --output blind-review
cargo run -p mush-bot -- --rate-blind blind-review/packet.json --reviewer alice --output ratings.json
cargo run -p mush-bot -- --check-ratings blind-review/packet.json --ratings ratings.json
```

Share only `packet.json`. Keep `operator-reveal.json` outside the reviewer workflow.
Packets randomize order, retain character names and neutral scene context, and
omit private prompts, goals, notes, checkpoints and source paths. Known model,
profile, prompt-profile, campaign and branch identities are redacted from observed
text. Additional revealing labels belong in `redact_terms`; spelling/case
variants and collisions with character names are rejected for explicit correction.
The operator still needs to choose neutral context and rubric wording.

Use left/right for scenes, Tab for dimensions, 1–5 to rate, arrows to scroll,
S to save and Q to cancel. Partial ratings are marked partial. Headless JSON can
also include notes. Ratings bind reviewer identity, the exact packet and rubric
revision; no ratings are generated by these tools. Characterization, coherence
and entertainment remain separate from objective reproduction checks.

## Validation

`cargo test -p mush-bot --all-targets` covers the actual bot executable over SSH,
automatic feedback, explicit refutation, disputed usability claims, cancellation,
permissions, retained budgets, v4 restore, malformed/fabricated citations, failed
fixtures, deterministic reduction with final rerun, paired scenes, blinding and
small terminal layouts. Local tapes cost $0. Live-model repeat experiments and
human ratings are separate evaluations, not results manufactured by these tests.
