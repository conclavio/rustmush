### 1. Facts, beliefs, rumors, and lies

Extend the NPC knowledge model into a proper epistemic system:

- Canonical fact: what the server says is true.
- Belief: what a particular character thinks is true.
- Rumor: a claim with provenance and transmission history.
- Lie: something deliberately asserted despite contradictory belief.
- Confidence and emotional valence.
- “Who told me?” and “Who have I told?”

NPCs could organically spread distorted information without modifying world canon. Investigation abilities could reveal contradictions or provenance. This would be much more interesting than ordinary vector memory and fits your existing memory seam in [PLAN.md](</C:/Users/faeni/cc/rustmush/docs/PLAN.md:112>).

### 2. Factions, fronts, and clocks

Add deterministic long-running world pressures:

- Faction goals and resources.
- Relationship matrices.
- Territory and influence.
- Progress clocks.
- Scheduled offscreen actions.
- Conditions that advance or reverse clocks.

The simulation decides that the Sabbat gained influence or a corporate extraction succeeded; the LLM only turns those state changes into rumors, news, NPC reactions, and scene hooks. This gives the world motion without allowing an LLM to invent permanent consequences.

> **2026-09-01:** promoted and designed in full — see `scenes_factions_knowledge.md` §3. Factions are now first-class epistemic actors: typed-edge DAG (`ChapterOf`/`SubordinateTo`/`ContractedTo`/`FrontFor`), membership truth vs. knowledge-of-membership, `ControlRecord` territory with de jure/de facto/covert control kinds over abstract Regions, and clearance-gated intel pools generalizing the M12 NPC `principal`. Split into a **registry** (ships first, no dependencies) and this item's **simulation** (fronts/clocks, lands after the canon ledger).

### 3. Player-declared hooks and boundaries

Give players structured ways to say what they want:

- “I want political intrigue.”
- “My unresolved hook is finding my sire.”
- “Invite me to investigation scenes.”
- “Do not involve this character in romantic plots.”
- Lines, veils, fade-to-black, and content intensity settings.

The Storyteller can prefer compatible hooks when offering complications. This is especially important for WoD, where an improvising model can otherwise blunder into intensely personal material.

A scene-level `pause`, `rewind`, `veil`, and “call human ST” mechanism would be excellent.

### 4. Structured scene lifecycle

Make scenes first-class objects rather than only activity in rooms:

```text
Proposed → Open → Active → Paused → Resolved → Archived
```

A scene could contain:

- Participants and observers.
- IC time and location.
- Privacy and invitation rules.
- Content rating/boundaries.
- Objectives and unresolved questions.
- Initiative or turn order when applicable.
- Transcript and resulting canonical events.
- XP/advancement nominations.

That would solve numerous future problems involving stale LLM responses, witnesses, Discord participation, private scenes, and recap generation.

### 5. Canon ledger and retcons

Record important outcomes as structured canonical events:

- Character A learned fact X.
- Door Y was destroyed.
- Faction Z lost two influence.
- Character B owes C a boon.
- This scene occurred before another scene.
- Staff later retconned event 41.

LLM narration and summaries should be derived from this ledger, not become the ledger themselves. Staff could preview the downstream impact of a retcon and selectively invalidate memories or recaps derived from it.

### 6. Relationship model with asymmetric state

Store relationships per direction:

- Trust, fear, affection, suspicion, respect, debt.
- Public role versus private opinion.
- Specific grievances and promises.
- Rate limits on change.

Alice trusting Bob does not imply Bob trusts Alice. NPC experiences can submit proposed relationship changes, while deterministic policy limits how quickly values move. This would give memory recall something meaningful to influence beyond stuffing more prose into prompts.

### 7. Boons, favors, debts, promises, and obligations

These are perfect persistent-game primitives, particularly for Vampire and Shadowrun:

- Typed obligation.
- Debtor and creditor.
- Witnesses.
- Terms and expiration.
- Secret/public status.
- Fulfilled, defaulted, disputed, or forgiven.

NPCs can understand and negotiate them, but the engine owns their lifecycle. They also create story hooks naturally.

> **2026-09-01:** obligations need faction-to-faction endpoints, not only character-to-character — a BattleTech merc contract is exactly a typed obligation between factions (the `ContractedTo` edge plus this primitive), and Camarilla boon-credit doubles as a faction-scoped standing currency (see `scenes_factions_knowledge.md` §3.3).

### 8. “Director” rather than omnipotent AI Storyteller

Give the LLM Storyteller a set of bounded director tools:

- Introduce a complication.
- Advance a clock.
- Offer a choice.
- Bring an existing NPC into the scene.
- Reveal an eligible clue.
- Shift spotlight.
- Ask for a roll.
- Escalate to a human ST.

Each tool would have deterministic eligibility checks. I’d also implement a per-scene “pressure budget” so the director cannot endlessly pile on complications.

### 9. Spotlight and participation fairness

Track approximate scene participation:

