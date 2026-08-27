# rustmush combined-design critique

## 1. CONTRADICTIONS

**Protocol/output layer â€” the big one. Three docs define three incompatible wire vocabularies:**
- Core: `OutEvent` in `mush-core::output` (`Say`, `Pose`, `Room`, `Channel`, `Page`, `Typed{tag,payload}`, `Prompt`), `Span{text, style: StyleClass}` where style IS the semantic class; ANSI renderer is a pure fn *inside mush-core*.
- Connectivity: `OutputEvent` in a `proto` crate (`Narrative{kind}`, `Sys`, `MailNotice`, `MailList`, `Who`, `Sheet`, `Map`, `Weather`, `SessionInfo`, `Disconnect`), `Span{text, style: Style{fg/bg/bold}, tag: Option<SmolStr>, link}` â€” explicit colors PLUS a separate semantic tag; ANSI renderer is its own `ansi` crate calling module-registered `render_text` callbacks.
- Game systems: string-namespaced frames (`panel.wod.roll`, `chan.msg`, `mail.state`, `weather.state`, `jobs.*`) with delta+keyframe semantics and capability negotiation â€” neither enum has these.
- Concrete collision: a channel message is `OutEvent::Channel{chan, speaker: String, text}` (core) vs `Channel{channel, actor: ObjRef, kind, text}` (connectivity) vs `ChanMsg{seq, speaker, speaker_name, title, kind, text, ts}` (game systems). Pick one; the game-systems shape is the only one rich enough.

**Crate layout / naming â€” every doc invents its own workspace:**
- `mush-proto` vs `proto`/`rustmush-proto`; `mush-net` (one crate) vs `session`+`net-ssh`+`net-web`+`ansi`+`core-api` (five); `mushd` vs `bin/rustmushd`; `mush-sys-comsys/mail/weather` + `mush-mod-wod` vs `mush-comsys/mail/weather` + `ruleset-wod`; `mush-agents` (one crate) vs `mush-worldgen`+`mush-npc` (two). Game systems adds `mush-ruleset`, `mush-dice`, `mush-jobs` absent from core's authoritative layout. Core doc claims to own the workspace; nobody followed it.

**russh version:** core pins `russh 0.60`, connectivity `0.63.x`. Trivial fix, but shows the docs weren't cross-read.

**Async re-entry model:** core mandates `Effects::defer` â†’ `Inbound::Apply(Box<dyn FnOnce(&mut Ctx)>)` as "the ONLY sanctioned way"; LLM doc re-enters via typed `WorldCommand::NpcAct/WorldgenDraftReady` with correlation ids. Closure vs typed-command are different designs (typed is more auditable/testable; closures compose better). Choose one, or bless typed commands as sugar over `Apply`.

**NPC representation:** core: `ObjKind::Npc` first-class kind with `agent:*` attributes; LLM doc: "normal character-like object with flag `AGENT`" + `AgentProfile` attribute. Also NPC scheduling: core's world-scheduler `NpcThink` re-armed by mush-agents vs LLM doc's `AgentHost` map of per-NPC tokio tasks with mailboxes. Both can't be the wake mechanism.

**Command handler type:** core `CmdHandler = fn(&mut Ctx)` (a plain fn pointer) vs game systems `Arc<dyn CommandHandler>` trait objects. Fn pointers cannot capture ruleset/module state â€” core's type is simply wrong for the module system it promises. Also `min_role: Role` vs game-systems lock-gated "Staff power" (not in core's `Powers` list), and game systems' `CommandSpec` carries help text while core's has none.

**Dispatch order:** core pipeline puts "module fallback matchers" *after* the command table; game systems requires per-player comsys aliases checked *before* general command matching (a reserved early stage). With core's order, a player alias shadowed by any command silently breaks. Direct conflict.

