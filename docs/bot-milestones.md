# External bot testing milestones

The bot remains a separate SSH player implementation. All executable profiles,
scripts, captures and harness manifests use JSON. Markdown is documentation.

The [five-wave social campaign roadmap](bot-social-campaign-plan.md) extends these
foundations. [Wave 1](bot-conversation.md) adds conversation gates, managed speaking
floors, observer recaps, scene provenance and validated bot bookmarks.
[Wave 2](bot-campaigns.md) adds certified rehearsal-world checkpoints, playable
branches, inherited character state, campaign quotas and sibling inspection.

- [x] Runtime model profiles and session/lifetime provider cost accounting.
- [x] Editable, versioned system prompts and a local terminal UI.
- [x] Captured decision replay without connecting to the game.
- [x] Repeatable experiment matrices with isolated artifacts.
- [x] Assertions on ordered/eventual observations and forbidden actions.
- [x] Scripted interruptions and deterministic model/transport faults.
- [x] Joined request/context/reply/command/observation traces.
- [x] Breakpoints, watches and staged stepping.
- [x] Reproducible context and memory ablations.
- [x] Coordinated multi-bot casts over independent player connections.

This is the first usable implementation of each milestone; memory and behavioral
evaluation remain evolving systems. The single-bot TUI supports local controls and
JSON editing. Coordinated casts run headlessly or in a TUI with phase holds and
stepping, actor selection, separate game sessions, prompt inspection, costs,
memory and affiliations.

The next iteration also implements:

- Named prompt components with ordering, purpose filters, byte budgets, priority,
  required blocks, activation reasons and revisions in the exact request capture.
- Local prompt previews, actual-request inspection, and version-2 composed capture
  replay while preserving legacy profiles and captures.
- JSON lore with explicit character knowledge grants checked before relevance;
  unclassified lore and other characters' private entries are excluded.
- Temporary mood, intent and concerns with applied-turn lifetimes, plus relevant
  voice-example selection that respects context ablations.
- Authored organizations with missions, shared prior knowledge, role assignments
  and private member briefings; actors keep independent memories and SSH sessions.
- Parallel or serial group phases, per-actor acknowledgments, and observation
  fences that prevent previously queued actor output from satisfying a new phase.
- Exact session cost aggregation by cast and overlapping organization, alongside
  the existing per-model, profile and prompt-purpose breakdowns.
- Offline harness plans with expanded matrix runs, group targets and assertions.
- Offline analysis with repeat grouping, exact session costs, explicit missing
  metrics, simulation markers, failure diagnostics and JUnit export for CI.
- Validated capture comparison that distinguishes actual request changes from
  composition metadata, with exact changed content available on request.
- Configurable lexical, recency and combined recall policies, bounded selection
  diagnostics and missing-evidence reports, plus a two-bot recall experiment.
- JSON answer probes with typed fields, private expected answers and deterministic
  checks against episode citations in the exact model request. Live `/probe`
  controls and offline regrading share the same strict response validator.
- Group probe phases with frozen rubrics, per-actor grades, automatic failure
  gates, separate request costs, and analysis by actor and rubric revision.
- Output-field assertion selectors that distinguish a game's response from a
  command's text, with strict JSON pointers and observation-fragment boundaries.
- Offline baseline/candidate regression gates with JSON thresholds, explicit
  matrix variant selection, sample minimums, exact cost and rate arithmetic,
  comparable rubric/simulation evidence, and JSON/JUnit acceptance reports.
- Interactive phase holds and whole-phase stepping with JSON breakpoints,
  revision-checked controls, live observation/failure handling, bounded hold
  deadlines, and separate wall/active timing. Debugged and unclassified legacy
  runs cannot silently pass unattended regression gates.
- Offline trace browsing for saved casts and standalone sessions, with authored
  JSON queries, paginated metadata/content exports, exact input hashes and a
  read-only TUI. Explicit request/command associations remain scoped to each actor.
- Session request and exact spending thresholds with JSON policies, explicit
  unknown-cost handling, local/TUI editing and per-actor harness configuration.
  Goal/model changes preserve usage; all model request purposes share the gate.
- A character/cast authoring studio with ten numbered alternatives, readable
  portraits and structured JSON profiles, natural-language blends, persistent
  selection/provenance, model switching, and a local REPL/TUI draft browser.
  Selected groups export individual roles/goals/private briefings into the
  existing harness through explicit runner-authored account bindings. Studio
  generation has separate prompt-purpose costs and shared session budget gates.

Validation: 195 passing tests (121 unit, 74 integration/test helpers), including
real SSH casts, a four-run matrix, cancelled unsent actions, exact model-only
replay, rejection of unsupported success claims, connection loss and owned
process readiness/cleanup, private group briefings, serial travel coordination,
group cost aggregation, composed request replay, recall-policy changes preserving
the journal, offline analysis of real cast artifacts, group probe success and
fabricated-citation rejection, cancelled probes without stale grades, and offline
CLI planning/comparison/regrading in an environment with unavailable credentials,
quality/cost regression gates on real cast artifacts, missing and duplicate probe
identities, currency thresholds at the eighteenth decimal place, serial phase
stepping across matrix runs, stale controls, live forbidden observations while
held, controller cancellation/disconnect cleanup and separate hold/run deadlines,
offline browsing of actual SSH cast transcripts, strict trace/query validation,
duplicate-key rejection, actor-scoped request associations, filtered exports and
Unicode trace navigation without modifying source records, session request caps
across goal/model changes, exact cost thresholds, unknown and cancelled charges,
budget edits retaining usage, per-actor inherited/overridden limits, zero-budget
command scripts and model-only replay budget gates, studio selection/blending and
resume with unavailable credentials, rejected drafts retaining exact costs,
checkpoint write failures stopping further work, and exported authored casts
reaching real SSH players with scoped profiles, goals and private briefings.
Room memory accepts both legacy and current line displays, including wrapped
multiword exits, while excluding speaker text and object-list references.
TUI layouts and Unicode editing use Ratatui's test
backend; native terminal interaction has not been automated. Clippy is clean.
The normal/build dependency graph contains no game packages other than mush-bot.
Tests use local HTTP stubs or response tapes and incur no live model charges.

See [the bot guide](bot.md#local-terminal-ui) for controls and source examples.
The [probe format](bot-probes.md) explains what these checks establish: exact
answers and captured citations, without inferring semantic entailment or truth.
The [regression guide](bot-regression.md) explains acceptance thresholds and
inconclusive results, including why observed differences are not significance tests.

The [SillyTavern/TavernAI research](bot-prompting-research.md) informed the composer,
lore eligibility, temporary state, voice selection and swappable local recall.
A general plugin execution pipeline and semantic recall/summary jobs remain future work. Decisions
about autonomous group authority, campaign persistence, extension execution and
setting knowledge exports are recorded in [decision points](bot-decisions.md).
