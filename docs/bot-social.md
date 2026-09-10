# Social memory and group agency

Wave 3 adds directed relationship assessments, attributed claims, explicit
commitments and selectively retrieved background to the external player. Each
bot still runs in its own process and interacts with the world through SSH.
The harness coordinates the experiment; it does not supply a shared mind.

## Try the band

```text
cargo build -p mush-bot --bin mush-bot --example campaign-fixture
target/debug/mush-bot --check-campaign bots/social/band.json
target/debug/mush-bot --campaign bots/social/band.json --output social-band-01
target/debug/mush-bot --inspect-campaign social-band-01/summary.json --tui
target/debug/mush-bot --inspect-campaign social-band-01/summary.json --output social-comparison.json
```

Use `.exe` for commands on Windows. The source manifest's `fixture_program`
includes `.exe`; edit that path on other platforms or for a custom target
directory. The fixture uses a fresh ephemeral loopback port and SSH key. No
existing server or provider key is needed. The local model tapes simulate
decisions; this demonstrates protocol behavior, not live-model social ability.

The singer respects the guitarist's skill while resenting his interruptions.
He proposes an arrangement and a tuning task. The bassist accepts one; the
guitarist declines the other. A rumor passes by private pages to the bassist,
then to the guitarist, without reaching the drummer. A private correction,
withdrawal and unchanged-parent continuation produce separate histories.
The bassist's private evaluation of the band remains in her own profile and
background grant. The singer can recall the drummer's familiar preference for
tea; the newer bassist does not receive that grant.

The checked-in tapes cite the singer's fixed authored background episode, whose
ID is determined by the source profile. Do not reuse those evidence IDs with a
different profile/journal. Ordinary model annotations use the IDs in their actual
request. The SSH regression additionally derives tapes from real checkpoint
episode IDs, records different rumor beliefs, and inspects the exact prompts.

## Familiar background without telepathy

Harness and campaign `groups`, and studio group drafts, accept `background`:

```json
{
  "fact": {
    "id": "roommate-drink",
    "revision": 1,
    "subject": "Marco",
    "predicate": "usual drink",
    "value": "Tea before rehearsal",
    "source": "Rose and Marco have shared a flat for two years",
    "confidence": "strong"
  },
  "known_by": ["rose", "marco"]
}
```

The identifiers in `known_by` must be members of that group. Projection happens
before delivery: actor affiliations contain only granted facts, without anyone
else's facts or the grant list. Group membership and roster roles grant no
additional knowledge. A private fact can have a single recipient.

Granted facts enter the recipient's durable evidence journal with authored
provenance, group ID and revision. Relevant facts are selected for a question or
recall query; the full catalog is omitted from ordinary affiliation prompts.
The character can retrieve these memories while the roommate's process is
offline. No request reads the roommate's live private memory. New discoveries
still require an observed conversation, page or other supported game exchange.

Revisions are immutable. Reusing a group/fact/revision with different content
fails; a deliberately authored new revision preserves the older record as
history. Facts are frozen during a run and in campaign checkpoints. There is no
automatic refresh when another character changes their mind or preferences.
Changing grants for a fresh run is an authoring action, not a fictional disclosure.
It does not erase an existing recipient's remembered history; use an isolated
memory namespace when testing a different initial knowledge allocation.

`shared_knowledge` remains a compact prior known by all authored members.
Private agendas belong in each character's profile, member briefing or a scoped
background grant. The studio understands these distinctions. Operator manifests
and studio sessions contain the complete cast and must not be supplied wholesale
as character knowledge documents.

## Relationships and social claims

Decisions can include an optional `social` array, alongside the existing `beliefs`
and command fields. Annotations use the existing decision request and its cost
category. There are no background summarization requests or automatic broadcasts.

```json
{
  "type": "relationship",
  "subject": "Guitarist",
  "dimension": "respect",
  "assessment": "I respect his improvisation, even when I dislike his timing.",
  "basis": "inferred",
  "confidence": "likely",
  "evidence": [{"episode": 42, "quote": "plays a fluent counterpoint"}]
}
```

Dimensions are `trust`, `respect`, `affection`, `resentment` and `dependence`.
The assessment belongs to this observer, may concern an uncertain referent, and
does not imply a reciprocal assessment. It is not a numeric or engine-certified
relationship score. An optional `supersedes` journal ID revises an active
assessment of the same referent and dimension; the old evidence remains.

Claims use `type: "claim"`, `subject`, `predicate`, `value`, `attribution`,
`stance` (`unverified`, `believed`, `doubted`), `basis`, `confidence` and quoted
`evidence`. Optional `supersedes` is an array of active claim IDs for the same
subject/predicate. Contradictory claims can coexist. Corrections are explicit;
they do not change what an absent listener remembers. Attribution itself is the
observer's account, not an authenticated universal transmission graph.

All annotations require exact excerpts from earlier local observation or prior
episodes. Sent commands are attempts and cannot supply social evidence. The
validator checks provenance, field bounds and revision transitions. It cannot
prove arbitrary semantic entailment: an exact quote can still be misinterpreted.
Invalid annotations receive `memory-feedback` without discarding a valid game
action. A model cannot create background grants or acceptance annotations.

