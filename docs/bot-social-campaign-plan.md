# Five waves for social play and branching campaigns

Status: wave 1 foundation implemented, 2026-09-10; waves 2–5 remain proposed.
See [conversation and scene observation](bot-conversation.md) for shipped behavior,
controls and deliberate limits, and [the wave-1 reviews](bot-wave1-plan.md).
Baseline: the external bot laboratory and character/cast studio published in `a79afef`.

The aim is a usable foundation for all twenty proposed additions, with natural
conversation first and playable story branches early. Waves are dependency
boundaries, not estimates of equal effort. Each ends in a runnable demonstration,
JSON source records, inspection tools, and meaningful regression tests.

## Systems and coverage

| Game system | Original ideas | First usable wave |
| --- | --- | --- |
| Conversation and scene observation | 8. Conversation choreography; 20. Spectator and chronicler | 1 |
| Campaign continuity and alternate histories | 1. Persistent campaigns; 18. Counterfactual scene branches | 2 |
| Relationships, obligations, and organizations | 2. Negotiated leadership; 3. Evolving relationships; 4. Promises; 5. Private agendas | 3 |
| Social knowledge and its transmission | 6. Perspective debugger; 7. Rumor experiments | 3 |
| Daily activity, navigation, and adaptation | 9. Everyday routines; 13. Remembered maps; 14. World change detection | 4 |
| Learning and personal development | 10. Character development; 11. Learned procedures; 12. Teaching and apprenticeship | 4 |
| Investigation and evaluation | 15. Exploratory testing; 16. Adversarial social actors; 17. Failure minimization; 19. Blind human evaluation | 5 |

The common proving ground is the four-piece band from the studio concept. A
second, small travel/commerce scenario keeps the systems useful beyond dialogue.

## Architectural commitments

- Keep the external bot independent from in-game NPC planning and LLM services.
  Bots continue to observe and act through ordinary player SSH sessions. A trusted
  harness adapter may prepare or snapshot an owned test server, but its privileged
  state never enters a character prompt.
- Model a participant separately from its controller: human PC, external bot,
  or in-game NPC. Lessons, commitments, and relationship events should have useful
  semantics for all three. Controller adapters and authoritative game mechanics
  remain separate implementations.
- New behavior policies, sessions, lessons, interventions, and test records are
  versioned JSON; documentation remains Markdown. Continue using the existing
  connection configuration rather than adding a second connection format.
- Give events campaign, branch, actor, observation, and clock provenance. Preserve
  the distinction between authored background, observed events, reported claims,
  and inference. A runner's complete view is never automatic character knowledge.
- Keep physical game outcomes authoritative: saying that someone practiced guitar
  does not award a game skill, and imagining a commute does not move a character.
- Every automated model purpose gets an inspectable eligibility rule, cost
  category, and budget. Silence is a valid outcome and requires no model call
  when deterministic policy already establishes that nothing needs doing.
- Design modules with versioned checkpoint contributions. Later relationship,
  routine, and learning state must join the campaign snapshot without replacing
  the branching implementation. Unknown required state versions block restoration.

## Wave 1 — Conversation that leaves room for people

**Usable result:** three bots can share a room with a person without producing
an instant chorus, while a spectator can record the scene it actually witnesses.

Build on the existing attention window, cancellation of stale model work, and
TUI inspection. Add conversation threads, addressed participants, open questions,
reply intentions, and two separate gates:

1. **Model eligibility:** collect a burst of recent speech, consider relevance
   and direct address, and apply cooldowns and request limits before calling an
   LLM. Routine observations and passive listening can cost zero requests.
2. **Speech eligibility:** pace a prepared reply, allow another speaker a turn,
   enforce utterance limits, and recheck its observation revision just before
   sending. New speech can invalidate, revise, or cancel an unsent response.

Offer two explicit coordination modes. In a managed cast, an expiring speaking
lease can serialize willing bots and prevent starvation. In independent-player
mode, bots use observed speech and seeded scheduling delays; measure collisions
rather than claiming to eliminate them. Neither mode knows that a human is typing.
Timing coordination does not share private memories or confer in-character
leadership. Interruptions can be an authored behavior rather than always a bug.

The TUI shows who is attending, preparing, waiting, or declining to speak, with
the reason and request cost. The spectator joins through its own SSH character;
its recaps cite observed events and remain incomplete when it was absent. Runner
recaps from complete traces must be labeled separately. Start with on-demand or
scene-end summaries rather than continuous model narration.

