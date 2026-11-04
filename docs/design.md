# A guide to the design

RustMUSH is building a persistent science-fiction world around the year 2350:
ships connect settlements, organizations control resources, and players trade,
mine, build, fight, and recover what survives. The MUSH supplies the social world;
TacSim supplies its physical and economic consequences.

The [design catalogue](design/README.md) holds the specifications, decisions,
implementation records, and reviews. This page is a reading guide through them.
For the code behind the systems, use the [crate map](../crates/README.md).

## Start with the world

Read [TacSim foundations](design/10-tacsim-foundations.md) for the premise: a
solar-system industrial warfare sandbox, with inhabited interiors, local tactical
engagements, regional operations, and interplanetary travel. The central idea is
that battles have economic causes and supply chains create reasons to cooperate
or compete.

[The corridor and its factions](design/11-tacsim-setting.md) gives that structure
names and motives: Earth and Luna's Concord, the Republic of Mars, the Freeport of
Ceres, and the companies and crews between them.
[The trade run](design/18-trade-run-vertical-slice.md) turns the premise into a
concrete play loop, with [stations and a ship](design/18a-trade-run-stations-and-ship.md)
to walk around. It is a useful first example of several systems working together.

## Follow a system

| Interest | Start here | Then explore |
|---|---|---|
| Places and travel | [Spatial world](design/14-spatial-room-system.md): rooms, terrain, visibility, and travel share a physical model | [Neighborhoods](design/11-neighborhood-geography-and-authoring.md), [moving sky](design/19-astronomy-engine.md), [communications](design/15-comms-relay-backbone.md) |
| Trade and industry | [Economy and ownership](design/08-economy-organizations-ownership.md): accounts, assets, custody, and obligations | [Trade run](design/18-trade-run-vertical-slice.md), [surface mining](design/20-surface-mining.md), [production chain](design/39-industry-production-chain.md) |
| Piloting and combat | [Combat arenas](design/16-combat-arena.md): authoritative real-time simulation and observer-specific views | [IC combat fields](design/32-ic-combat-field.md), [ship flight and combat](design/35-ship-free-flight-combat-and-mining.md), [recovery and repair](design/36-ground-combat-after-the-field.md) |
| Units, crews, and autonomy | [Unit kit](design/22-unit-kit.md): the definitions that make a unit | [Detachments](design/26-detachments.md), [unit minds](design/17-unit-minds.md), [crew custody](design/42-crew-custody-and-capture.md) |
| Building the world | [World generator](design/30-world-generator-v2.md): terrain drafts, validation, and published packs | [Unit designer and refit bay](design/31-unit-designer-refit.md), [Content Studio](design/34-content-studio.md), [neighborhood brief template](design/neighborhoods/TEMPLATE.md) |
| What players see | [Graphics engine](design/21-graphics-engine.md): one frame across terminal and browser renderers | [Client performance](design/24-performance-client-debt.md), [trade acceptance](design/25-trade-acceptance.md) |
| Engine and operations | [Mechanical contracts](design/09-rules-engine-contracts.md): authority, transactions, clocks, and projections | [Architecture review](design/28-argon-arch-design-review.md), [SSH doorway](design/29-sshdoorway.md), [gateway generations](design/33-gateway-generations.md) |

These are connected systems. A ship's passage uses the sky and route model; its
cargo belongs to the inventory and economy; its occupants still inhabit rooms;
its communications follow the relay network. Combat hands persistent units into
an arena and returns their outcomes to the world. Each authority has one writer;
the world and a running arena communicate through explicit boundaries.

Unit minds are deterministic, sandboxed tactical programs with observation and
execution limits. They are distinct from LLM-driven conversational NPCs and the
[external playing bot](bot.md). The [mind specification](design/17-unit-minds.md)
explains their authoring, proving, replay, and licensing model.

## What is built, and what comes next?

The repository has implementation records for the trade run, surface mining,
combat simulator and IC fields, unit minds, graphics, authoring tools, and gateway
generations. Their limits and follow-up work matter as much as their initial
completion. Start with each design's **as-built** sections and companion **cut
notes** when checking a particular capability.

The [IC phase close-out](design/37-ic-phase-closeout-2026-11-02.md) is a dated
assessment of what landed and what remained open on November 2, 2026.
[The next-systems programme](design/38-after-the-ic-phase-next-systems.md) connects
the subsequent work on production, field combat, restoration, crew custody, and
licensed minds. These programmes have individual implementation steps; a ruled
design is not a claim that every step is complete.

| Active programme | Specification | Implementation record |
|---|---|---|
| Ore, parts, and unit construction | [Production chain](design/39-industry-production-chain.md) | [Industry cut notes](design/39-cut-notes.md) |
| Field control, reinforcement, and component effects | [Field, second round](design/40-field-second-round.md) | [Field cut notes](design/40-cut-notes.md) |
| Restoration and the salvage loop | [Restoration](design/41-restoration-and-the-salvage-loop.md) | [Restoration cut notes](design/41-cut-notes.md) |
| Crew custody, capture, and treatment | [Crew](design/42-crew-custody-and-capture.md) | [Crew cut notes](design/42-cut-notes.md) |
| Test coverage and cost | [Testing strategy](design/43-testing-strategy.md) | Decisions, review dispositions, and implementation steps are in the specification |

Ship flight has its own [remediation review](design/35c-remediation-train-review.md)
and [cut notes](design/35-cut-notes.md). The review covers SF.R9–SF.R16; the cut
notes were corrected from its findings, and SF.R17–SF.R18 have landed since. The
train is not yet seam-complete: 35c §8 lists the remaining cuts.

## Reading the records

- **Designs and owner rulings** explain intended behavior and settled choices.
  “Draft” is a document revision label; some drafts contain accepted decisions
  and implemented systems.
- **Plans** divide work into numbered steps called *cuts*. **Cut notes** and
  **as-built sections** record what actually landed, with deviations and evidence.
- **Reviews** describe a particular commit or date. Findings, test counts, and
  deployment observations belong to that baseline, not automatically to today's
  checkout. Material under [`design/proposals/`](design/proposals/) is input to
  the decision process; acceptance is recorded in the resulting design.
- **IC** means in-character: the persistent game world and its calendar. **OOC**
  means out-of-character, such as a simulator exercise. Lettered identifiers
  (`E7`, `WG.3`, `GC.2`) name steps within a particular programme.

The early platform documents remain useful architectural history, but the project
has since committed to one TacSim game. References to selectable WoD/Shadowrun
rulesets, SQLite, Lua, hex maps, or a permanently frozen sky need the
[superseded decisions](design/README.md#superseded-decisions) beside them.
The [original plan](PLAN.md) preserves the initial treaty rulings; later explicit
decisions govern the areas they amend.

[Browse the full design catalogue →](design/README.md)