- Recent speaking time.
- Number of meaningful actions.
- Who has been directly addressed.
- Who has received plot hooks.
- Whether someone is repeatedly interrupted or ignored.

The Storyteller can be instructed to offer opportunities to quieter participants without dictating their actions. This is one of the few areas where an LLM could make group roleplay noticeably better than a traditional server.

Keep this private and approximate; it should support play, not become a player score.

### 10. Clue and mystery graph

Represent mysteries as an authored graph:

```text
Secret → clues → interpretations → locations/NPCs → possible conclusions
```

The engine decides which clues are eligible based on perception, skills, prior discoveries, and scene state. The LLM describes clues without being given unrevealed answers unnecessarily.

Support redundancy—at least three ways to discover important conclusions—so campaigns do not stall after a missed roll.

### 11. Downtime and asynchronous play

Persistent games benefit enormously from structured downtime:

- Research.
- Training and advancement.
- Healing.
- Crafting.
- Contact maintenance.
- Faction projects.
- Legwork.
- Resource acquisition.

A player submits intent, the system calculates time/cost/roll requirements, and staff or the bounded Storyteller resolves it. Results can generate mail, rumors, clocks, and future scene hooks.

This also maps well to Discord, where full live scenes may be awkward.

### 12. Chronicle bible and canon linting

Give builders a structured “story bible” for:

- Tone and themes.
- Naming conventions.
- Technology level.
- Geography.
- Factions.
- Forbidden contradictions.
- Style vocabulary.
- Canonical terminology.

Then add linting to world generation:

- Exit lacks a reciprocal connection.
- Description mentions a nonexistent direction.
- NPC claims incompatible affiliations.
- Room violates zone era/theme.
- Proposed lore contradicts established canon.
- Description exposes something requiring a perception check.

The LLM can explain suspected conflicts, but deterministic references should identify them.

### 13. Narrative replay and automatic recaps

Build recaps from structured events plus transcripts:

- Personal recap: only what that character perceived.
- Scene recap: shared public events.
- Staff recap: hidden rolls and secret consequences.
- “Previously on…” chronicle recap.
- NPC perspective recap, including incorrect beliefs.

Privacy filtering must happen before sending context to the summarizer. A particularly fun feature would be showing two characters’ differing recollections of the same scene.

### 14. LLM evaluation and shadow deployment

Treat personas and Storyteller behavior as testable software:

- Golden conversation scenarios.
- Jailbreak and secret-exfiltration tests.
- “Did the NPC invent knowledge?” checks.
- Ability-result contradiction detection.
- Persona consistency scoring.
- Cost and latency regression gates.
- Replay production snapshots against a candidate prompt/model without publishing results.

Prompt versions should be attached to every audit record. A “shadow route” could run a new model on a small sample and compare outcomes without letting it act.

### 15. Graceful intelligence levels

Instead of simply enabled/disabled:

- Full model dialogue.
- Cheap model dialogue.
- Local model dialogue.
- Retrieval plus templated response.
- Deterministic fallback line.
- Silent ambient NPC.

The server can downgrade automatically based on backlog, budget, provider health, NPC importance, and whether speech was directed. Players should see consistent character behavior rather than an obvious infrastructure error.

### 16. Ruleset-owned scene modes

Let rulesets add modes to the common scene system:

- WoD social conflict, extended actions, chases, combat, frenzy.
- Shadowrun legwork, matrix, astral, vehicle, combat.
- BattleTech strategic, tactical, repair, and economic phases.

Each mode supplies its own structured panels, actions, cadence, and observability while sharing participants, transcripts, permissions, and persistence.

### 17. Server-to-server federation—eventually

A carefully constrained federation model could support:

- Shared identity assertions without shared passwords.
- Cross-game mail.
- Public channel bridges.
- Guest portals between worlds.
- Exportable character dossiers.
- Signed event invitations.

I would keep characters, capabilities, and economic state local. Full object federation would become a security and consistency nightmare, but social federation could be genuinely novel.

My top-five sequencing after your current hardening work would be:

1. First-class scenes.
2. Belief/rumor/provenance model.
3. Faction clocks and deterministic world pressure.
4. Player hooks plus safety boundaries.
5. Canon ledger and perspective-filtered recaps.

Those five reinforce each other: factions produce events, events create scenes, scenes update beliefs and canon, and player hooks determine which developments become playable stories. The LLM then acts as connective tissue and performance—not as the database, rules engine, or source of truth.

> **Sequencing revised 2026-09-01** after the code review (full order in `scenes_factions_knowledge.md`): the **faction registry** moved to the front — it is cheap, has zero scene/canon dependencies, and immediately generalizes the shipped M12/M14 intel machinery — followed by the **ledger substrate** (module store-read seam, chunked-doc append helpers, transaction-boundary test rig: engine prerequisites the review surfaced), then durable scenes/Timestop → scene event log → safety → canon ledger → beliefs → faction *simulation* (fronts/clocks/territory) → director tools.