## Agreements through ordinary speech

The foundation deliberately uses an explicit convention for tracked agreements:

```text
say I propose [arrangement] for Bassist: bring a counterpoint | condition: none | due: next rehearsal
say I accept [arrangement].
say I decline [arrangement].
say I withdraw [arrangement].
say I cancel [arrangement].
say I completed [arrangement]: I have drafted the counterpoint.
```

Use the appropriate speaker for each response. Only a proposed participant may
accept or decline. Withdrawal requires that participant's observed acceptance.
Only the proposer may cancel, and only while no accepted obligations remain.
Multiple participants are comma-separated. The ID is immutable in each
observer's history. A counterproposal uses a new ID and may append
` | replaces: arrangement`; it does not erase the old obligation or inherit its
acceptance. Negotiate and explicitly withdraw the old assignment when appropriate.

Participants are terminal handles, not automatically known IC names. This
convention also works over supported page framing and for human-controlled PCs.
Each observer records only the proposal and responses they actually receive.
A response to an unseen proposal produces an inspectable gap; it does not invent
the missing terms. Private responses need not produce the same view everywhere.

Only complete explicit utterances change agreement state. Informal discussion,
tentative offers, attempted sends and quoted examples do not bind someone.
The adapter currently recognizes ordinary flush-left line-mode `says`, `pages`,
`whispers` and `tells you` frames, including the native renderer's hanging-indent
continuations within a settled observation. Incomplete blocks or different server
framing may leave the agreement untracked. These are observations, not an authenticated
identity protocol; a server permitting convincing speech spoofing remains capable
of misleading an observer. No extra game privileges are conferred.

An accepted personal assignment enters `social_context.accepted_assignments`
for subsequent decisions. It remains subordinate to the runner's role, goal,
permissions and instructions; neither dialogue nor a leader's title overwrites
them. It does not automatically resume a paused bot. Conditions and due triggers
are retained as text for interpretation; there is no new timer or automatic breach
adjudicator. Those are distinct from the later routine scheduler.

`I completed [...]` records a report. The optional `outcome` annotation records
an observer's `fulfilled`, `missed` or `uncertain` assessment, with a participant,
proposal, explanation, confidence and evidence after acceptance. Fulfillment
requires observed non-speech evidence; it remains fallible, and poses do not prove
inventory transfers or other game mechanics. It never awards skills, items or
engine outcomes. A fulfilled assessment removes the assignment from this
observer's active personal task context but preserves its history.

## Inspection, limits and restoration

- `/social` shows the actor's complete bounded social ledger.
- `/social Marco` searches this actor's social knowledge; `/recall` also searches
  the original episodes and annotations. `/ask` uses the question for retrieval.
- `/social save new.json` exports a single actor's perspective without a model call.
- F9 cycles to **Social perspective** in the bot TUI and **Selected actor social
  perspective** in the cast TUI. The existing context editor exposes `social_bytes`.
- Campaign inspection provides each actor's social ledger and an operator-only
  `social_comparison`: claim histories, contradictions/corrections, agreement views
  and missing coverage. This aggregate never enters character prompts.

An absent typed claim can still exist in unannotated conversation. Missing
coverage means unestablished, not false and not proof that a character is ignorant.
The existing evidence probes still require episode records supplied through
`remembered_experience`; social interpretations are not independent probe evidence.

Default social retrieval allows 16,000 serialized record bytes, at most 64
interpretations and 16 personal assignments plus 16 other proposals. Set
`social_bytes` to 0, or disable context `retrieval`, for ablation. Omitted counts
are visible; the durable journal is retained. The hard foundation limits are 8
annotations per decision, 4,096 interpretations per journal, 1,024 proposal IDs,
128 transitions per proposal, and 256 grants per group. Rejected protocol
transitions leave diagnostic gaps, capped at 128 displayed entries with an omitted
count. The existing journal capacity and request budgets still apply.

Actor checkpoints now use v2 with a required v1 social contribution. Restoration
validates it against the original journal, refusing missing, changed or unknown
required state. Legacy v1 actor checkpoints are accepted only when they contain
no social interpretations or tracked proposals. All new branch interpretations
record their branch scope; inherited events keep theirs. The projection cache is
reconstructed, not trusted as a separate source of knowledge. Bookmarks include
the social contribution but remain non-restorable world bookmarks.

Campaign `groups` seed initial worlds. A `from_checkpoint` continuation inherits
its saved affiliations; do not also supply new `groups`. No silent grant or
personality replacement happens during restoration.

## Deliberate boundary

The shipped mode is `in_world_negotiation`. External coordination handles process
control, timing and scoped authoring/retrieval. It does not negotiate outcomes
privately and then perform a prearranged dialogue. Director-assisted scenes,
offscreen disclosure and formal commander authority remain separate future
experiment modes. They should record their interventions and label their results
before being compared against the embodied negotiation mode.

See [the plan and reviews](bot-wave3-plan.md), [campaigns](bot-campaigns.md),
[conversation pacing](bot-conversation.md), and [the five-wave roadmap](bot-social-campaign-plan.md).
