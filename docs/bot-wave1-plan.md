# Wave 1 implementation and review

Scope: the [social campaign roadmap](bot-social-campaign-plan.md), wave 1.
This work starts from `origin/main` at `1fd9299`, in an isolated bot worktree.

## Implementation plan

1. Add versioned JSON conversation policy and inspectable scheduling state.
   Independent mode is the default; a spectator policy disables autonomous
   decisions. Maintain bounded, explicitly heuristic exchange threads,
   participants, addressed-to-self evidence, and open-question candidates.
   Speech labels remain interface labels, not proof of acquaintance.
2. Gate idle model requests on pending attention, a settled speech burst,
   cooldown, and a rolling social request allowance. Retain the existing session
   budgets, costs, goal limits, and runner questions. Silence uses no requests.
3. Hold autonomous communication decisions before applying their effects. Use
   actor-specific seeded pacing, utterance limits, and an observation watermark.
   New output cancels a held reply; new speech cancels social model work. Check
   again immediately before sending, including debugger commits. Explicit
   operator sends and exact JSON scripts retain their deliberate semantics.
4. Add an optional cast speaking-floor policy. Actors advertise only readiness,
   intent identity and priority, never reply text or private character knowledge.
   The parent grants one intent at a time through ordered local stdin controls.
   Address priority, oldest-waiter promotion and no consecutive-speaker monopoly
   provide bounded fairness. A grant expires into revocation, not reassignment:
   wait for the actor's matching completion/cancellation acknowledgment. Missing
   acknowledgment or disconnect fails the cast and stops granting. Retain the
   speaking window after completion. Log floor state and independent-mode
   near-simultaneous sends with their actual measurement scope.
5. Add local scene/campaign/branch provenance, monotonic elapsed time, observed
   output and speech watermarks. Add a versioned bot-state bookmark with hashed
   module contributions, strict version/size checks and a validated JSON round
   trip. Require paused, settled bot state. This is an inspection artifact, with
   no world restore or transport resumption command.
6. Add an on-demand extractive chronicler: current-session observed episodes,
   exact citations and bounded excerpts, observation coverage and omissions.
   Exclude authored priors, inferred beliefs and other actors' journals. Recaps
   stay local; they are never automatically spoken into the room. Zero provider
   calls makes this a useful first foundation for later optional prose summaries.
7. Expose conversation state, reasons, pending intentions, costs, scene provenance
   and floor events in the existing single-bot and cast TUI. Supply JSON policy
   examples and a runnable multi-bot fixture. Document controls and boundaries.
8. Run focused state-machine tests and real-SSH/model-tape regressions, including
   three bots plus a PC, stale replies, silence, slow requests, fairness, lease
   expiry and disconnect. Run all bot tests, formatting, Clippy and the independent
   production dependency check. Review the actual design and diff, fix findings,
   then commit and fast-forward push to current `origin/main`.

## Adversarial plan review (before implementation)

| Challenge | Resolution incorporated into the plan |
| --- | --- |
| A timer expires while a child still holds a queued grant. | Expiry begins a revocation handshake; no next grant until an acknowledgment. Timeout fails closed. |
| The TUI drops a speech-intent event under load. | Coordination consumes durable JSON stdout, never lossy UI delivery. |
| A prepared reply applies memories or acknowledges a question before it is cancelled. | Hold the whole decision; apply only after freshness and permission checks. |
| Someone corrects a fact without using the bot's name. | All newly observed speech affects freshness; held replies also check raw output watermarks, including partial lines. |
| Continuous chatter burns one cancelled LLM call per line. | Debounce before requests and enforce a rolling social request allowance, including cancelled work. |
| Shared floor coordination leaks the bassist's private goals. | Share timing metadata only. Floor participants and grants never become character knowledge. |
| A generic greeting gives every model an excuse to speak forever. | Existing relevance cues, handled attention, explicit silence, bounded utterance/request rates and fair scheduling; no automatic paid follow-up narration. |
| An apparent open question is rhetorical or has already been answered. | Label questions and threads as heuristic candidates; do not certify semantic resolution from punctuation. |
| A recap silently imports old sessions, priors, or offscreen knowledge. | Select current-session observed journal episodes only and include exact local evidence plus explicit incomplete coverage. |
| A bookmark pretends pausing the bot freezes the server or refunds spend. | Require local quiescence, preserve cost references, label non-restorable world state, provide validation only. |
| An ablation hides speech but derived thread context leaks it back. | Keep conversational prompt metadata separate from content; derive any speech content only from the filtered attention window. |
| A cooperative guarantee is mistaken for knowledge that a PC is typing. | Guarantee serialized autonomous client dispatch windows only. Independent collision figures measure harness-observed send events, not unseen human intent. |