Also establish stable scene/branch IDs, explicit clocks, output watermarks, and
the bot-state checkpoint schema needed by wave 2. A bot bookmark is not yet a
restorable world snapshot.

**Acceptance demonstration:** a person addresses the band; the appropriate member
answers at a configured pace, others get a chance to join, and a correction
arriving during a held reply prevents stale speech. Test direct address, silence,
lease expiry, disconnects, fairness, and slow models. Cooperative mode enforces
its speaking window; independent mode reports its measured collision rate.

**Boundary:** no claim of humanlike conversation from artificial delay alone;
topic handling and reply eligibility must work too. No routine scheduler yet.

## Wave 2 — Save the moment and explore another path

**Usable result:** stop at a pivotal exchange, preserve the shared past, and run
two or three playable continuations in separate copies of a supported test world.
Keep any branch as the continuing campaign without combining contradictory events.

Introduce a campaign manifest, immutable checkpoint IDs, parent/child branch
relationships, and explicit JSON interventions. Capture each bot's persona,
goals, instructions, memory, knowledge grants, attention state, pending controls,
model/prompt settings, and scheduling state. Ordinary resumption preserves
unfinished work; branching records exactly what changed from the parent.

Implement a distinct **quiesce and checkpoint** protocol. The current harness
debugger holds phase dispatch while bots and the world continue; it cannot serve
as a save-game boundary. For the first supported owned fixture:

1. Fence new ingress and pause autonomous producers, including server NPCs and
   world jobs, at a supported boundary.
2. Drain or explicitly invalidate model requests and outbound actions; retain
   their accounting. A restored pending response must not send an old action.
3. Drain or capture supported queues, freeze the declared game clock, and record
   scheduled work and fixture RNG state.
4. Bind the world snapshot and every bot checkpoint to one epoch and verified
   output watermark. Refuse a checkpoint if the adapter cannot account for
   outstanding work or required state.
5. Restore children into separate server/storage namespaces and reconnect through
   normal SSH. The runner may inspect operator state; characters may not.

Support three honestly labeled artifact modes: **restored world branch**,
**reconstructed run** from setup/prefix replay, and **model-only replay**. Only the
first claims a matched world/bot snapshot. Restoring a database alone is not
sufficient. Begin with a constrained fixture whose full state can be accounted
for; arbitrary live-server rollback is outside this wave.

The TUI gets a branch tree and sibling scene comparison using existing traces,
probes, and cost reports. Model tapes plus a controlled fixture can establish
repeatability. A live provider continuation is a sampled outcome, not a promise
of determinism or proof that an intervention caused a general effect.

Costs stay append-only. Children reference shared history instead of duplicating
its charges; show common-prefix cost, new branch spending, and a campaign-wide
spending cap. Restoring a story never refunds money or replenishes a financial
cap. Forked experiments may receive explicit new turn allowances, recorded as
configuration rather than silently resetting limits.

**Acceptance demonstration:** checkpoint immediately before the bassist objects
to playing only roots and fifths. Try a private discussion, a public confrontation,
and an invitation to demonstrate her arrangement. Restore the parent again and
verify that no child memories, goals, commands, timers, or costs leak into
another child. An unsupported checkpoint fails with a precise reason.

**Boundary:** branch selection is a runner action; no automatic winner selection
or merging incompatible histories. Checkpoint adapters are operator facilities,
not new character powers.

## Wave 3 — Relationships and a group with internal disagreements

**Usable result:** members remember how they treat one another, make commitments,
hold different beliefs, and negotiate work without becoming a shared mind.

Add directed relationship records: trust, respect, affection, resentment, and
dependence can disagree and need not be reciprocal. Record event evidence,
confidence, and changes over time. A character's assessment is not a universal
relationship score or an engine-certified fact. Preserve uncertainty about
people's identities rather than assuming interface names establish acquaintance.

Add commitments with proposer, participants, explicit acceptance, conditions,
deadlines or observable triggers, and fulfillment/breach evidence. Extracting
“perhaps I could help” must not silently become an accepted promise. Distinguish
missed obligations, withdrawal, renegotiation, and an uncertain outcome.

Use the existing studio's motivations and private briefings to support public
objectives, private agendas, and disclosure choices. These are bounded by the
runner's role and permissions. Relationship changes should influence decisions
without rewriting biography or granting knowledge of someone else's intentions.

