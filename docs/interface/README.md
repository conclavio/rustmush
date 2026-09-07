# rustmush interface documents

The client-facing surfaces: the SSH line mode (`mush-ansi`), the SSH TUI
(`mush-net-ssh/src/tui`), and the web client (`web/`). This folder holds the
design handoff and the plans that act on it. Programme letter: **I**
(interface); cuts are I1, I2, …; decisions are D-I-n.

| Doc | What it is |
|---|---|
| [mockups/](mockups/) | The Claude Design handoff bundle (2026-09-04): `Current State`, `Redesign` (turn 1, two web directions plus the shared TUI and line mode), `Ruleset TUI` (turn 2, five HUD screens). Open the `.dc.html` files with `support.js` beside them. |
| [01-assessment.md](01-assessment.md) | The bundle read against the tree: what already exists, what the mockups assume, effort by item, suggestions. |
| [02-implementation-plan.md](02-implementation-plan.md) | The I programme plan: eight cuts of groundwork and vertical slices through WoD and Shadowrun, with the TacSim HUD gated on E7. |
| [03-discord-owner-questions.md](03-discord-owner-questions.md) | Discord as the fourth interface (Draft 0.3, owner survey — every question answered, the library spike done: twilight): a follow is a real connection of the character's session; the verified link, the lossy DM policy, the comsys relay, staff paging, the on-call seams (§7a). |
| [04-discord-implementation-plan.md](04-discord-implementation-plan.md) | The D programme plan (Draft 0.7, D1–D6 built): six cuts D1–D6 — identity and the privilege/audit ledger, the follow connection, interaction, the relay, the staff channel and on-call seams, consent — sixteen decisions D-D-1..16, constants, eight open questions with defaults. |

The design docs these build on live in [`../design/`](../design/): 02
(connectivity: SSH, session, web, protocol), 10 and 10b (TacSim foundations
and the E7 plan), 08f (E6, scenes and Shadowrun economy).
