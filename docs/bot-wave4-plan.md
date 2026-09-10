# Wave 4: daily activities and learning

## Design

Build the quiet-life foundation in the external player only. Preserve the SSH
boundary, character-local evidence, JSON executable records, existing model cost
admission and the conversation scheduler. The authored character remains immutable.

1. Project an inspectable directed map from traveled journeys and cited navigation
   reports. Failed travel invalidates an edge until newer travel establishes it.
   Preserve reported and uncertain edges without treating them as traversable proof.
2. Add versioned JSON activity plans: clock domain, recurring windows, priorities,
   prerequisites, destinations and revisioned procedures. Steps require observed
   receipts; timeout or rejection suspends them. Decisions, actions and unsolicited
   speech have separate limits. Choose the current window, never replay missed
   windows. Engagement and explicit goals take precedence. Persist progress and
   counters; do not automatically replay an unsettled command after reconnect.
3. Add cited lesson records covering assessment, demonstration, practice, feedback
   and transfer. Participants declare PC, external bot or game NPC. Keep corrections
   in revision history and surface the latest instruction in future model contexts.
   An inspectable PC lesson does not change a PC sheet or control their character.
4. Add sourced development proposals with frozen, suggestion-only and bounded
   automatic policies. Apply permitted changes only to the bot's learned overlay;
   keep temporary condition, practice and authored biography separate.
5. Expose activity, map and learning state through REPL, TUI snapshots, model
   context, reports, bookmarks and certified campaign checkpoints. Extend save
   versions while retaining verification of genuine wave 2 and wave 3 saves.
6. Check in dog-walker and guitar examples. Test quiet scheduling, interruption,
   route changes, both PC/bot teaching directions, correction reuse and restoration
   through the real bot process and public SSH, with deterministic model tapes.

## Adversarial plan review (before implementation)

This is a self-review, not an independent reviewer endorsement.

- **False accomplishment:** a command echo, utterance or elapsed timer can resemble
  success. Match receipts only against observed output associated with the command;
  discard speech and its wrapped continuations. Missing receipts suspend progress.
- **Noise and loops:** an idle routine must not call the model or issue repeated
  look commands. Deterministic scheduling does neither. Bound commands and decisions,
  coalesce windows and stop at unknown routes. Keep unsolicited speech disabled by
  default and use existing public speech pacing when enabled.
- **Interruptions:** pause cannot undo an already sent command. Reconcile its
  observation before resuming; cancel unsent intents and recompute location/routes.
  Do not overwrite the runner's goal with a scheduled activity.
- **Knowledge laundering:** reported routes and lesson prose are evidence of a
  report, not hidden world truth or awarded skill points. Require exact local
  citations and preserve participant attribution. Reject sent-only evidence.
- **Corrections:** historical practice is not erased by a new instruction. Current
  procedure/lesson revisions appear before history, and superseded procedures cannot
  silently continue from an old step offset.
- **Personhood:** automatic growth is opt-in, bounded and restricted to the logged-in
  external bot. PC and engine-NPC proposals remain suggestions.
- **Clock confusion:** declare scheduler time explicitly. Local clock advancement
  never sends or implies a change to an uncontrolled server's fictional clock.
- **Restore compatibility:** default fields affect normalized JSON digests. Version
  new contributions, strip only absent default contributions for legacy identity,
  reject hidden nondefault state in old versions, and test actual saved artifacts.
- **Scope:** first foundation supports authored procedures and fallible cited
  learning, not an economy, generic needs simulator, NPC adapter or skill adjudicator.

## Validation and final review

Run focused state-machine and adversarial evidence tests, actual executable SSH
acceptance scenarios, checkpoint roundtrips and historical-save verification. Then
run the complete mush-bot suite, formatting, Clippy and runtime dependency audit.
Review the final diff against each boundary above before committing and publishing.

## Final design review

Self-review of the implementation and executable acceptance evidence, 2026-09-10.

The foundation is implemented in `activity`, `navigation`, `learning` and local
evidence helpers. The runner supplies SSH, pause/breakpoints, speech pacing and
existing cost admission. Actor-local journals back inspection and version 3
checkpoint contributions; runtime/build dependencies remain external-bot-only.

Findings resolved during review:

1. Routine commands initially inherited the previous model conversation's follow-up
   flag. This could request another model turn after quiet travel. Deterministic
   steps now preserve conversation state and temporary mood; the complete workday
   test requires exactly **one** request, for the direct PC engagement.
2. Positive feedback could replace a correction with an unhelpful “good.” Only a
   demonstration or a new correction revision changes the current instruction.
   Captured later requests contain that corrected instruction before application.
3. Several public utterances can share an SSH journal episode. Lesson ordering now
   compares exact quote positions within an episode as well as episode IDs; repeated
   or reversed evidence cannot become a later phase.
4. Pausing and checkpoint cancellation must invalidate an unsent activity intent.
   Sent steps still reconcile their observations while paused. Reconnect suspends
   unsettled work; revision replacement discards its old cursor without refunds.
5. Campaign `/activity load` inputs are resolved and frozen before launching the
   branch, matching tape/probe reproducibility. Full activity history is version-
   validated, not merely the latest snapshot.
6. New defaulted fields would change old normalized save identities. Legacy v1/v2
   serialization excludes only new default contributions; validation rejects hidden
   wave 4 state. Actual saved wave 2 and wave 3 campaigns pass offline inspection.
7. Automatic growth recognizes the existing `account+Character` SSH login syntax,
   while still restricting application to that external bot and permitted keys.

Validation: the complete bot suite passes (248 tests, including nine life tests).
Clippy with warnings denied, formatting and whitespace checks pass. The SSH tests
cover the quiet work/study cycle, map invalidation, response to engagement, both
teaching directions, correction-bearing later actions, PC control, transfer and
checkpoint continuity. Negative checks cover fabricated/sent-only evidence,
coalesced lesson order, unsatisfied prerequisites, exhausted allowances, frozen and
suggestion-only growth, revision changes and unresumable commands. TUI rendering
tests include the new bot/cast panels. All model responses in these tests are
local tapes/stubs; there are no live provider charges.

Deliberate limits remain visible in [the guide](bot-life.md): exact transcript
receipts and lesson interpretations are fallible, a relocated shop needs new
observations or a cited report, procedures are authored/revisioned candidates,
and a manual scheduler clock does not advance fictional time. An engine-NPC
learning adapter, real skill adjudication, live pedagogical evaluation and broader
everyday needs remain separate work. No critical fork requiring user input arose.
