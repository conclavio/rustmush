# rustmush design documents

The first five documents were produced by parallel design passes (2026-08-27) and reconciled by a
critique pass. They are the *reference* design; where they conflict with each other, the rulings in
[`../PLAN.md`](../PLAN.md) ("Treaty rulings") are binding.

| Doc | Area |
|---|---|
| [01-server-core.md](01-server-core.md) | Object model, world task, command dispatch, module API, scheduler |
| [02-connectivity.md](02-connectivity.md) | SSH (russh), session layer, web server, WS protocol, frontend |
| [03-llm-integration.md](03-llm-integration.md) | Provider adapters, budgets/audit, worldgen pipeline, NPC agents |
| [04-game-systems.md](04-game-systems.md) | Ruleset API, dice, WoD, TacSim seams, comsys, mail, weather, jobs |
| [05-critique.md](05-critique.md) | Cross-doc contradictions, gaps, risks, milestone plan |
| [06-object-behaviors-spatial-domains.md](06-object-behaviors-spatial-domains.md) | First-class object behavior, nested spatial relationships, simulation domains, and the TacSim implementation path |
| [07-npc-control-and-tactical-ai.md](07-npc-control-and-tactical-ai.md) | Link-loss automation, allegiance, typed orders, hybrid tactical AI, belief state, and NPC mobility |
| [08-economy-organizations-ownership.md](08-economy-organizations-ownership.md) | Cross-ruleset economy, first-class organizations, scoped authorization, asset ownership/custody, contracts, housing, and production |
| [08a-economy-build-status.md](08a-economy-build-status.md) | **Build ledger for doc 08**: what of E0–E8 is shipped (commits), the decisions made while building, the live seams, and the E3 pickup point — read first when resuming economy work |
| [08f-e7-implementation-plan.md](08f-e7-implementation-plan.md) | E7 requisition/production vertical slice, its pulled-forward K4/K5 dependencies, migration gates, models, milestones, and fault/restart review checklist |
| [09-rules-engine-contracts.md](09-rules-engine-contracts.md) | Twelve shared mechanical contracts, authoritative action transactions, and five minimum cross-ruleset proving gates |

`06-object-behaviors-spatial-domains.md` and its later companion proposals are
implementation-informed drafts produced by dedicated design conversations deferred in `PLAN.md`.
For their subject areas, prefer them over the earlier sketches while they proceed through review.

## Superseded decisions

Two choices in these docs were overridden after review (owner's call) — read the docs with these
substitutions in mind:

1. **Persistence is Redis 8+, not SQLite.** 01-server-core §5 (rusqlite/write-behind/snapshots) and
   every other doc's SQLite reference are superseded: Redis is the authoritative persistent store
   (RedisJSON object docs, Query Engine indexes, streams, TTL keys) with **write-through**
   per-command MULTI/EXEC pipelines via the `fred` client. The in-process `World` remains a full
   read cache; crash recovery = reload from Redis. See PLAN.md "Persistence" and "Redis data layout".
2. **Scripting is Rhai, not mlua/Luau.** 01-server-core §8's mlua recommendation is superseded;
   the sandbox/budget/hook design carries over onto Rhai's built-in limits
   (`set_max_operations`, call-depth and size caps). Script API namespace is the `mush` Rhai module.