**Module API duality:** core's `GameModule`/`Registry` (sync observers, coarse `ModuleTick`, no veto hooks) vs game systems' `Ruleset`/`RulesetRegistrar` (veto-capable `MoveAttempt`, `TickCadence::Fast/Scene`, `panel_feed`, `script_api`, `job_queue`, `SceneStarted/Ended`). Game systems' phase-1 checklist requires `SceneId`, `RulesetStateStore`, `MapBlob`, dynamic sub-second timers â€” none exist in the core doc's World, scheduler, or schema. Either core absorbs the registrar or two plugin systems will coexist badly.

**Sheet storage:** core: `AttrValue::Json` in a `wod:sheet` attribute; game systems: `SheetDoc` (with `schema_version` + `ApprovalState`) via a `SheetStore` trait "implemented by persistence area" â€” which doesn't know about it. Where approval state and sheet audit live is unresolved.

**Zone theme record:** core: ZMO attrs `theme:prose/palette/era`; LLM doc: `Theme{style_guide, vocabulary, banned, exemplars, desc_length, pov_rules}` in `worldgen.theme`. Weather (Â§10.4) references "the zone theme record" â€” three consumers, two schemas.

**Worldgen command surface:** `@llm/desc here=` (core) vs `@gen here=` (LLM doc).

**mush-llm dependency claim:** core's graph has `mush-llm â†’ mush-core`; LLM doc says "mush-llm depends on nothing game-side" â€” yet `CallCtx` contains `ObjId`. Minor; resolve with an `LlmMeta` id newtype or accept the dep.

**Rulesetâ†”agent seam:** LLM doc's `RulesetAgentExt` trait (`sheet_summary`/`ability_tools`/`invoke_ability` over a `WorldSnapshot`) vs game systems' `Ruleset::agent_profile(&SheetDoc) -> AgentCapabilityProfile` + `wod_roll` tool. Same job, different signatures â€” reconcile before P4/Phase 6.

**Scripting namespace:** core exposes `mush.*` to Lua; game systems exposes `rs.<id>.*` (`rs.dice.roll`, `rs.weather.current`). Also core says "no locks" single-writer, while game systems' `WorldCtx` says "under the core's locking discipline" â€” stale phrasing that will confuse implementers.

**Session/audience routing ownership:** core's world task "resolves audiences to connection ids"; connectivity's `SessionManager.send(Target::Char)` does the charâ†’connections mapping itself. Both claim the fan-out. Related: core's `Inbound::Cmd{line: String}` vs connectivity delivering `ClientCommand` including `Req{Query::MailList,...}`, `Resume`, `Pong` â€” core has no path for `Req` panel hydration to reach modules.

## 2. GAPS