Leadership initially follows an in-world proposal → response → agreement or
refusal flow. An accepted assignment becomes that bot's own subgoal within its
existing runner mandate. A character's speech cannot replace another bot's local
system instructions. Stronger coordinator authority remains a separately
configured experiment rather than an implicit property of a leader title.

The perspective debugger compares actors' beliefs, evidence, and known gaps.
Rumor scenarios trace actual conversations, attribution, distortion, correction,
and willingness to repeat uncertain claims. The runner can see the comparison;
actors receive only their own knowledge and what they actually hear.

**Acceptance demonstration:** the singer respects the guitarist's ability while
resenting his interruptions. The bassist promises to bring an arrangement but
does not reveal that she is evaluating the band. A mistaken rumor reaches only
some members, is challenged, and leaves different evidence-backed beliefs. The
group negotiates rehearsal tasks, including a refusal, with no hidden goal edits.

**Boundary:** no universal numeric affection simulator and no omniscient rumor
bus. Snapshot these new records through the wave-2 extension contract.

## Wave 4 — Quiet daily lives, learning, and personal development

**Usable result:** a character can work, travel, study, and learn with another
person while remaining responsive to interaction and unobtrusive when idle.

Add interruptible routines with prerequisites, destinations, time windows,
priorities, and resumable activities. Schedules use a declared clock domain;
live response pacing uses monotonic wall time, while controlled tests may use a
fixture game clock. Advancing a test scheduler does not implicitly advance an
uncontrolled server's fictional time.

Give routines three separate controls: decision frequency, game-action frequency,
and unsolicited speech frequency. Most routine bookkeeping should be silent and
deterministic. Coalesce missed schedule windows instead of replaying a backlog
of “now going to work” announcements. Meaningful encounters and direct engagement
can interrupt a routine; completion resumes or revises the plan.

Record arrivals and performed work only when supported by game observations.
Travel still uses actual game commands and produces the world's normal visible
effects. Reduce unnecessary actions rather than hiding meaningful arrivals,
departures, or failures from other players. A muted transcript is not a quiet bot.

Extend existing remembered routes with an inspectable map: traveled edges,
reported connections, uncertain paths, and invalidated routes stay distinct.
World change detection reacts to new observations, such as a blocked exit or
relocated shop, and invalidates affected plans/procedures without continuous
polling of the whole world.

Add reusable procedures with source episodes, applicability conditions, steps,
failure handling, and a revision history. A successful procedure is a candidate
for reuse, not evidence that it works in every context.

Build a small teaching protocol: assess → explain/demonstrate → practice →
feedback → later application. Lessons record who taught what, corrections,
practice attempts, and demonstrated transfer. The participant contract covers
PCs, external bots, and in-game NPCs. This wave implements both directions of
PC/external-bot teaching through the public game interface: the bot owns its
learned state; a PC receives an inspectable lesson record without surrendering
control of the character. From the player's perspective the bot can play the
teacher or student NPC. An existing in-game NPC can contribute public dialogue,
but persistent learning in that separate NPC implementation needs its own later
state adapter; this wave does not claim to update its hidden memory. Start with
procedural knowledge and a guitar-lesson scenario; keep skill-stat advancement
behind real game rules or a separately authorized adjudicator.

Character development proposes a concrete, sourced change: greater confidence,
a revised opinion, or a new ambition. Support frozen, suggestion-only, and bounded
automatic policies. PC sheet changes remain player-controlled; a suggested arc
does not take control of a human character. Preserve the distinction between
temporary mood, accumulated practice, and lasting personality changes.

**Acceptance demonstration:** the dog walker completes a quiet work/study cycle
under a configured action/request budget, handles a changed route, and talks when
engaged. A PC teaches a bot a guitar technique; the bot later applies it in a new
exercise and can teach it back. Correcting the original lesson changes later
behavior. A second test reverses teacher and learner. Neither successful prose
nor a practice timer silently awards engine skills.

**Boundary:** no fabricated offscreen accomplishments, general-purpose pedagogy,
or full needs/economy simulator. Persist only the supported activity/learning
state and make unresumable activities explicit.

## Wave 5 — Turn scenes into a repeatable testing laboratory

**Usable result:** discover a problem, reproduce it, compare alternatives, and
evaluate the result without relying solely on a model's claim that it succeeded.

