# Wave 3: distinct people acting together

## Design

Deliver actor-local relationships, social claims, negotiated commitments and
retrievable background, with operator perspective inspection and campaign
restoration. Keep the standalone processes and existing controller transport.
No peer-memory RPC is necessary: storage can be shared by the authoring tools,
but each running character receives only its own frozen knowledge grants.

1. Extend JSON group authoring with versioned background facts and explicit
   `known_by` grants. Project these before writing actor affiliations; never send
   another member's private facts or the grant roster to an actor. Store granted
   facts in the actor's durable journal. Retrieve relevant facts without placing
   the entire background catalog in every model prompt. Preserve old revisions
   as historical knowledge; a roommate's new discovery does not update the store.
2. Add versioned social journal events and a deterministic actor-local projection.
   Relationships are directed, evidence-backed assessments across trust, respect,
   affection, resentment and dependence. Claims retain attribution, uncertainty,
   corrections and contradictions. Model annotations require exact local
   evidence and receive feedback separately from the game action.
3. Give commitments a deliberately explicit conversational convention: a named
   proposal identifies participants, task, condition and due trigger; participants
   accept, decline or withdraw through observed speech. Counterproposals preserve
   the original terms. An observed completion report remains a report; fulfillment,
   missed obligations and uncertainty are separate evidence-backed assessments.
   The convention travels in ordinary game speech/pages and works for humans too.
   Informal offers remain ordinary remembered speech until explicitly committed.
4. Feed the model its own relationships, claims, relevant background and accepted
   assignments. Assignments are bounded by the runner mandate and never replace
   its goal or instructions. No additional automatic model requests are needed;
   social annotations use the existing decision request/cost/budget category.
   Private agendas remain in existing scoped profiles and member briefings.
5. Add local REPL/TUI social inspection and JSON export. Extend campaign comparison
   with an operator-only perspective view, evidence and unknown/missing coverage.
   Include a required versioned social checkpoint contribution, validated against
   the journal; preserve it across parent restoration and sibling branches.
6. Ship a four-member band scenario: respect plus resentment, a private agenda,
   accepted/refused assignments, partial rumor transmission and correction.
   Exercise real bot processes over the owned SSH fixture using local tapes.

## Adversarial design review (before implementation)

- **Retrieval can become telepathy.** Filter background before process launch;
  the model cannot request an arbitrary actor's journal. Membership alone grants
  nothing. Selection must happen before token/byte limits so hidden records cannot
  displace visible ones. Empty retrieval means unestablished, not false.
- **A display label is not an introduction.** Commitment participants are terminal
  handles, explicitly distinct from known IC names. Relationships may concern an
  uncertain referent. Never generate identity beliefs from a handle match.
- **Natural-language extraction can manufacture consent.** Use complete explicit
  utterances for binding lifecycle transitions, immutable proposal IDs/terms and
  participant checks. Attempts, quotes of someone else's acceptance, tentative
  wording and completion claims cannot establish acceptance or success.
- **A transcript is not an authenticated identity protocol.** The first adapter
  recognizes ordinary line-mode speech framing; preserve its observation basis.
  Do not claim immunity to a server that permits spoofed speech. Unsupported or
  incomplete framing must not create a commitment.
- **Silent coordinator agreement invalidates a negotiation test.** In-world
  negotiation is the shipped mode. Director-assisted/offscreen agreement remains
  an explicitly documented extension; do not silently fall back to it.
- **One listener cannot certify what everybody heard.** Ledgers are per observer.
  Missing proposals/acceptances remain gaps; a private page updates only its
  recipients. The operator comparison is never character prompt input.
- **Counteroffers can erase accepted work.** New terms get a new ID and new
  acceptance. Existing obligations survive a counteroffer until explicitly
  withdrawn; missed, withdrawn, reported-complete and assessed-fulfilled differ.
- **Exact quotes do not prove semantic entailment.** Relationship/claim/outcome
  annotations are identified as the observer's interpretations. Validate local
  provenance and transitions, without claiming objective truth or automated
  adjudication of arbitrary game mechanics.
- **State loss at forks can create convenient amnesia.** Validate the required
  contribution version and journal projection at restore. Test a branch with
  different acceptance/rumor histories and restore the unchanged parent again.
- **Unbounded background and social history can exhaust context.** Bound input,
  annotation count and retrieval bytes; expose omitted records and retain durable
  history. Passive bookkeeping performs no model calls or unsolicited speech.

## Validation plan

Test knowledge grants, revision/conflict preservation, exact evidence, independent
relationship directions, false consent, membership checks, reused proposal IDs,
counteroffers, reports versus outcomes, private transmission, checkpoint versions
and sibling isolation. Run the real SSH scenario and inspect model captures for
private leakage and accepted assignments. Exercise REPL and TUI inspection, then
run the complete bot suite, formatting, Clippy and the runtime dependency audit.

