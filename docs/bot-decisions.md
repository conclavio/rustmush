# Bot laboratory decision points

These questions are recorded while independent implementation continues. They do
not block prompt composition, trace inspection, authored groups or scripted group
coordination. All configuration and executable records remain JSON. Offline
execution plans, capture comparisons, CI analysis and local recall-policy
experiments are also implemented independently of these choices.
Baseline/candidate regression policies now gate those saved experiments without
adding a coordinator, changing world state, or choosing an extension runtime.

## D1: Autonomous group authority

Should a pack/coterie/party leader assign objectives through in-world conversation,
or should a separate coordinator be allowed to change members' local runner goals?
These exercise different agent capabilities and produce different evidence.

Current implementation: the source-controlled harness can issue local controls to
named actors or groups. Bots cooperate through their actual SSH sessions. Group
roles and missions inform their decisions but do not confer control over another
bot's runner. Individual briefings and memories stay private.

The character/cast studio can now derive profiles and initial personal goals from
a runner's concept. The runner selects a numbered draft and binds its members to
existing game characters before exporting a harness. This authors starting roles
and goals; it does not grant a persistent coordinator authority over active bots.

Deferred work needing a choice: a persistent autonomous coordinator, its authority
to override personal goals, and whether instructions need member acknowledgment
or can be declined. Progress independent of this decision: group barriers, roles,
per-member briefings, conversation scenarios and observable team outcomes.

## D2: Campaign persistence and world reset contract

For long-lived organizations, which unit should survive a restart: each actor's
memory, the entire organization, or a checkpoint of both the game and all bots?
What server-visible setup/reset procedure identifies a fresh world?

Current implementation: each harness run gets new actor memory directories and
explicit authored starting knowledge. Session costs are aggregated without
re-summing lifetime ledgers. Running against an existing game does not reset it.

Deferred work needing a choice: resumable campaign checkpoints and synchronized
world rollback. Progress independent of this decision: isolated experiments,
captured model-only replay, owned test servers and explicit setup scripts.

## D3: Third-party extension execution and auxiliary model policy

Should external extensions be subprocesses using a JSON protocol, sandboxed WASM,
or compiled Rust components? Should embeddings and summaries use a local model or
a separately budgeted provider profile?

Current implementation: prompt components and lore rules are declarative JSON
processed by the standalone bot. Previews execute no arbitrary code or model calls.
Prompt captures record component provenance and the actual request.

Deferred work needing a choice: third-party executable extensions and automatic
paid embedding/summary jobs. Progress independent of this decision: component
ablations, configurable lexical/recency retrieval, provenance, voice selection
and test tapes. Retrieval-policy experiments now report their evidence selection
and can be compared through the existing cast matrix without auxiliary model jobs.

## D4: Author knowledge versus character knowledge

How should exported game lore identify common knowledge, character-specific
knowledge and author-only secrets? Existing Markdown/text reference directories
do not carry those distinctions.

Current implementation: JSON lore entries require either an explicit common
knowledge flag or a grant to a named character before relevance is considered.
Unclassified entries remain unavailable. Existing supplied reference documents
retain their established behavior, so author-only material belongs outside those
directories. Group shared knowledge is an explicit authored starting grant.

Deferred work needing a choice: a game export manifest with stable knowledge IDs
and rules for acquiring lore through observed play. Progress independent of this
decision: conservative lore activation and tests against accidental omniscience.