- **Accounts don't exist in the core.** Connectivity assumes `AccountId`/`CharId`, argon2 hashes, SSH keys, web tokens "in the core's DB" â€” schema v1 has no accounts, credentials, characters-per-account, or ban tables. Also unspecified: how the async `AuthService` queries a sync single-writer world/DB (needs a read-path decision: direct SQLite read connection? snapshot cache?).
- **Character creation/login end-to-end is unowned.** Connectivity stops at "first character creation"; game systems' `ChargenFlow` starts at an existing character with a sheet. Who creates the Player object, binds it to the account, places it in a start room, gates play behind approval (can an Unapproved char walk around?), names/renames it, enforces name rules? No doc owns this seam.
- **Guest pool provisioning** (Guest1..N objects, reset-on-disconnect) â€” referenced by connectivity, built by nobody.
- **Password reset/recovery** â€” email is optional at signup and there's no reset flow at all; also no `@newpassword` wizard command.
- **LLM spend permissions:** core defines `Powers::BUILDER_LLM` but the LLM doc never gates `@gen` on it, and there's no *per-builder* quota â€” one enthusiastic builder can drain the global worldgen budget. Add per-actor budgets + power check in `LlmService::call`.
- **Sitewide admin tooling:** connectivity says "persistent ban list managed by staff commands lives in the core" â€” no such commands/tables anywhere. Missing: `@boot`, sitebans, `@wall`, queue/`@ps` inspection, loop-lag/uptime `@stats`, `@dump` UX, log tailing for staff.
- **Help system:** game systems says help is generated from `CommandSpec`s; core's `CommandSpec` has no help field and there's no `help`/`+help` command, no topic files, no `help @dig` UX. For a MUSH this is day-one furniture.
- **Game clock:** weather needs season/time-of-day from "the core game clock" â€” core has no game time, IC/OOC time ratio, or calendar.
- **Config file:** LLM defines a `[llm]` TOML section; core/connectivity mention config values (flush interval, ports, trusted_proxies, TLS paths, idle policy) with no owner, format, or reload semantics. Define `mushd.toml` schema + which keys are hot-reloadable.
- **Observability:** `tracing` is name-dropped; nothing on log rotation, log files vs stdout, a metrics endpoint (loop-lag watchdog is promised as a metric), per-conn/session dashboards, or LLM cost dashboards pre-P5. Web `/healthz` exists; nothing reads it.
- **Graceful shutdown sequencing across subsystems:** each doc has a piece (Control msg + Flush barrier; NPC summary flush + CancellationToken; `Disconnect{reason}` event). Nobody specifies the order: stop accepting â†’ notify clients â†’ drain/abort LLM â†’ NPC summaries â†’ final flush â†’ checkpoint â†’ exit. Also crash-only paths (what if the persist thread died?).
- **Backup restore story:** snapshots exist; no restore procedure, no offsite copy hook, no periodic restore verification, no `@dump/verify`.
- **Migration of live data as designs converge:** e.g., attrsâ†’`SheetStore`, `theme:*`â†’`worldgen.theme`. The per-namespace migration machinery exists but no policy for cross-namespace/ownership moves, or for downgrade/rollback after a bad deploy (snapshot-before-migrate should be mandatory).
- **Web token store is a `DashMap`** â€” every web session dies on restart despite 7-day TTL claims; persist token hashes (there's a DB right there).
- **Sheet schema delivery to the web client:** game systems serves schemars JSON Schema "at handshake"; connectivity's `hello` frame has only a features list. Add a schema/panel-manifest frame or `Req` query.
- **Spam/abuse inside the game** (not transport): `say`/`page` flood, page-lock exists, but no per-player mute/ignore list, no `@gag` of players (only channel gag).
- **Testing cross-cuts:** testkit (core), fuzzers (connectivity), MockProvider (LLM), golden dice tests (game systems) â€” but no end-to-end harness spec driving a real russh client + WS client against a real world for CI, and no soak test for the persistence flusher.
- **Time zones / locale for @mail timestamps, WHO idle formatting** â€” trivial but unassigned.

## 3. RISKS (prototype these first)

1. **The proto/session/output seam.** Three vocabularies and duplicated fan-out ownership is the highest-probability integration failure. Prototype in week 1â€“2: one `Say` traveling world-task â†’ SessionManager â†’ (a) russh ANSI and (b) WS JSON, with the merged event enum and the game-systems panel-namespacing requirements folded in. Everything else stacks on this.
2. **Single-writer loop + async edges.** `Effects` closures vs `WorldCommand`s, async `AuthService` against a sync world, AgentHost tasks, `Flush` barriers deferring command output â€” this is where deadlocks/stalls/stale-world bugs live. Prototype the loop with a fake 3-second "LLM" call and a login flow before real features.
3. **mlua/Luau vendored C++ build on Windows MSVC** (core's own top risk) plus the sandbox budget story. CI canary on day 1; the Rhai fallback decision gets expensive later because the Lua API surface will be everywhere.
4. **russh raw-mode LineEditor across the terminal matrix** (ConPTY, tmux, mosh, CJK IME) + the multi-flow login UX (`guest`/`new`/`play`/keys) + the input-context-stack requirement (mail compose, chargen, masked passwords). This is the entire first-session impression; build the SSH MVP against a stub core immediately and fuzz `feed()` early.
5. **Module/ruleset API convergence.** If comsys ships against core's `Registry` while WoD is designed against `RulesetRegistrar`, you get two half-APIs frozen by working code. Merge the traits (trait-object handlers, veto hooks, fast ticks, scenes, panel feeds) *before* comsys â€” comsys/mail are supposed to be the proving ground, so make them prove the merged API.

## 4. PHASING (single unified milestone track, each demoable)

- **M0 â€” Skeleton (wk 1â€“2):** unified workspace (settle names now), CI Win+Linux with Luau canary, `mush-core` object model/matchit/perms, merged `mush-proto` v0, testkit, stdin REPL. *Demo: look/say/@dig in tests.*
- **M1 â€” Two players SSH and talk (wk 3â€“5):** world task + scheduler + merged re-entry model; SessionManager; russh + LineEditor + `play` screen with minimal accounts (create/connect, argon2) straight into a DB-lite store; say/pose/movement/WHO/QUIT. *Demo target from the brief achieved.*
- **M2 â€” Durability + real accounts (wk 5â€“7):** `mush-db` schema v1 **including accounts/characters/bans**, char select, multi-connect + linkdead, snapshots/backups, graceful shutdown, kill-9 test, IpGuard. *Demo: crash â†’ restart â†’ same world, same account.*
- **M3 â€” Building + module API v1 (wk 7â€“10):** locks, zones, `@`-suite; merged Registry/Registrar with input context stack; **comsys + mail + jobs as proving modules**; help generated from specs; admin tools (@boot, sitebans, @stats). *Demo: builders build, channels + mail + tickets over SSH.*
- **M4 â€” Web client MVP (wk 10â€“12):** axum, token auth (persisted), WS envelope/resume, SPA scrollback + input, channel tabs, WHO rail, mail panel. *Demo: SSH and web user in the same room.*
- **M5 â€” Scripting + weather (wk 12â€“14):** mush-script sandbox + hooks; game clock; weather module exercising ticks/zones/LookRendered. *Demo: scripted room trigger; rain rolls in.*
- **M6 â€” Dice + WoD playable core (wk 14â€“17):** mush-dice, sheet schema/store decision implemented, `+sheet`/`+roll` full syntax, damage tracks, wod roll/sheet panels. *Demo: an ST runs a scene end to end.*
- **M7 â€” LLM foundation + worldgen (wk 17â€“19):** mush-llm adapters/budgets/audit/kill switches with `BUILDER_LLM` gating + per-builder quotas; `@gen` single-room draft/review, web preview. *Demo: builder generates and accepts a desc.*
- **M8 â€” NPC agents (wk 19â€“22):** agent runtime (one wake mechanism, decided in M1's model), stimuli/coalescing, action tools, memory v1, WoD `wod_roll` grounding, fallback lines. *Demo: players chat with a bartender who rolls real dice.*
- **M9 â€” Progression + hardening (wk 22â€“25):** chargen wizard (terminal + web) wired to accountâ†’character creation end to end, XP/jobs approval, notes; backpressure/flood/fuzz/load test; observability pass. *Demo: new player signs up, makes an approved character, plays.*
- **M10 â€” Depth:** batch worldgen, memory summarization, weather LLM flavor, cost dashboards, streaming previews. btech seam-test crate stays green in CI from M3 onward â€” that's the standing acceptance test, not a milestone.

Cross-cutting rule: before M1 starts, publish three short "treaty" addenda the four owners sign off on â€” (1) the merged event/proto enum + panel namespacing, (2) the merged module/ruleset registrar + handler type + dispatch order (comsys aliases early), (3) the async re-entry + account/auth read-path contract. Every contradiction above traces to the absence of exactly these three documents.