## Final design review

Completed as a separate self-review after the first implementation and passing
SSH scenario. Findings and resolutions:

- **Prior accomplishments could fulfill new promises.** Outcome annotations now
  require evidence after the observer's recorded acceptance, including ordering
  within an episode. Earlier evidence can describe context but cannot alone
  establish the outcome of a new commitment. Added chronology regressions.
- **Native speech wraps.** The first parser accepted only one-line frames, which
  missed long native RustMUSH proposals. It now groups hanging-indent continuations
  within one settled episode, retaining the exact original block as evidence.
  A regression uses the actual `mush-ansi` wrapper, as a development-only dependency.
  Wrapped testimony also stays speech rather than becoming non-speech evidence.
- **Quotation and channel framing matter.** Page content is unquoted by the
  renderer, so user-supplied quotation marks are now treated as quoted examples,
  not stripped into acceptance. OOC and channel prefixes cannot create IC
  proposals or acceptance. Incomplete or foreign framing remains unsupported;
  the terminal text protocol is not a spoof-resistant identity mechanism.
- **Inspection could repeatedly replay a long journal.** The bot now extends a
  cached projection only over newly appended records. The cache is discarded at
  restoration; checkpoints are independently validated against a full projection.
  Incremental/full projection and restored-cache tests agree.
- **A combined debugger view could leak into play.** Group grants are projected
  before writing actor files, affiliation prompt views omit the full background
  catalog, and comparison construction exists only in operator inspection.
  Exact model captures confirm that the bassist cannot see an ungranted roommate
  preference and the guitarist cannot see a correction he never heard.
- **A restored manifest could silently change initial knowledge.** Campaign groups
  seed only initial worlds. `from_checkpoint` rejects new group seeds and uses the
  inherited affiliations/journal. Changed source revisions cannot replace a saved
  background fact. Existing recipient memories are not retroactively revoked.
- **New serde defaults could break an old parent hash.** Legacy actor v1 saves
  reconstruct their original fields for the combined checkpoint's immutable
  identity. Validation accepts only the injected defaults and rejects unbound new
  state. New actor v2 saves require the complete validated social contribution.
- **A comparison could overstate ignorance or success.** Missing typed claims are
  labelled unestablished: an actor may have unannotated speech in its journal.
  Completion speech remains a report; outcome annotations are fallible observer
  assessments, never engine certification. Poses do not prove physical transfers.
- **A live-log test reader assumed every read ended at a record boundary.** A
  repeated full-suite run exposed a partially written JSONL snapshot in the cast
  debugger test helper. It now parses only newline-completed records, even if the
  pending tail ends inside a UTF-8 character. A regression still rejects corrupt
  completed records. This changes the live test reader, not offline corruption
  checks or the producer's trace format.

The implementation keeps the intended authority boundary. Explicit observed
acceptance supplies a personal subgoal; it never changes the runner goal,
instructions, permissions or paused state. Counteroffers require new IDs and
acceptance. Different observers retain different proposal and rumor histories.
Private agendas remain member-scoped. Social bookkeeping makes no model requests;
annotations use the existing decision purpose and request/cost limits.

Shipped inspection includes REPL retrieval/export, individual and cast TUI panels,
and campaign perspective comparisons. The source-controlled four-member scenario
runs acceptance/refusal and private rumor transmission, then private correction,
withdrawal and parent-restoration branches. A further real-SSH test continuation
records different evidence-backed beliefs from actual checkpoint episode IDs.

Final verification: **239 tests pass** (149 unit tests and 90 integration/offline/
helper tests), including all previous bot coverage. Clippy with warnings denied,
formatting and diff checks pass. The actual standalone CLI completes all four
four-actor branches and exports its comparison with no error/feedback events and
zero provider spend. Its outcomes distinguish accepted work, reported completion,
withdrawal and the unchanged parent. Read-only inspection of the previously
saved five-branch wave 2 CLI campaign also succeeds, preserving its original
checkpoint identities. The normal/build dependency audit contains `mush-bot`
as its only RustMUSH package; the renderer is a development-only test dependency.

Publication base: `origin/main` at `3ab10e9`. Work and validation use an isolated
worktree and target directory, leaving the original checkout and live games alone.

### Deliberate remaining boundaries

- Director-assisted/offscreen agreement and formal commander authority are not
  enabled. They need separately labelled experimental contracts.
- Conditions and due triggers are stored text, not a new automatic clock or breach
  adjudicator. Outcome interpretation remains fallible.
- The first explicit agreement convention favors reproducible consent over
  unconstrained semantic extraction. Ordinary conversation remains available.
- Production server restoration and paid campaign admission retain wave 2's
  existing adapter boundaries. No game runtime dependencies were introduced.