Add bounded exploratory missions over available player commands, with coverage
tracking, explicit testing permissions, and evidence-backed issue candidates.
Separate a failed player assumption from a demonstrated world defect.

Create adversarial social roles: unreliable guides, hecklers, withholding
teammates, and conflicting patrons. Their access remains ordinary player access;
they do not receive the subject's hidden goals or memory unless the authored
experiment explicitly grants it. Combine them with existing model/transport
faults, assertions, and repeat matrices.

Implement failure minimization first for deterministic tapes and resettable
fixtures. Reduce steps, actors, and delays while preserving the required setup,
intervention, and observed failure. Re-run the minimized case. Live-model cases
need repeated evidence and may remain probabilistic reproductions rather than
stable regression tests.

Add blind review packets and a small local rating UI: preserve scene context
and character identities, hide model/configuration identity and branch labels
that reveal the experiment, randomize presentation, and store reviewer ratings
and rubric versions. Ratings for characterization, coherence, and entertainment
complement objective checks; they do not replace them. Optional model judges
are separately identified, versioned, and budgeted.

**Acceptance demonstration:** an exploratory run finds a broken social/travel
sequence, a controlled adversarial actor reproduces it, minimization exports a
small JSON regression, and paired branch results receive blind human ratings.
Repeated live trials report uncertainty and cost rather than declaring a winner
from one attractive story.

**Boundary:** no unrestricted self-directed testing of shared public worlds and
no claim that a model judge establishes truth. Extend restoration support only
when a new adapter passes its restoration contract.

## Definition of a foundation

Every wave must ship a source-controlled scenario, an inspectable local/TUI flow,
a headless harness path, versioned JSON state, and bounded costs. Deterministic
fixtures test contracts; adversarial cases test failure behavior; live-model
evaluation is explicitly separate. Wave 1 proves bot-state checkpoint round trips;
wave 2 adds matched world/bot restoration for the state implemented at that point.
Every later wave extends and tests that restoration contract for its new state.
All state remains private to the correct participant.

Within a wave, schema/event work, UI inspection, and fixtures can proceed in
parallel once their contracts are agreed. Checkpoint certification, speech
freshness checks, and knowledge boundaries are completion gates, not polish to
postpone. Each wave is useful without requiring every later wave to exist.

## Proposed defaults and decision points

These are implementation recommendations, not approvals already given by the
runner. Record a blocking fork and continue independent work rather than stalling
the whole program.

| Decision | Recommended initial scope | Broader choice left open |
| --- | --- | --- |
| Speaking coordination | Cooperative lease mode for managed casts; observed-speech pacing for independent bots | Whether particular games expose an in-world turn-taking mechanic |
| Restoration, extending D2 | One owned isolated fixture with certified world/bot checkpoints; refuse unsupported state | Additional server adapters and long-lived public campaigns |
| Branch continuation | Runner explicitly chooses the continuing branch; other branches stay available | Automatic selection or reconciliation of alternate histories |
| Group authority, extending D1 | In-world proposals and accepted personal subgoals | Formal commander powers and autonomous runner-goal overrides |
| Learning and growth | Evidence-backed lessons and reviewable development; game mechanics remain authoritative | Ruleset-specific XP, skill advancement, and automated adjudication |
| Background activity | Quiet event-driven routines with explicit clock/action/request/speech policies | Game-specific offscreen simulation and time compression |
| Supplemental model use, extending D3 | On-demand summaries and explicit evaluation jobs with separate costs | Automatic semantic summaries, embeddings, and learned evaluators |

## Proving systems before bringing them into the engine

For each candidate system, retain the behavioral contract, event/state schema,
public observation requirements, invariants, failure examples, and demonstration
scenarios. Those become the evidence for an engine proposal. Relationships and
lessons should describe people and events, rather than be defined as pieces of
an LLM prompt.

When the core implementation is ready, run the same observable scenarios with
external bots, in-game NPCs, and human-controlled PCs where applicable. Keep the
external bot as an independent comparison client. Choose engine-side ownership
of authoritative state deliberately; do not automatically combine the two agent
implementations because a prototype worked.

Existing foundations: [milestones](bot-milestones.md), [decision points](bot-decisions.md),
[cast debugging](bot-cast-debug.md), [studio](bot-studio.md),
[probes](bot-probes.md), and [regression gates](bot-regression.md).