No critical user decision blocks this scope. Rich semantic topic resolution,
generated recap prose, routine scheduling and matched world restoration remain
later work. These boundaries preserve a usable, testable wave 1.

## Completion and final design review

The separate design review examined the completed scheduler, runner handoff,
harness protocol, artifact boundaries, prompt/privacy flow, comparison identity,
and inspection UI. Findings were corrected before publication:

| Finding | Final disposition and evidence |
| --- | --- |
| Several aged directed requests could still outrank an older generic request. | Once aged, oldest arrival ranks first. A regression covers two already-aged contenders. |
| A debugger-held decision could acquire the current output revision rather than its request's revision. | Preserve the captured revision through staging, committing and speech preparation. Uncued corrections cannot apply the old memory or utterance. |
| Active goals could request another decision during a speech burst even though idle conversation debounced it. | Autonomous goal decisions now wait for the same speech quiet boundary; explicit stepping and goal/session budgets retain their semantics. |
| New standalone elapsed clocks conflicted with the trace parser's joined-envelope detection. | Distinguish by actor/event envelope, preserve optional old clocks, validate both explicit clocks, and filter joined traces using harness time. Real and legacy traces are tested. |
| Log and scheduler elapsed times originally had different session origins. | Both now use the same monotonic session origin; wall timestamps remain separate. |
| Different floor settings could be grouped as identical experiments. | Freeze each actor policy into the run, record effective initial policies and floor settings, and hash these conditions into comparison variants. |
| A cancelled grant or missing acknowledgment could be mishandled only in process integration. | Actual SSH/model-tape fixtures exercise late grants, expiry/revocation, a lost acknowledgment and disconnect. Missing acknowledgment stops the cast without granting the waiter. |
| A model's decision to remain silent could leave a stale “preparing” reason in the TUI. | Record the declined phase and explanation; waiting, retrieving, preparing and sending have distinct reasons. Policy editing is validated and waits for runner state. |
| The observer inherited a model requirement for `/listen`. | Spectator mode permits listening without a model; recap extraction itself has no model path. |
| A bookmark needed a stable reference and enough configuration provenance for later readers. | Add a content-derived identity, hashed required contributions, the complete agent policy and explicit non-restorable boundary. Round-trip, corruption, missing-state and version checks cover the schema. |
| A dropped final floor UI event could leave an obsolete lease displayed indefinitely. | Periodic floor snapshots refresh the display; protocol authorization continues to use reliable stdout/stdin only. |

The privacy pass confirmed that prompts receive the actor's filtered attention,
character and memory as before. Derived thread contents and the runner's shared
floor are not an alternate route around speech ablations. Floor intents contain
only intent ID and address priority; recaps use current-session observed episodes.
Cancelled requests remain in the existing cost ledger. Production dependencies
remain independent of all game/server/NPC crates.

Accepted foundation limits are documented in the user guide: heuristic threads,
extractive recaps, one declared floor per cast, recognized speech commands only,
harness-receipt overlap measurements, and non-restorable bot bookmarks. Explicit
operator commands/scripts retain direct dispatch. There is no PC typing signal,
automatic narration, offscreen routine scheduler or world restoration in wave 1.

Final validation, 2026-09-10:

- All **215 tests passed**: 133 unit tests and 82 integration/offline/helper tests,
  including seven conversation SSH tests. Fixtures use local model stubs/tapes;
  no paid provider or live game was contacted.
- `cargo clippy -p mush-bot --all-targets --no-deps --offline -- -D warnings` passed.
- `cargo fmt -p mush-bot -- --check` and `git diff --check` passed.
- `cargo tree -p mush-bot --edges normal,build --target all --offline` contains
  `mush-bot` as its only RustMUSH package.
- Both checked-in conversation policies and the three-player cast example pass
  their offline CLI validation.
- Fetched `origin/main`; it remains at the tested base `1fd9299`, so publication
  needs no merge with unrelated work. The change was built in an isolated worktree.

No unresolved blocking design findings remain for this wave's documented scope.
