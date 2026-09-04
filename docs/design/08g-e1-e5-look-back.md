# E1–E5 look-back

**Status:** Draft 0.1 — 2026-09-04. The owner's audit of E1 through E5 before E6 (08f) and E7 (10b)
build on top. Code is grounded at `ba75fc3` (E5c verify fold, "E5 complete"); docs at `d3f2010`,
which differs only by 08h (Draft 0.2) and its README row. Inputs: ten per-cut claim extractions
(2,127 claims from the commits, plans and as-built notes), a verifier pass over every claim, a
refuter pass over every `partial`, and three debt lenses (record, tree, contracts). Every `file:line`
below is at `ba75fc3` unless a commit hash is given; `08x §n` cites a design doc section. 08h §0
names this document's §11 as **the single fold list** for the E5/K6 gate, so D-LBK-1..7 are merged
there under their own ids.

## 1. Verdict

Yes, with qualifications. Of 2,127 claims, 2,041 (96.0%) are implemented as stated, 26 were
superseded by a later, documented cut, 59 were partial and none is missing. The refuters overturned
27 of the 59 — the claim's wording, attribution or cut label was wrong and the tree is right — leaving
**32 confirmed gaps**, none of which moves money wrongly or opens a disclosure today. The headline
gaps: (i) the organization lifecycle exists only as values — no `SetOrgStatus`/`PutRole` op, no close
workflow, and `OrgMembership` has **no production writer at all** (`PutMembership` is constructed only
by the persist arm, `crates/mush-org/src/api.rs:288`), so role templates, `from_role` approvals and
`OrganizationMember` policy subjects are unreachable in play (E1-C122, E1-C127, E1-D23); (ii) staff
freeze/unfreeze/override acts are journaled nowhere (E1-C126); (iii) `register_instrument` is an
unconditional upsert and `Suspended`/`Retired` have no producer (E2-C18); (iv) a title re-issue over a
written-off asset never consults the frozen latch (E3-C36); (v) the "one masking renderer" is three
renderers plus two inline copies (E4c-C110); (vi) a lapsed WoD prompt re-opens in the period it lapsed,
against deviation 8 (E5c-C96). The rest are test gaps and doc drift. The headline **debt** sits mostly
outside the E crates: 08f is unreviewed, grounded pre-E5, and its run board collides with E4b2's roster
lock (DL-01/DL-02); the org/system notification seam and the staff-page fan-out were handed forward by
four cuts and picked up by none (DL-04/DL-05); doc 08's as-shipped notes stop at E3 (C1); CI's blocking
`fmt` step has hidden clippy/test signal since before E3 (T1b) with 2,467 rustfmt hunks awaiting the
post-K6 reformat (T1); and the sub-threshold drip meets org-funded run offers in E6 with no budget
primitive (DL-10).

## 2. Per-cut claim table

| Cut | Claims | Impl. | Partial (confirmed / refuted) | Missing | Superseded | Confirmed gap ids |
|---|---|---|---|---|---|---|
| E1 | 135 | 128 | 7 (6 / 1) | 0 | 0 | C122 C124 C126 C127 C129 C135 |
| E2 | 160 | 134 | 8 (5 / 3) | 0 | 17 | C16 C18 C70 C159 C160 (one claim unstatused in the tally: 134+8+17 = 159) |
| E3 | 250 | 246 | 3 (2 / 1) | 0 | 1 | C36 C123 |
| E4a | 216 | 205 | 6 (2 / 4) | 0 | 5 | C114 C210 |
| E4b1 | 248 | 245 | 3 (3 / 0) | 0 | 0 | C47 C62 C189 |
| E4b2 | 275 | 271 | 4 (3 / 1) | 0 | 0 | C55 C193 C232 |
| E4c | 194 | 184 | 8 (2 / 6) | 0 | 2 | C109 C110 |
| E5a | 178 | 172 | 5 (0 / 5) | 0 | 1 | — |
| E5b | 181 | 175 | 6 (4 / 2) | 0 | 0 | C30 C98 C161 C171 |
| E5c | 290 | 281 | 9 (5 / 4) | 0 | 0 | C2 C90 C96 C116 C231 |
| **Total** | **2127** | **2041** | **59 (32 / 27)** | **0** | **26** | 32 |

**Confirmed gaps, one line each** (verifier evidence; refuter's reason in brackets).

*E1*
- **E1-C122** doc 08 §5.2 close workflow: the never-delete/never-reuse half is built (`mush-org/src/authorize.rs:148-158`, `ids.rs:43-46`, test `allocators_resume_and_never_reuse_ids`), but no op or verb moves an org out of `Active` — `OrgMutationOp` (`plan.rs:25-95`) has no status op and `commands.rs:133` is the tree's only `OrgStatus` write; tests patch the doc and reboot (`mush-economy/tests/e3b2.rs:996-1009`). [08b:578-582 defers `SetOrgStatus`/`PutRole` to "its own cut"; nothing named it since.]
- **E1-C124** doc 08 §15.1 omits `policy:{id}`, which E1b persists (`mush-org/src/api.rs:64-68`, `boot.rs:92`); the E3 note (08:1521-1523) removed only `approval:{id}`. [Only namespace list in any doc; still wrong.]
- **E1-C126** §12.3 bullet 4: freezes, recovery acts and staff overrides are journaled nowhere — `set_asset_frozen`/`set_account_status`/`set_economy_frozen`/`set_vault_status` say "Not journaled" (`mush-economy/src/api.rs:702-786`), only approve/deny reach the org ledger (`commands.rs:1592-1618`). [No `OrgAction` token for a freeze or override; `econ_unfreeze` never records who cleared the latch.]
- **E1-C127** D2's "promote rather than duplicate" half was struck (08:178-181, :498-499; 08a:158-160): `FactionStandingProvider` is the sole bridge, `PutPolicy` has no operational author (`@org` is create/list/show/grant/scope/audit, `commands.rs:32-47`) and D2 carries no "As built". [Consistent with D-E3-11; the doc entry is what is stale.]
- **E1-C129** §5.5: a matching Deny always wins with no specificity ordering (`authorize.rs:210-267`); `AuthWitness` exists on the Allow path only and nothing outside `mush-org` reads it (every caller uses `.is_allow()`); deferred actions are made safe by live re-authorization instead (`mush-economy/src/plan.rs:40-42`). §5.5 never got the "As shipped" note §5.6 did. [Conservative direction; doc reconciliation owed.]
- **E1-C135** `allocators_resume_and_never_reuse_ids` (`tests/e1b.rs:214-227`) replays a complete `meta`, so the `.max(next_after(..))` floor (`boot.rs:193-201`) is implemented but pinned by nothing; every other module pins its floor (`mush-faction/tests/faction.rs:642`, `mush-economy/tests/e3b.rs:858`, `e4b2.rs:1493`, `mush-housing/tests/e5b.rs:1264`).

*E2*
- **E2-C16** `InstrumentDef` (`mush-economy/src/record.rs:28-48`) lacks §6.1's `display: InstrumentDisplay` (08:540); the type is defined nowhere and no decision dropped it (08a:181 covers only the decimal wire form and `MAX_SCALE`).
- **E2-C18** `InstrumentStatus::Suspended|Retired` has no writer anywhere (only `Active` is constructed; `@instrument` is register|list, `commands.rs:711-720`); `InactiveInstrument` is asserted by no test; and `api::register_instrument` (`api.rs:670-675`) is an unconditional upsert — the command (`commands.rs:741-743`) and wod boot (`ruleset-wod/src/economy/boot.rs:307-313`) are the only id-reuse guards.
- **E2-C70** `alloc_transaction()` was dissolved into prepare/install (fd3108c; `api.rs:1852-1856`, `:1979`); nine of ten listed members are as claimed. [Doc-only.]
- **E2-C159** `@econ/mint` accepts any two accounts (`commands.rs:1825-1839`; `parse_account` `:293-295` never resolves a system name); "requires a system source" holds economically (only `@account/system` and wod `ensure_system_account` mint unbounded credit) and by usage text; `api.rs:651-655` overstates.
- **E2-C160** fuzz amounts are bounded to about ±2^40 (`tests/e2a.rs:311`; replay: max |balance| 3.3e13), so overflow is unreachable inside the fuzz; the edge is pinned only by `overflow_is_refused_not_wrapped`. [Test-wording.]

*E3*
- **E3-C36** the `Title` arm re-issues over an unencumbered `WrittenOff` asset (`engine.rs:785-794`, fa6a979) and never consults `a.frozen` (`:828` sits in the non-Title arm); boot's principal-gone sweep (`boot.rs:1099-1109`) can freeze a written-off asset that a `Title` then posts over. Mitigation: post carries `prev.frozen` onto the new generation (`engine.rs:1516-1519`).
- **E3-C123** `@instrument/list` (`commands.rs:753-769`) bypasses `emit_list`/`LIST_CAP`; no test asserts the 20-row cap or the "(N more)" tail on any `*/list` (only `@econ/history`'s own `HISTORY_CAP`, `tests/e4c.rs:756`).

*E4a*
- **E4a-C114** forgiveness writes `Forgiven{..}` and notifies both parties (`sweep.rs:516-549`) but emits no `ObligationChanged` — `put_obligation` emits on a status-kind change only (`api.rs:1426-1443`, 8873863); D-E4-14 (08c:236-239) still promises the event. [Refuter reproduced zero events with a recording observer.]
- **E4a-C210** seventeen `vf_` tests for eighteen items: 14e was a comment-only fix (`hash.rs:127-137`) and `hold_op_order_never_changes_the_hash_or_the_applied_effects` (`tests/e4a.rs:1618`) passes under both readings.

*E4b1*
- **E4b1-C47** a re-delivered duplicate is `Ok(unchanged)` (`obligation.rs:445-447`) — no log, no break; only `OutOfOrder` is loud (`sweep.rs:592-612`). 08c:693-694 overstates. [Doc-only.]
- **E4b1-C62** `@obligation/resume`'s `Suspension` forgiveness informs only the staffer (`commands.rs:3512-3534`); `cause_word`'s "a suspension" arm (`sweep.rs:947`) is unreachable; debtor and creditor are never told.
- **E4b1-C189** the named test (`tests/e4b1.rs:376`) places `expires=ic:1000000` — `WorldIc`, since no command yields `Manual` (`commands.rs:2699-2706`); Manual-never-passes is pinned only at engine level (`tests/e4a.rs:421`).

*E4b2*
- **E4b2-C55** `MAX_AMENDMENTS` is enforced only at `@contract/amend` (`contract_live.rs:2301-2306`); boot's orphan-escrow repair bumps `amendments` unchecked, once per orphan hold (`boot.rs:973-976`); no test reaches the cap.
- **E4b2-C193** no panics (verified by grep and read), but four `ContractState` matches end in `_ =>` (`contract_live.rs:381`, `:2261`, `:2396`; `sweep.rs:1283`) — a new variant would fall into `unwind`'s `Expire` silently.
- **E4b2-C232** the audience is one member (`tests/e4b2.rs:1238`, `:1254-1258`); `canonical_visibility`'s sort+dedup (`contract.rs:972-982`) is permuted by no test.

*E4c*
- **E4c-C109** `emit_list` never sorted (`commands.rs:695-708`, unchanged since 288f769); the comment at `:2080` repeats the wrong reason; "Nothing recorded against account #<id>." is untested.
- **E4c-C110** `@econ/history` never calls `mask_account` (`commands.rs:1960-2068`, its own `other accounts` token); `render_expiry_masked` (`contract_live.rs:2969-2981`) and the `side` closure (`:2998-3004`) duplicate it; the "one renderer" comments (`commands.rs:436-444`, `contract_live.rs:2756-2759`) were aspirational from 8873863. Every site still gates on `may_view_account`, so nothing is open.

*E5b*
- **E5b-C30** `property.manage` covers show/list and org-inspection standing (`hooks.rs:170-173`) but no guest-policy path — `+guest` resolves only the actor's tenancies (`commands.rs:1495-1522`); "guest policy" is doc language (`module.rs:81-82`; 08d:648-650).
- **E5b-C98** `tier=6..255` parses and is clamped by `PropertyTier::new` (`record.rs:76-82`) instead of refused (`commands.rs:492-496`; `property_tier` at `:738-745` has the guard `create` lacks); none of the six create refusals is tested by message.
- **E5b-C161** `property_rooms_refuses_a_thoroughfare` (`tests/e5b.rs:629-649`) adds a room the rig already holds, hits the conflict branch (`commands.rs:597-607`) first, and its disjunctive assert passes on it; the thoroughfare refusal (`:640-656`) is exercised by nothing.
- **E5b-C171** §4.1 records eleven deviations (08d:606-651), not ten; item 11 has been there since b1b4254 and 08d:1085 cross-references it. [Doc-only.]

*E5c*
- **E5c-C2** the default (`off` + `housing=true`) still creates a `chronicle.period` Manual cadence at boot (`economy/boot.rs:49-55`, `:282`) — deliberate (`boot.rs:254-260`; `mush-config/src/config.rs:399-401`) but `boot.rs:4` and 08d:1030-1034 say "creates nothing"; `off_creates_nothing_and_shows_nothing` asserts nothing about cadences.
- **E5c-C90** a lapse period bumps `revision` twice (`prompt.rs:237` and `evaluate.rs:180`); nothing reads it. [Cosmetic.]
- **E5c-C96** the `lapsed.is_some()` guard (`evaluate.rs:313-315`) sits below the housing-pressure (`:274-293`) and strain-relief (`:295-306`) branches, which read the slot `expire` already cleared, so a `HousingPressure` or `StrainRelief` prompt re-opens in the period it lapsed (walked unasserted by `strain_is_bounded_and_asks_for_relief_at_the_ceiling`); deviation 8 (08d:1057-1058) is unqualified.
- **E5c-C116** one `?` at `evaluate.rs:224` (an `Option` short-circuit) against "no `?` at all" (`evaluate.rs:42-43`; 08d:1002-1004). Cannot panic. [Doc-only.]
- **E5c-C231** `the_wod_config_extends_the_reserved_room_list` (`tests/e5c.rs:310-344`) refuses Limbo, which `reserved()` short-circuits before the list is read (`mush-housing/src/commands.rs:674-676`); the rig sets `reserved_rooms: Vec::new()` (`e5c.rs:78`); the `[wod.economy].reserved_rooms` → `config_ext.rs:121` → `modules.rs:118-125` → `with_reserved_rooms` (`:225`) chain is pinned by nothing.

**Refuted (27)** — the tree is right, the claim was not: E1-C134 (the composed property is pinned at `mush-core/src/world.rs:1708`, `:2019`, `mush-org/tests/e1b.rs:148`, `:273`; the cited test pins scrub precision and its comment at `e1b.rs:326` is wrong); E2-C10 (raw `minor()` prints are `to_decimal_string(0)` byte-for-byte — but see the scale-consistency residual in §3), E2-C71 (`meta` grew fields; the claim never said "only"), E2-C131 ("zero projection *until* E4's holds" is exact); E3-C5 (1695 was the fail-fast gate's printout, d8d17c3; 08a:52-55 carries the `--no-fail-fast` 2012); E4a-C164/C166/C167/C173 (E4b2 and its fold refined collateral liveness, `in_flight` refund, the extended refund bundle and the unconditional pledge `Release`, all recorded in 08c §5.1 with `vf_` tests; stale sentences remain at 08c:406-407, :491 and `contract.rs:2226`); E4b2-C21 (B1's `finished_transition` precedes quarantine, 08c:1108-1118); E4c-C4, C39, C56, C119, C132, C134 (fold-recorded refinements, cosmetic omissions, or a miscount — 8 + "recorded, not built" = nine); E5a-C28, C53, C83, C156, C177 (glosses, cut credit, a prose ordinal, a sibling test, and "ahead of the E4-gated E5 chain"); E5b-C170, C179 (scoped by the rig's own doc; no landlord guest verb was ever specified); E5c-C78, C141, C184, C195 (no-mechanics not no-display; a third exit reachable only after a failed boot cadence; a staff view; the sentence's subject is the tick).

## 3. Confirmed gaps, ranked for E6/E7

Severity: **M** matters before E6/E7 builds on it; **L** small and contained; **D** documentation only. Class order: money, consent, disclosure, durability, scope.

| # | Gap | Sev | Class | Smallest fix | Owner · cut |
|---|---|---|---|---|---|
| 1 | E2-C18 `register_instrument` upsert; `Suspended`/`Retired` unreachable | M | money | `register_instrument` refuses an existing id (`EconomyError::InstrumentExists`); keep the command refusal; one engine-level `InactiveInstrument` test through a KV-patched status | economy · **before E6a registers nuyen** |
| 2 | E3-C36 `Title` re-issue never consults `frozen` | M | money | in the Title arm, refuse `AssetFrozen` when the prior record is frozen (staff `@asset/unfreeze` first); test: frozen-then-reissue | economy · before E6c (titles every purchase) |
| 3 | E1-C122 + E1-D23 org lifecycle: no `SetOrgStatus`/`PutRole`, no close workflow, `OrgMembership` has no runtime writer | M | consent | the DL-06 cut: `OrgMutationOp::{SetStatus, PutRole}` + a `CreateScope` revision bump; `@org/status`; a `PutMembership` producer (`@org/member`) so role templates and `from_role` approvals are reachable | mush-org · before E6c |
| 4 | E1-C126 staff freeze/unfreeze/override acts journaled nowhere | M | disclosure | route `@econ/unfreeze`, `@account/unfreeze`, `@asset/unfreeze`, `@vault/freeze\|unfreeze`, `@contract/resolve` through `mush_org::audit` with `economy.freeze.*` / `economy.override.*` tokens (seam exists: `api.rs:145-165`) | economy · E6a |
| 5 | E4c-C110 three masking renderers + two inline copies | M | disclosure | one `render_account(ctx, id, noun)` used by history, expiry and deliverable renders; keep the history collapse token as a second helper over the same predicate | economy · E6b, with 08f's D5 handoff |
| 6 | E5c-C96 lapsed prompt re-opened in the same period | M | scope | owner picks: hoist the guard above `evaluate.rs:274` (one period of quiet; a standing condition re-asks next period) **or** scope deviation 8 to the budgeted kinds; pin either with a test | ruleset-wod · §12 |
| 7 | E4b1-C62 `Suspension` forgiveness tells only the staffer | L | disclosure | `notify_both` from `@obligation/resume` with `cause_word(Suspension)` | economy · E6a |
| 8 | E1-C129 deny-always-wins; unconsumed `AuthWitness` | L | consent | doc: add §5.5 "As shipped" (explicit-deny-wins, live re-run per R-D4); delete `AuthWitness` or keep it documented as a hint | mush-org · docs |
| 9 | E1-C127 D2 promotion half struck without an as-built note | D | consent | D2 "As built" pointing at 08a decision 5 / D-E3-11 | docs |
| 10 | E4b2-C193 wildcard `ContractState` arms | L | durability | name the terminal variants at the four sites | economy · E6a |
| 11 | E2-C159 `@econ/mint` source not structurally system | L | money | resolve `<sysacct>` through the registered system accounts (or check `owner == System`); fix `api.rs:651-655` | economy · E6a |
| 12 | E4b2-C55 amendment cap unchecked on boot repair | L | money | `min(amendments+1, MAX)`-style guard is wrong — instead page when the cap is hit and skip the bump; loop-to-cap test | economy · during |
| 13 | E4a-C114 forgiveness emits no `ObligationChanged` | D | durability | amend D-E4-14: disclosure is the two party notices; the event fires on a kind change only | docs |
| 14 | E1-C135, E4b1-C189, E5b-C161, E4b2-C232, E3-C123, E4a-C210(14e) tests that do not pin what their names say | L | durability | one test each: stale-meta org floor; a store-built Manual hold through `expire_holds`; a reachable non-anchor thoroughfare room (drop the disjunction); a two-member audience permuted; a 21-row `@account/list`; a `Place`-on-higher-id sort | owning crate · during |
| 15 | E5c-C231 config → reserved-room chain untested | L | durability | a `mush-server` test building `server_modules_with(Some(wod))` and asserting `with_reserved_rooms` received the list; rename the wod test to what it pins | server + ruleset-wod · **before Shadowrun copies the plumbing** |
| 16 | E5b-C98 tier clamp; create refusals untested | L | scope | `.filter(\|n\| *n <= MAX_LEVEL)` as `property_tier` does; six refusal tests by message | housing · during |
| 17 | E5b-C30 `property.manage` "guest policy" clause | D | scope | strike the clause (08d:648-650, `module.rs:81-82`, `commands.rs:9,116,242,895`) or add a landlord guest verb — the former | docs |
| 18 | E2-C16 `InstrumentDisplay` never built, never dropped | L | scope | owner decision (§12): `display: Option<InstrumentDisplay>` (serde default) for nuyen, or strike from 08 §6.1 | economy · §12 |
| 19 | E5c-C2 off-config cadence creation | D | scope | reword `boot.rs:4` and 08d deviation 1 ("one inert Manual clock is created so housing can assess"); pin with a cadence-count assertion | ruleset-wod · docs |
| 20 | E1-C124, E2-C70, E2-C160, E4b1-C47, E4c-C109, E5b-C171, E5c-C90, E5c-C116 | D | — | one doc/comment batch: add `policy:{id}`; note the prepare/install split; drop "near-i64"; "a duplicate is absorbed"; fix the `:2080` comment; "eleven"; delete `prompt.rs:237`; "one `Option` `?`" | docs · with §11 item 11 |

**Residuals found while refuting** (real, not gaps in the claims): five player-visible renders print raw minor units regardless of `scale` — `mush-economy/src/commands.rs:2353`, `contract_live.rs:645`, `:3009`, `sweep.rs:732`, `mush-housing/src/commands.rs:815` — against `parse_amount`'s own rule (`commands.rs:1660-1664`); no test drives a plain-collateral contract through *settlement* asserting the `Collateral` claim came off via `settlement_plan` (E4a-C164); `@econ/audit` does not print `observer_depth()` (E4c-C56); `put_residence(ctx, who, None)` should be hoisted above the cadence exit (`chargen.rs:254-268`, E5c-C141); stale lines at 08c:406-407, :491, `contract.rs:2226`, `e1b.rs:326`, `economy/mod.rs:86`, 08d:1187-1188.

## 4. Dropped or narrowed features

| Feature | Ids | Status | E6/E7 | Note |
|---|---|---|---|---|
| Faction→org membership mirroring; the membership adapter | E1-D4 D5 D6, E3-D11 | struck | safe | Faction authority stays faction-side (R-D1); consequence in §3 #3. 08f's parties are their own subsystem (08f:146, :261), so E6a does not need org membership |
| Player-created organizations | E1-D7, E3-D10, E4a-D12 | deferred, unassigned | owner decision | needed only if E6 mints per-team or per-fence orgs at a player's request |
| Org close workflow, `SetOrgStatus`, `PutRole`, revision bump | E1-D13 D14 D17, E3-D20 D21 | never built | **critical for E6c** | DL-06; the `created_rev` stamp (`mush-org/src/commands.rs:441-448`) is the only revision movement |
| `@org/role`, `@org/delegate`, revoke verbs; `aliases`/`charter`/`ruleset_ext` surface | E1-D8 D22 D23 | never built | safe | six of the ten ops reachable only through `submit` |
| `PolicyRef` | E2-D11, E3-D9 | never | safe | leaf scope + `AccessPolicy` (D-E3-5) |
| Instrument `display`; engine enforcement of `negative_policy`/`transferable`; `@instrument/register` shape | E2-C16, E2-D25 D26 D27 | narrowed | E6a display (§12); `transferable:false` matters only when E7 requisitions/entitlements arrive | `record.rs:74-78` calls the instrument stance advisory |
| Approvals reserve nothing; no budget primitive; sub-threshold drip | E3-D5 D59, E4b1-D22 | deferred to "E6/E8" | **critical for E6a** | DL-10; no plan names it |
| Consent model: roster lock at first acceptance; amendment voids every acceptance; per-party binding | E4b2-D16 D17 D7 | narrowed | **critical for E6a** | DL-01 |
| Adjudicator replaces the condition's attestor set | E4b2-D18, E4c-D23 | narrowed | safe | stale comment `contract.rs:466-476` |
| Offered lapse always refunds; live cancel staff-only; attest while Disputed | E4b2-D15 D21 D22 | narrowed | safe | |
| `ExpenseTreatment` declarative | E4b2-D20 D8 | deferred | E6a consumer, safe | 08f keeps it declarative |
| One custody effect per plan (`MAX_DEPOSITS` 1, module cap 1, no two-phase provider) | E4b2-D27, E3-D17 D60 | limit | **design constraint for E7** | DL-15: one object per contract or a container |
| Org/system depositor gets no automatic return | E4b2-D28 | limit | **critical for E6c** | a vendor org's deposit cannot be handed back by `refund_plan`; DL-03's seam must cover it |
| `DisclosureLevel` struct | E4a-D29, E4b1-D43, E4c-D40 | never | safe | `Audience` covers boards |
| LLM/prose settlement conditions | E4a-D7, E4c-D39 | never | safe by design | |
| Per-obligation scheduler; per-debtor durable gate | E4a-D9 D10, E4b1-D37 | never | safe | one bounded sweep |
| `EconomicClock::Registered` | E4a-D1, E4b1-D25 | deferred | D-LBK-7 owns | C3 |
| Per-account/asset freeze isolation | E2-D4, E3-D14 D52 | deferred | owner decision | DL-12 |
| Durability checkpoint; cold paging; consumed approvals never GC'd; RAM tx index | E2-D1 D2 D28 D40, E3-D24 D40, E4a-D11 | deferred | owner decision | C17 |
| Dependents (retainers, ghouls, Herd) | E5a-D35 D36, E5c-D4 | deferred | owner decision | C14 |
| Delinquency access restriction dropped; Pending/Ended admit nobody; Strained counts as covered | E5a-D4 D5, E5b-D41 | narrowed | safe | |
| Housing pulls the cadence; scene asked via `@tenancy/resolve` | E5b-D18 D13 | narrowed | safe | |
| Hidden org / hidden membership visibility | E1-D18 D19 D29 | never exercised | **critical for E6b** | fence and vendor listings are the first non-staff render of organizations; doc 08 §17.2 scenario 12 has no test |
| Careers without a front are quiet; `Advance` no canon; `Ambient` no prompts; `+resources` refuses third parties | E5c-D27 D28 D29 D44 | narrowed | safe | |
| Officer tenancy creation | E5a-D38, E5b-D40, E5c-D35 | staff-only in E5 | owner decision post-E5 | |
| `check_move_attempt` cannot see the issuing actor | E5a-D32, E5b-D20 | core limit | owner decision | a core change |
| Ruleset `FactProvider` registration | E4c-D1 | **dropped by E5** | E7 takes it | only `JobsFactProvider` is wired (`modules.rs:182`) |
| `Property.upkeep` parsed, rendered, never charged | E5b-D34 | dead field | safe | charge in exact mode or drop |

## 5. The handoff chain

| Handoff | Raised by → owner now | Status |
|---|---|---|
| Notify an org/system principal (sweep and contract paths reach `PrincipalId::Object` only) | E4b1-D17 → E4b2-D3 → E4c-D11 → E5b-D6 → **DL-04** (server observer) | **OPEN, passed four times** |
| Staff-channel escalation fan-out; no `EscalationOpened` event | E4b1-D5 → E4c → **DL-05** (economy + server) | **OPEN, dropped by E4c** |
| Budget primitive / sub-threshold drip | E3-D59 → E4b1-D22 → "E6/E8" → **DL-10** (owner) | **OPEN, unclaimed by 08f** |
| Per-party acceptance binding | E4b2-D7 D17 → E6a/E6b | OPEN (08f §1.1 defers as substitution only; DL-01) |
| Audience notice render for run boards; `ExpenseTreatment` consumer; party subsystem crate | E4b2-D6 D8 D9 → E6a | OPEN (08f:261 has the party) |
| `scene.outcome` producer; canon outcome kinds table filled | E4c-D5 D6 D7 D36 → E6b | OPEN |
| E7 `FactProvider`s from applied markers, `None` never `Some(false)` | E4c-D8 D9 D10 → 10b | OPEN |
| Ruleset `FactProvider` beyond jobs | E4c-D1 → E5 | **DROPPED**; re-target to E7 |
| Owner abandonment | E4b2-D11..D14 → 10b `@asset/abandon` | PICKED UP |
| Operator-eviction verb for a new title owner | E3-D16 → E7 | OPEN (10b:233-235) |
| `ClockRef` alias / `ClockProvider` | E4a-D1, E4b1-D26, E4c-D16 → D-LBK-7 | SUPERSEDED (C3) |
| `EconomicEvent` → `SemanticEvent` | E4b1-D27, E4c-D15 → K (C6) | OPEN, rides the family migration |
| State families for E4/E5 records | E4a-D40, E4b1-D28 D29, E4b2-D39, E4c-D13, E5a-D10, E5b-D19 D42, E5c-D10 → K review | OPEN; order fixed by 08h Q5 (property, tenancy, then profiles, then obligations/contracts) |
| `post` as a kernel action provider | E4a-D58, E4b1-D33, E4b2-D40, E4c-D14 → owner/K | OPEN |
| Dropped-batch hazard | E4a-D41, E4b1-D30, E4c-D12 → K | CLOSED for durability (C9); residual on the K6 agenda (08h §3) |
| `CausalRef` alias, one release | E4a-D56 → economy | **OVERDUE** (C5) |
| `fmt_ago` helper move | E4c-D4 → E5 | dropped, cosmetic |
| `@econ/cadence`; `Delinquent` hook; title-issuer entry | E4b1-D18 D19 D20, E4c-D2 D3 → E5 | CLOSED (b1b4254; 816efdf) |
| B6/S2 tests against real cadence clocks | E4b1-D21 → E5 | not recorded in 08d as done — verify or add |
| Escrow-hold command gate; `authorize_attestor` | E4b1-D16 D40 D41 → E4b2 | CLOSED (e56c2a9) |
| E5a → E5b (one live tenancy per property, guest subjects are characters, re-home after unhome, `remove_tenant` at boot, resolve window job/ack, `assess_at` on tick, `may_enter_with` via `authorize_in_world`, window driven after `advance`) | E5a-D14..D24 → E5b | CLOSED (`mush-housing/src/module.rs:248-273`; `commands.rs:1627` "Only a character can be a guest."; `flow.rs:157-200` re-home, fold H3; `hooks.rs`) |
| Re-check `WINDOW_ESCALATED_RENOTIFY` when the clock is registered | E5a-D2 → E5b | **not recorded** — still 7 periods (`tenancy.rs:77`); document the cadence-length assumption in `@tenancy/create cadence=` help |
| E5b → E5c (`submit_support_plan`, restate every period, reserved rooms, housing knob, chargen via `create_tenancy`, assessment word, `put_tenancy` writers) | E5b-D1..D9 → E5c | CLOSED (`evaluate.rs:238-243`; `modules.rs:118-125`, untested per E5c-C231; D6 → DL-04) |
| Off-screen ops pool | E5c-D1 → faction simulation | unclaimed (DL-17) |
| `FrontRef` → C6 chain; `wod.career.exposure` not an attestor | E5c-D2 D3 → K / E6b | OPEN |
| Migrate pattern; boot-reconciliation pattern (self-identifying records); viewer-aware render; partial-switch table; deadline from the ordinal | E5c-D5..D9 → E6 | **08f cites none** — fold into DL-02 |
| Faction→Org linkage; capability vocabulary; approvals; org status inert; approval → org audit | E1-D2 D1 D9 D12 D33 | CLOSED (E3d, E3b2, E3b2, E3b2, E4c) |
| `AuthWitness` consumer | E1-D11 | dropped by design (live re-run, R-D4) |
| System-principal registry, metadata, telemetry | E1-D27 D28, E2-D12 → C15 | OPEN |
| Doc amendments owed: `policy:{id}` (E1-D24); stale faction comments (E3-D47, verified present at `mush-faction/src/econ.rs:21-27`, `data.rs:277-280`); vault-revision wording (E3-D49); "four verbs" (E4b2-D24); `contract_live.rs:18-21` module doc (E4b2-D31, verified present); S9 escalation half (E5a-D28) | → docs | OPEN, batched in §11 item 11 |

## 6. Deferred findings and documented limits

| Id(s) | Limit | Still true? | Risk for E6/E7 | Disposition |
|---|---|---|---|---|
| E2-D24 | `EconomicCause::Command { serial: 0 }` hard-coded | yes — five sites (`commands.rs:635, 1774, 2782, 2830, 2869`) | low; command-caused entries uncorrelatable to dispatch | fix-during: plumb `ctx` serial or drop the field |
| E2-D26 | `negative_policy`/`transferable` recorded, never enforced | yes | E7 requisitions | accept until a non-transferable instrument exists |
| E2-D30 | no Redis-backed economy journal suite | yes (mushd persistence is Redis-gated; economy is not) | medium at convergence | fix-during: one Redis-gated economy reboot test |
| E2-D38 | journal contiguity hole fails boot, no in-game repair | yes (`boot.rs:87-88`) | accepted (money) | accept; DL-13 bridge pages the gap |
| E2-D45, E2-D49 | no per-account freeze reason; no per-entry zero-sum on replay | partly (escalation records carry boot diagnoses since E4b1) | low | accept |
| E3-D22, E3-D23 | inert `Unplaced` doc after interior withdrawal; `CustodyRef::Registered` has no provider | yes | low / E7 | accept / C13 |
| E3-D35 D36 | grown firing set unpostable until TTL; staff cannot credit a Suspended org | yes | moot until DL-06 | accept |
| E3-D40 | Consumed approvals never GC'd | yes (`boot.rs:686-694`) | growth | with C17's threshold |
| E3-D53 | approvals pull-only; nobody paged | yes | E6 org payers | DL-04 |
| E3-D56 D57 | `backing_valid` `None` leaves stale Active vaults; boot custody depends on module order | yes | low | accept; note in 10b for facility vaults |
| E4a-D39 | `Hold.holder` rule enforced by doc + test only | yes | low | accept; DL-16 wording |
| E4b1-D2 D3 D4 D8 D9 | frozen sweep does not stamp; one record per subject; `@econ/ack` never unfreezes; concrete syntax; hold ids from 1 | yes | none | accept |
| E4b1-D23 | S7 increment path unit-only | yes | none | accept |
| E4b1-D34..D39 | `question_mark` allow; no record when the escalation allocator is exhausted; unstamped terminal holds never GC'd; no durable gate from the sweep; frozen-account holds age out on unfreeze; `MAX_BOOT_ESCALATIONS` 16 | yes | low | accept |
| E4b1-D44 | `catchup=` clamps where `grace=` refuses | yes | low | fix-during with E5b-C98 |
| E4b2-D26 | `Fact` conditions untested at E4b2 | closed by E4c (`tests/e4c_facts.rs`, `e4c.rs` fact paths) | — | closed |
| E4b2-D30 | `escrow` list trusted only through `qualifying_escrow` | yes | none | accept |
| E4b2-D32 D33 | boot pages capped; **allocator exhaustion at activation leaves an Active lease with no rent and no page** | yes (`contract_live.rs:1596-1601`) | low | fix-during: page it |
| E4b2-D34..D38 | predicted hold ids healed; amend refused not queued; collapsed refusal text; `LIST_CAP` no paging; sweep notices never claim durability | yes | none | accept; D-LBK-5 adds cursor pages housing-side only |
| E4c-D28 D29 D45 | canon evidence replay is a 64-record RAM tail | yes | **E6b/E7** | DL-09 |
| E4c-D33 D34 D35 | 8 settlements/event; 8 republishes/sweep; events dropped past depth 4 | yes | none | accept |
| E4c-D37 | tail re-scan cannot apply the party gate | yes | low | with DL-09 |
| E4c-D41 | disclosure filter covers only E4c's pages | yes | **every E6 page** | rule: new pages render through §3 #5's helper |
| E4c-D42 | `SettlementRefused` reused for dropped-event and stale pages | yes | low | fix-during: two kinds |
| E4c-D47 | delinquency chronicle line keyed on `missed_total` | yes | none | accept |
| E5a-D30, E5a-D46 | period rules meaningful only on ≥ daily cadences; pre-field plans read as period 0 | yes | low | help text; accept |
| E5b-D21 | a person can be teleported out of an occupied home | yes | none | accept |
| E5b-D23..D33, D43 | `MAX_BOOT_PAGES` 16; end sweep depth 6 / 4096; lowest-id wins on two live tenancies; clockless tenancy frozen; last-tenant-gone pages; corrupt `assess:`/`notice:` non-fatal; notice bounds; best-effort `set_home`; window unchanged on mint failure; refused title consumes a property id | yes | low; D-LBK-1 replaces the lowest-id rule with a `Conflict` quarantine | accept (D-LBK-1 supersedes D25) |
| E5c-D15 D16 | over-cap income rows and stakes warned, marked, never refused | yes | none | accept |
| E5c-D21 D22 D23 | hand-cancelled income never re-opened; failed boot cancel keeps minting with a log; failed `stop_income` retried by a second migrate | yes | low | accept |
| E5c-D24 D54 | a skipped profile loses strain and the open prompt | yes | low | D-LBK-3 tightens the load |
| E5c-D25 D26 | boot-created clock is Manual/1; no cadence freezes evaluation | yes | staff | accept; E5c-C2 reword |
| E5c-D31 D32 | orphaned front stops generating; phantom `domain=` org id accepted | yes | none | accept |
| E5c-D55 | 128 characters per fire from a cursor | superseded by D-LBK-2 (256 visited / 128 worked) | — | fix-before (D-LBK-2) |
| E5c-D58 D59 | `set_system_obligation` moves amount/status only; `CadenceError` outside `EconomyError` | yes | none | accept |

## 7. Standing tech debt

| Id | Item | Priority |
|---|---|---|
| T1 | **rustfmt: 2,467 hunks in 88 files** (`cargo fmt --all --check`, rustfmt 1.9.0, no `rustfmt.toml`): mush-economy 1602, mush-housing 360, ruleset-wod 239, mush-org 104; 162 in K-shared crates. Up from 1334 at d7f6707 (08a:55-56). Style-only. 08h §3 schedules the single semantic-free reformat after the K6 merge, both sides rebasing once. | **done** `91208ab` (rustfmt.toml `edition = "2024"`, `newline_style = "Unix"`; CI Format blocking again; gates identical before/after) |
| T1b | CI's Format step (`.github/workflows/ci.yml:29-30`) precedes Clippy/Build/Test in one job with no `continue-on-error`; every push since the tree went fmt-red has produced no clippy or test signal, and the Linux-only Redis proofs (`:37-41`) and TS drift check never run. | **fix-before** (XS) |
| T2 | CRLF: 26 files `i/lf w/crlf` (`git ls-files --eol`), index and HEAD clean (`.gitattributes` `* text=auto eol=lf`); a text-mode tool on Windows wrote them after checkout. Harmless to git/cargo; rustfmt's `Auto` would preserve CR. | **done** with `91208ab` (the Rust files rewritten LF; one doc file left, normalized by git on commit) |
| T3 | Help: no prose topic in `docs/help/` covers any of the 16 E-series commands (`mush-server/src/help.rs:30-63` loads 30 topics); inline `help:` blocks are complete (`help @vault` works); 816efdf's "help topics" meant inline help. | fix-during (S): `economy.md`, `housing.md`, `org.md`, a "Careers and money" section in `wod.md`; add stems to the pin at `help.rs:91` |
| T4 | Flake: `mushd::persistence::store_failure_enters_read_only_and_recovery_exits` failed once in two workspace runs (60 s `expect_text` timeout, `persistence.rs:56-72`, `:1613-1660`; FlakyStore backoff under ~180 sibling binaries), 0 of 4 isolated; fail-fast stopped the run at 125/184 suites. `web_e2e` is clean in 6 runs; the documented failure was fixed in E4 (`web_e2e.rs:898`). | **fix-before**: `--no-fail-fast` on `ci.yml:36` and the gate command; fix-during: serialize the three FlakyStore tests, make `expect_text` name the needle |
| T5 | TODO/FIXME/todo!/dbg!: 3 hits, none live; `ruleset-wod/src/splats/kinfolk.rs:45-58` carries a stale "Integration TODO" for wiring that landed (`werewolf.rs:712 require_garou_or_kin`). | accept; rewrite the comment with any ruleset-wod commit |
| T6 | Since E1: 29 `#[allow]` (6 E-side, 23 K/M16-side), 3 `#[ignore]`. E-side to fix: dead `try_boot` (`mush-economy/tests/e3b.rs:104-115`), unread `Settled::Posted(TransactionId)` payload (`contract_live.rs:1775`). K-side: `mush-server/src/timeline.rs:236 redis_round_trip_preserves_exact_generations` is `#[ignore]` and named nowhere in `ci.yml`, so it runs nowhere. | fix-during (XS each; the timeline test is K's) |
| T7 | Panics on hot paths: tick/hook/observer files contain none (sweep, observer, contract_live; housing tick/hooks/boot/flow; wod evaluate/boot/mod/migrate). Deliberate invariant panics: `install_commit` (`api.rs:1965-1978`, reachable from the sweep via `post`) and the `with_facts` namespace assert (`module.rs:619`); boot `expect`s hold by construction; no `catch_unwind` anywhere — the designed fail-closed response. | accept; optionally `tracing::error!` before the `install_commit` panics |
| T8 | 08a (the E6 handoff) carries "(uncommitted)" for the E4c fold (fa6a979) and E5c (816efdf, fold ba75fc3), "40 tests"/"+40" (now 60), "923 passing"/"2735 across 184 suites" (post-fold 943/2755), mush-server 118 (154 post-K-merge), no hashes for E5a/E5b (6b606e5/7155efc, b1b4254/d9f9ceb), and "121 tests" for E5b (crate total; `e5b.rs` is 66). Every per-file count in 08b/08c/08d matches the tree (§9). | **fix-before** (S, docs) |
| — | Doc drift called out elsewhere: C1 (doc 08 as-shipped notes stop at E3), E3-D47, E4b2-D31, E5a-D28, E4c-D17 (stale conformance row), E2-D24's api doc. | §11 item 11 |

## 8. Doc 08 vs shipped

Doc 08 declares that an as-shipped note overrides surrounding prose (08:8), so a stale note is actively wrong for an E6 builder. Section map at `ba75fc3`:

| Doc 08 section | Note says | Tree has | Action |
|---|---|---|---|
| header (:3-8) | E0–E3 shipped | E0–E5 shipped (08a:42-50) | C1 |
| §4.3 (:184) | holds/obligations/contracts are E4+ | built (`record.rs:818-889`; `obligation.rs:228-268`; `contract.rs:639-662`) | C1 |
| §5.2 (:305-310) | closing is a workflow | no op or verb (E1-C122) | C2 / DL-06; interim note "staff by store edit" |
| §5.5 (:433-438) | explicit entries beat broad rules; every evaluation revision-stamped | deny always wins; live re-run (E1-C129) | add "As shipped" |
| §6.1 (:531-542) | `display: InstrumentDisplay` | absent (E2-C16) | §12 decision, then note |
| §6.2 (:587-591) | `held_amount` zero until E4 | journal-derived since E4b1 (`engine.rs:1492-1496`) | C1 |
| §9.3 (:969-970) | `CustodyRef::Registered` | no provider (C13) | §12 decision |
| §11.1, §18 E5 (:1251-1253, :1774-1779) | retainer/ghoul payroll | deferred with dependents (C14) | §12 decision |
| §12.2 (:1349-1355) | visibility recorded, never enforced; filter is E4 | `Audience` enforced through `mask_account` and `ShowClass` | C1 |
| §12.3 (:1357-1372) | mush-org journals freezes/overrides; rejected attempts as telemetry | not journaled (E1-C126); no telemetry (C16) | §3 #4; C16; note |
| §14.1 (:1431-1451) | system-principal metadata, flow telemetry | mint cap + issuer allow-list only (C15) | C15 |
| §15.1 (:1483-1527) | mush-org list without `policy:{id}`; hold/obligation/contract docs E4+ | `policy:{id}` written (E1-C124); `hold:`/`obligation:`/`contract:`/`cadence:`/`escalation:` exist | C1 |
| §15.3 (:1563-1568) | tenancy gate is E5 | `HousingDestroyGate` live (`hooks.rs:189-191`) | C1 |
| §16 (:1597-1610) | `@obligation`/`@contract` not built; `@org/role\|delegate` API-only | first two built; the org verbs still API-only (E1-D8) | C1 |
| §18 (:1759-1779) | E4 NEXT; E5 unstatused | E4/E5 SHIPPED; E6 → 08f; E7 → 10b | C1 |
| §19 D2 (:1825-1830) | promote membership | struck (E1-C127) | "As built" |
| §7.2/§7.5 checkpoint (:640-643, :804-814) | boot from checkpoint + tail | full replay (C17) | keep deferred; add the trigger |

Smaller shape deviations doc 08 never records (C1): `ContractState` folds `Accepted` into `Active` (D-E4-10); `Obligation` has no `SettlementPolicy`; `EconomicSchedule` gained `manual_now`; `posted_at` is a wall-clock `i64`. Planned-not-shipped sections with a named plan and no contradiction (C18): §9.4 catalogs/quotes, §11.2 run/pay/fence, §10 work orders and production, §10.3/§14.2 markets, §19 D5's operator eviction — E6 in 08f, E7 in 10b, E8 unplanned.

## 9. Test-claim reconciliation

- Per-file counts in 08b/08c/08d all match the tree and pass: e1a 23, e1b 20; e2a 12, e2b 11, e3_verify 12, e3a 46, e3b 35 (31 + 4 fold), e3b2 33, e4a 83 (66 + 17), e4b1 61 (33 + 28), e4b2 59 (37 + 22), e4c 35 / e4c_bridge 11 / e4c_facts 3 (18/4/2 + 17/7/1); custody.rs 10; faction economy.rs 21; e5a 54, e5b 66; e5c 60 (40 + 20). Crate gate 265 + 121 + 403 + 154 = 943; workspace `--no-fail-fast` 184 suites / 2,755 passed / 0 failed / 47 ignored (45 redis_suite + 2 mush-server lib). Matches 08d:925, :1269-1270.
- Mismatches, all in 08a (T8): three "(uncommitted)" cells; "40 tests" / "+40"; "923" / "2735"; mush-server "118"; missing E5a/E5b hashes; "121 tests" on the E5b row.
- Historical figures that are right for their commit and need no change: 08b:564 "E1's 34 tests" (46 = 34 + 12 from E3b2), 08b:898 "1695" (the fail-fast gate at d8d17c3; 08a:52-55 carries 2012). Discrepancies already in the record: E3-D48 (286c4d6 reports faction 58 / workspace 2014 versus 08a's 56 / 2012), E4c-D48 (08a "25 tests" versus fa6a979 "26 findings": E11/E12 doc-only, E3 carries two tests), E5b-C171 ("ten" versus eleven), E4b2-D24 ("four verbs" versus six).
- Tests that do not pin what their name or claim says: E1-C134 (mis-sourced; property pinned elsewhere), E1-C135, E4b1-C189, E5b-C161, E5c-C231, E4b2-C232, E3-C123 (no `*/list` cap test), E4a-C210 (14e), E4b2-C55 (no cap test), E5b-C98 (create refusals), E4c-C109 (empty history). Fixes in §3 #14-#16.
- E4b2-D42: `staff_resolve_settles_a_dispute_and_still_re_verifies` (`tests/e4b2.rs:895-902`) was rewritten by the E4c fold and now pins E4c's staff-override behaviour; attribute by reading e56c2a9 for E4b2-as-built (E4b2-D41).

## 10. K integration

08c §12's conformance table, row by row at `ba75fc3` (with K3/K4a/K5a merged):

| 08c §12 row | Claimed | Actual | Verdict |
|---|---|---|---|
| C8 `ReservationRecord` / idempotency keys | states `Held\|Consumed\|Released\|Expired`, `holder`; deterministic keys | as built (`record.rs:818-837`, `:868-880`; keys at `obligation.rs:298`, `sweep.rs:216`, `contract.rs:1048`) — C8 conforms | accurate. **E4c-D17's "rename at build time, not done" row is stale — strike it** |
| §2.5 `EconomicClock` → `ClockRef` alias, sweep as `ClockProvider` | when K4 lands | K4a landed; no `ClockProvider` symbol exists; `EconomicClock` still the four-variant adapter (`clock.rs:54-72`); `CadenceClock` lacks `generation`/`schema` | superseded by D-LBK-7 (08h:58): fields now, declared cadences at convergence, runtime cadences legacy. **Rewrite the row** (C3) |
| adapt to the landed timeline now? | — | `game_now_unix()` = `world_ic_now().at` with the generation dropped; `World` keeps `game_clock` in step across epochs (`world.rs:351-399`), `validate_epoch_transition` refuses discontinuity, a rate change reads as `TimeJump`; only wod stamps a typed `ClockStamp` | no correctness pressure; retype at convergence (C4); add one test: a mid-gap rate change classifies `TimeJump` and charges nothing |
| `CausalRef` → `ExternalRef`; `EconomicEvent` → `SemanticEvent` | rename + one-release alias; migrate when commit tokens exist | rename done; alias past its life with zero users (`record.rs:316-320`, `lib.rs:139-141`); kernel owns `PreparedCausalRef` | delete the alias (C5); `SemanticEvent` rides the family migration |
| §2.6 hash domain | `hash_domain: "economy.plan/v3"` stated in the schema doc | no such string anywhere; digests byte-compatible (`hash.rs:166-174`) | add `PLAN_HASH_DOMAIN` const, never in hashed bytes (C6) |
| C2/K2 migration gates | mixed write sets; `Ctx` family read path; seal in the bundle | gate 1 partial (`ReplayWrite::{ObjectSet, ObjectDelete, ObjectAllocatorSet, ModuleJsonSet, ModuleDelete}` in one bundle, `authoritative.rs:257-279`; no typed ledger-append op); gate 2 open (`StateFamilyHandle` only at registration; reads via `Engine::action_state()`); gate 3 open, precondition enforced (`world_task.rs:3021-3027`) | record per-gate status (C7); migration stays an owner decision; property/tenancy first (08h Q5) |
| dropped-batch hazard | pre-existing, open | closed for durability: complete batches spool to the quarantine on every drop path, unresolved records refuse writable boot, idempotent legacy batches replay (e2a17da, 64479b7; `quarantine.rs:238-289`; tests at `quarantine.rs:2999`, `:3031`); only `StreamAppend` is non-replayable and no E crate produces one. Residual = RAM/output ahead of durable truth before the halt (08h §3) | update 08c §12's paragraph and `persist.rs:1457-1462`'s "will NOT self-heal" message (C9) |
| store checkpoint readback | — | every post-image re-read before the intent is released, both backends (`memory.rs:926-988`; `checkpoint_authoritative.lua:85-113`; `redis.rs:410-412`) | verified (C10) |
| single unresolved intent | — | second reservation refused until the first finalizes, both backends (`memory.rs:699-714`; `reserve_authoritative.lua:95-104`) — the K3 lane is serialized to one action | verified (C11); a stated throughput ceiling for any economy migration |
| `ChannelGrant` union | 08h asks for the enumeration | `mush-sim/src/links.rs:18-74` emits one grant per occupied station; core never unions (`behavior.rs:513-519`, `world.rs:698`); the only production publisher | verified (C12): projections = {links.rs}; consumers = {behavior.rs resolve, world.rs lookup, `@spatial/authority`} |

Also: DL-13 (E0 hazard status on the K6 agenda — closed per C9, residual noted); DL-14 (`Registered{provider}` waits for a consumer; forbid economy schedules on `ClockRef::DomainTick` until the registration verb lands, 10b). 08h §3's order stands: K6 lands → both audits filed (this document; the K6 memo) → bilateral review → merge → full gates → the one rustfmt commit.

## 11. The fold list

08h names this section the single fold list; D-LBK items are merged under their ids with 08h's owner and effort. Owners: economy = `mush-economy`, housing = `mush-housing`, ruleset-wod, server = `mush-server`/CI, jobs, org = `mush-org`, docs, K = the K tree.

**Fix before E6/E7** (ordered; each is a gate for what follows):

1. **T1b + T4 — CI signal.** `[done]` `continue-on-error: true` on the Format step until the reformat lands; `--no-fail-fast` on `ci.yml:36` and the per-cut gate command. server · XS.
2. **D-LBK-1 + D-LBK-4a — boot pass 0.** `[done]` Container invariants (one property per room, ≤ 1 live tenancy per property, every live tenancy names a loaded property), fail-closed quarantine, rent suspended through an economy seam, release only by re-validation, one page per conflict with the shipped repair syntax; room → `Unique|Conflict` and property → `Live|Conflict|Vacant` projections with recomputation-equivalence tests. housing + economy seam · M.
3. **D-LBK-2 — tick budgets.** `[done]` `MAX_VISITED_PER_FIRE` 256 / `MAX_WORKED_PER_FIRE` 128, cursor past the last visited id, due check before clone, `population_version` short-circuit. housing + ruleset-wod · S.
4. **D-LBK-3 — bounded loads.** `[done]` `TryFrom` wires for `SpendingProfile` (+ nested) and `PendingNotice`; oversized/over-cap documents refuse or drop loudly. ruleset-wod + housing · S.
5. **D-LBK-6 — `resolve_plain_job` / `deny_plain_job`** `[done]` with queue identity replace `set_job_status`; housing's two callers switch. jobs · S.
6. **D-LBK-7 (now half) + C3.** `[done — the code half; the 08c §12 row is the doc batch's]` `CadenceClock` gains `generation` (default 1) and `schema` (default 1) with a load-unchanged test; rewrite 08c §12 row 2 to point at D-LBK-7. economy · S.
7. **E2-C18 — `register_instrument` refuses an existing id** `[done]`, plus the engine-level `InactiveInstrument` test, before E6a registers nuyen through the API. economy · XS.
8. **DL-02 — review 08f.** The three passes (adversarial money-safety, code-grounding at HEAD, design-fit vs 08/08a/08c/08d/08e/10), then re-ground §2: a `mush_economy::create_obligation` for `ObligationCause::Ruleset` player debits (debtor a party, never `System`, debit authority checked); the DL-03 deposit seam; a `[shadowrun.economy]` partial-switch table on E5c's `careers=false` precedent; the hidden-org render rule for fence/vendor listings (§4); the E5c-D5..D9 patterns; DL-04/DL-05/DL-10 named. docs + E6 authors · M.
9. **DL-01 — escrow-at-publication vs the roster lock**, decided inside item 8: Option A (a pure payer's acceptance does not lock the roster; first payee acceptance locks) or Option B (`+run/crew accept` as amend → outcome fill → re-offer → payer re-accept in one command, refused-re-accept path tested, board entry carries the escrow hold id). economy · S (A) / M (B).
10. **T8 — 08a ledger refresh** (hashes, counts, 2467 hunks). docs · S.
11. **C1 + the doc-only batch.** E4/E5 as-shipped notes across doc 08 (§8 map); §5.5 "As shipped"; D2 "As built"; `policy:{id}`; the residual and doc-only items from §3 (#8, #9, #13, #17, #19, #20), E3-D47, E4b2-D24, E4b2-D31, E5a-D28, E4c-D17's stale row, 08c:406-407/:491, `contract.rs:2226`, `e1b.rs:326`. docs · S.
12. **E5c-C231 + E5c-C2 — pin the config → wiring chain** `[done]` (`server_modules_with(Some(wod))` → `with_reserved_rooms`) and the off-config cadence count before Shadowrun copies the plumbing; rename the wod test to what it pins. server + ruleset-wod · XS.

**Fix during E6/E7** (owner · effort · when):

- §3 #4 E1-C126 staff freeze/override journaling via `mush_org::audit` — economy · S · E6a.
- DL-04 `OrgInbox` observer (kinds-only line to connected `acts_for` holders; offline notice per the housing pattern) — server · S · E6a.
- DL-05 `EconomicEvent::EscalationOpened` + `broadcast_wizards` fan-out with the ladder's schedule — economy + server · S · E6a.
- DL-06 / §3 #3 org lifecycle cut (`SetStatus`, `PutRole`, revision bump, `@org/status`, a membership producer) — org · S–M · before E6c.
- §3 #2 E3-C36 Title over a frozen written-off asset refuses — economy · XS · before E6c.
- §3 #5 E4c-C110 one masking renderer — economy · S · E6b (08f D5).
- DL-03 contract-scoped `Deposit` authority for a counterparty vault, covering an org depositor's return (E4b2-D28) — economy · S · E6b.
- DL-07 `SrProfile` identity rule + `sr::boot::run` reconciliation — ruleset-shadowrun · S · E6a.
- DL-08 stale-contract age: deadline-aware and in `[economy]` config — economy · XS · before E6c.
- DL-09 persist the canon tail (or a `mush_canon` read seam keyed on `source`) — economy + canon · S · E6b.
- DL-11 buyer-as-issuer for quotes; `with_party_caps` for registered vendor/maker orgs — economy · S · E6c.
- C15 `SystemPrincipalDef {category, module, instruments}` behind the existing registration; per-post faucet/sink counters — economy · S · E6a.
- C16 bounded per-actor refusal telemetry at the error renderer, surfaced via `@econ/alerts` — economy · S · E6b.
- C5 delete the `CausalRef` alias; C6 `PLAN_HASH_DOMAIN`; C9 reword `persist.rs:1457-1462` and 08c §12 — economy / server / docs · XS.
- D-LBK-4b (tenant/guest → tenancies, faction → open-prompt count) — housing + ruleset-wod · M; D-LBK-5 visit-bounded cursor pages — housing · S.
- §3 #7 E4b1-C62; #10 E4b2-C193; #11 E2-C159; #12 E4b2-C55; #16 E5b-C98 (+ E4b1-D44); the §3 residual scale-aware renders (five sites) — economy / housing · XS–S.
- §3 #14 the six pinning tests; E4a-C164's settle-path collateral test; E4b2-D33 page on allocator exhaustion; E4c-D42 two escalation kinds; E2-D24 dispatch serial; E2-D30 one Redis-gated economy reboot test — owning crate · XS each.
- E5b-D36/D37 lift the duplicated helpers (`notice` list, `parse_opts`, existence-hiding renderer) to `pub` in `mush-economy` before Shadowrun makes a third copy — economy · S · E6a.
- T3 help topics; T6 (`try_boot`, `Settled::Posted` payload; the ignored timeline test into `ci.yml:41` — that one is K's); T5 kinfolk comment — docs / economy / K · XS–S.
- E5a-D2 document the cadence-length assumption in `@tenancy/create` help; E4b1-D21 verify or add the real-cadence B6/S2 tests — housing / ruleset-wod · XS.

**Accept** (true, safe, no action beyond this record): T1 timing per 08h §3; T2 (normalize at the reformat); T5; T7; C4, C8, C10, C11, C12, C17 (add the boot-cost log + threshold only), C18; DL-14, DL-15 (strike the TacticsModule line from 08a; note `MAX_DEPOSITS` 1 in 10b), DL-16 (two 08c sentences), DL-17, DL-18; the §6 rows marked accept; E4b1-D2..D9, D23, D34..D39; E4b2-D15..D22, D27, D29..D38; E5a-D4..D9, D30, D46; E5b-D21, D23..D33, D41, D43; E5c-D15..D32, D50..D59.

## 12. Owner decisions needed

1. **DL-01** — Option A (pure-payer acceptance does not lock the roster; a small E4b2 change) or Option B (E6a sequences amend → fill → re-offer → re-accept in one command, no economy change).
2. **DL-10** — ship E6a with `organization_run_offers = false` in the mode table, or add a journal-derived rolling `window { total, period }` budget to `ApprovalRule` as an E6a economy item. E8 is too late for a ruleset whose payers are organizations by design.
3. **E5c-C96** — hoist the lapse guard (one period of quiet before a standing condition re-asks) or scope deviation 8 to the budgeted kinds and let pressure/relief re-ask at once.
4. **E2-C16** — build `InstrumentDisplay` (symbol/prefix, serde-default `Option`) for nuyen, or strike `display` from doc 08 §6.1.
5. **DL-12 / C17** — whether asset-projection drift should freeze that asset and page instead of latching the whole economy; the journal-size threshold at which the checkpoint becomes the next cut's first item (measure at E6a).
6. **C13** — `CustodyRef::Registered`: a provider for E7 facility vaults, or remove the variant (boot can assert no doc carries it) and have `@vault/create` refuse it meanwhile.
7. **C14** — dependents: an obligation-only E5d follow-on, or strike scenario 24 from doc 08 §17/§18 and record the substitution.
8. **Organizations at player request** (E1-D7) if E6 mints per-team or per-fence orgs; and whether landlord officers may ever create tenancies (E5a-D38).
9. **Core change** — let a `MoveAttempt` hook see the issuing actor of a `@teleport` (E5a-D32, E5b-D20), or keep "add them as a guest first".
10. **DL-13** — keep the E0 residual on the K6 agenda as 08h says; as a bridge, have the E0 boot contiguity check open a level-2 escalation with the chunk range instead of a log line.
11. **E5b-D34** — charge `Property.upkeep` in exact mode or drop the field.

### 12.1 Decisions taken (E owner defaults, 2026-09-04)

The owner's standing "defaults accepted" order applies; any of these can be overridden by a word. Items marked *fold* are in the look-back fold; items marked *E6a* are in that cut's first step.

1. **DL-01 → Option A′.** An amendment authored by a party does not void that party's own acceptance (consent by authorship); everything else in E4b2's model stands. With no payee accepted yet, the payer's `+run/crew accept` amends the roster and the payout accounts without voiding the payer's acceptance, so the escrow never moves; the first payee acceptance locks the roster. Economy S, *E6a*; 08f §2 re-grounded on it.
2. **DL-10 → the rolling budget.** `ApprovalRule` gains `window { total, period }`, journal-derived; a post that would take the window over `total` requires approval like an over-threshold post. Organization run offers stay on. Economy S, *E6a*.
3. **E5c-C96 → hoist the guard.** One period of quiet for every prompt kind; a standing condition re-asks next period; the housing ladder pages regardless. *fold*.
4. **E2-C16 → build it.** `InstrumentDef.display: Option<InstrumentDisplay { symbol, position: Prefix|Suffix }>` (serde default) when E6a registers nuyen; doc 08 §6.1 keeps the field. *E6a*.
5. **DL-12 / C17 → keep the latch, measure.** Asset-projection drift keeps freezing the whole economy (money-safe). Boot logs replay entries and elapsed time now; the checkpoint becomes the next cut's first item when a cold replay exceeds 5 s or 1,000,000 entries. *fold* (the log).
6. **C13 → keep `CustodyRef::Registered`** for E7's providers (10b cut 2); `@vault/create` refuses it until a provider registers. E7.
7. **C14 → dependents as an obligation-only E5d after E6**; scenario 24 stays in doc 08 with a deferral note.
8. **E1-D7 / E5a-D38 → stay deferred.** E6 mints no organizations at a player's request (parties are their own subsystem; fences and vendors are staff-built); officer tenancy creation stays staff-only.
9. **Core change → not taken.** A `MoveAttempt` hook still cannot see the issuing actor of a `@teleport`; "add them as a guest first" stands.
10. **DL-13 → structured error, no escalation.** The E0 contiguity failure reports the chunk range and the operator command in its error text; an escalation cannot be opened from a boot that does not run. The residual stays on the K6 agenda per 08h §3. Fix-during, XS.
11. **E5b-D34 → drop `upkeep`.** Ignored on load, removed from renders, `@property/create`, and help. *fold*.

## 13. Fold outcome (the code half, 2026-09-04)

Every item of the fix-before list that is code, folded at the tree this section is filed against; the
doc-only items (8–11) are the parallel doc batch's. Test sections are `// ---- look-back fold: <id> ----`,
test names `lb_<id>_…`. Gate totals are the fold's own run: `cargo test -p mush-housing -p ruleset-wod
-p mush-economy -p mush-jobs -p mush-server` **994 passing, 0 failing** (economy 408, housing 130, jobs 31, server 155 + 2 ignored, ruleset-wod 270); `cargo clippy --workspace --all-targets -- -D
warnings` clean; `cargo test --workspace --no-fail-fast` **2779 passing across 184 suites, 0 failing, 47 ignored**. The fold added **24** tests (housing 9,
ruleset-wod 5, economy 5, jobs 4, server 1); no existing test changed meaning — one (`vf_s5`) was
re-pinned to the new forgiveness rule, one (`the_wod_config_extends_the_reserved_room_list`) renamed to
what it pins.

| # | Item | Outcome |
|---|---|---|
| A1 | T1b + T4 — CI signal | done. `.github/workflows/ci.yml:31` `continue-on-error: true` on the Format step ("until the post-K6 reformat lands"); `:38` `cargo test --workspace --no-fail-fast`. |
| A2 | D-LBK-1 + D-LBK-4a — pass 0, quarantine, rent suspension, projections | done. Housing: `crates/mush-housing/src/quarantine.rs` (`QuarantineKey` `housing.corrupt:room\|tenancy\|orphan:<id>`, `Conflict`, `compute` `:139`, `revalidate` `:236`, `suspend_rent` `:325`); the projections `RoomEntry::{Unique,Conflict}` / `LiveEntry::{Live,Conflict,Vacant}` (`module.rs:208`, `:217`), `rebuild_projections` (`:353`), `projections_consistent` (`:386`), the single write path `put_property`/`put_tenancy` (`:602`, `:617`) with the `#[cfg(debug_assertions)]` equivalence assert and pass 0 re-run after every write; `may_reach` (`hooks.rs:132`) refuses a `Conflict` and every room of a quarantined property for everyone but staff; the destroy gate refuses those rooms (`hooks.rs:231`); the tick skips a quarantined tenancy (`tick.rs:200`); `+home`/`@tenancy/show`/`@property/show` render "(under review)" to members and officers, the hidden text to everyone else (`commands.rs:1404` and its property twin); `@housing/quarantine show\|recheck <key>` (`commands.rs:210`), no release verb. Boot: `boot.rs:191` rebuilds the projections after the per-document loads, `boot.rs:210` runs pass 0 before passes 1–7. Economy seam, game-neutral: `SuspensionKey` (`obligation.rs:107`), `Obligation.suspensions: BTreeSet<SuspensionKey>` (`:432`, serde-default so old documents load; the status reason is the one implicit holder of a pre-fold `Suspended` doc), pure `hold`/`release` (`:720`, `:747`), `suspend_obligation`/`resume_obligation` (`api.rs:718`, `:755`), `SuspendReason::Held { key }`, `ObligationEvent::{Held,Released}`, `EscalationSubject::HousingRoom` (`escalation.rs:111`). The sweep's page-suspension holds under its own key `escalation:<id>` (`sweep.rs:872`), staff's under `staff`; `@econ/ack` releases the ladder's key alone (`commands.rs:3828`), `@obligation/resume` refuses while an alert or a module holds (`commands.rs:3506`), `@obligation/reinstate` refuses a held obligation; **every period that falls due while suspended is forgiven as it falls due** by the sweep, `Forgiven { Suspension }` (`sweep.rs:492`), so nothing is left for any release to charge. Tests: `lb_dlbk1_two_properties_sharing_a_room_are_quarantined_fail_closed`, `lb_dlbk1_two_live_tenancies_on_one_property_are_skipped_by_the_tick`, `lb_dlbk1_a_live_tenancy_on_a_missing_property_is_quarantined_and_paged`, `lb_dlbk1_a_quarantined_exact_tenancys_rent_is_suspended_and_resumes_on_repair`, `lb_dlbk4a_projections_match_a_rebuild_after_every_mutation` (e5b.rs; `ok()`/`err()` assert equivalence after **every** command in the file); `lb_dlbk1_the_seam_alone_two_keys_resume_one_keeps_it_suspended`, `lb_dlbk1_ack_and_a_module_hold_never_lift_each_other` (e4b1.rs). |
| A3 | D-LBK-2 — tick budgets | done. Housing `MAX_VISITED_PER_FIRE = 256` / `MAX_WORKED_PER_FIRE = 128` (`module.rs:75`, `:79`; `MAX_ASSESSMENTS_PER_FIRE` kept as the worked budget's E5b name), `tick.rs:83`: the cursor advances past the last **visited** id, the due check reads `assess:{id}` before any clone (`clone_tenancy` `:178` counts), `population_version` bumped by `put_tenancy` on a create/end/resolve and by a quarantine change, the idle mark keyed on `(version, every cadence's position)` with a sweep that must be **complete** across fires before it arms (`:92`), `TickStats { visited, worked, cloned, short_circuited }` via `HousingStore::last_fire()`. WoD the same shape in `evaluate.rs:81` / `:115` and `store.rs:47`, `:147` (`EvalStats`, `put_profile` bumps on a new profile, the mark keyed on `(position, version)`). Tests: `lb_dlbk2_a_fire_visits_at_most_256_works_at_most_128_and_idles_between_periods` in both e5b.rs (1,000 tenancies) and e5c.rs (1,000 profiles) — ≤ 256 visited, cursor past the block, 0 visited after a complete idle sweep until a create or an advance, ≤ 128 worked per fire at a boundary with the rest on later fires, `cloned == 0` before the due check. |
| A4 | D-LBK-3 — validating wires | done. `record.rs:806` `SpendingProfileWire` and the nested `CareerProfile`, `HavenQualities`, `PromptOption`, `Prompt` (`:615`), `ProfileEvent` wires: `MAX_PROFILE_HISTORY` 32, `MAX_SOURCES` 8 and one source per kind, qualities ≤ 8, options `1..=4` (`MAX_PROMPT_OPTIONS`, `:47`), `MAX_TEXT_BYTES` 64 on kind/key/quality, a history line and an option label ≤ 256 (`MAX_LINE_BYTES` — the shipped labels run past 64 bytes, so the line cap is the honest one for a label: **deviation from the "64 per text field" letter**), `resources`/`level` ≤ 5, `strain` ≤ 10; the loader's skip-and-re-derive policy unchanged; `+prompt` renders `.take(MAX_PROMPT_OPTIONS)` (`commands.rs:375`). `PendingNotice` gets a 240-byte `TryFrom` wire (`module.rs:178`); the loader drops one oversized entry with a log line and keeps the rest. Tests: `lb_dlbk3_an_over_cap_profile_refuses_to_load_and_is_re_derived` (33 history / 9 sources / two of a kind / 5 options / 65-byte kind / 257-byte line / 9 qualities / strain 11 refuse and re-derive; a profile inside every cap loads as written), `lb_dlbk3_prompt_renders_at_most_four_options` (e5c.rs); `lb_dlbk3_an_oversized_pending_notice_is_dropped_on_load_and_a_240_byte_one_kept` (e5b.rs). |
| A5 | D-LBK-6 — plain-job terminal ops | done. `set_job_status` is `pub(crate)` (`mush-jobs/src/api.rs:396`); `resolve_plain_job`/`deny_plain_job` (`:329`, `:345`) with `PlainJobError::{PayloadNotPlain, WrongQueue, NotOpen, NoSuchJob, NoJobs}` (`:249`) — "open" means not terminal, so an assigned (`InProgress`) story job still closes on recovery; housing's two callers pass `STORY_QUEUE` (`flow.rs:379`, `:420`). Tests: `lb_dlbk6_a_payload_job_is_refused_by_the_plain_seam`, `lb_dlbk6_a_wrong_queue_job_is_refused`, `lb_dlbk6_a_plain_job_resolves_or_denies_once`, `lb_dlbk6_the_fact_provider_still_answers_from_the_applied_marker` (jobs.rs); `lb_dlbk6_housing_resolves_its_story_job_through_the_plain_seam` (e5b.rs). |
| A6 | D-LBK-7 — cadence fields | done. `CadenceClock.generation: u32` (`clock.rs:273`, serde default 1 = `CADENCE_GENERATION_INITIAL`) and `schema: u16` (`:277`, default 1 = `CADENCE_SCHEMA_LEGACY`); `create_cadence` stamps both; `@econ/cadence show` prints "generation N · schema N" (`commands.rs:1370`); no verb repoints, no registration verb, no config key. Confirmed: the cadence doc is outside the journal hash — `plan_hash` covers a plan alone and a cadence is a `JsonSet` doc — so existing worlds boot unchanged. Test: `lb_dlbk7_an_old_cadence_document_loads_with_generation_1_and_schema_1` (e4b1.rs). |
| A7 | E2-C18 — `register_instrument` create-only | done. `api.rs:685` returns `Result`, refuses `EconomyError::InstrumentExists` (`plan.rs:517`; classified `Human(SettlementRefused)` in `sweep::disposition` with the new `ObligationTerminal`); the command and the WoD boot keep their guards. Tests: `lb_e2c18_register_instrument_refuses_an_existing_id` (the API past the command's guard; the live definition untouched), `lb_e2c18_a_suspended_instrument_refuses_a_post_at_the_engine` (a KV-patched `Suspended` status, the e3b2 org pattern, → `InactiveInstrument`, nothing moves). |
| A8 | E5c-C231 + E5c-C2 | done. `mush-server/src/modules.rs:1334` `lb_e5cc231_the_wod_reserved_rooms_reach_housing_through_server_modules_with` (the config's rooms + both start rooms + Limbo reach `HousingStore::is_reserved`; a `None` wod wires no issuer); the wod test renamed `housing_refuses_a_room_wired_into_with_reserved_rooms` (`e5c.rs:322`); `off_creates_nothing_and_shows_nothing` pins exactly one `Manual` `chronicle.period` clock (`e5c.rs:259`); `economy/boot.rs:4` reworded. |
| A9 | E5c-C96 — hoist the lapse guard | done. `evaluate.rs:327`: one period of quiet for every prompt kind, above the housing-pressure branch. Tests: `lb_e5cc96_a_lapsed_housing_pressure_prompt_waits_one_period_before_re_asking`, `lb_e5cc96_a_lapsed_strain_relief_prompt_waits_one_period_before_re_asking` (e5c.rs). |
| A10 | E5b-D34 — drop `Property.upkeep` | done. The field is gone from `Property`; `PropertyWire` accepts and ignores it (`record.rs:309`); the render, `@property/create` and the help no longer know it. Test: `lb_e5bd34_an_old_document_carrying_upkeep_loads_and_is_not_rendered` (e5b.rs). |
| A11 | C17 log | done. `module.rs:43` `CHECKPOINT_REVIEW_SECS = 5`, `CHECKPOINT_REVIEW_ENTRIES = 1_000_000`; `on_world_loaded` logs `entries` and `elapsed_ms` at `info!` (`:712`) and `warn!`s past either threshold naming 08g §12.1 item 5. |
| A12 | Comment-only fixes | done, one line each: `mush-economy/src/commands.rs` (`emit_list` does not sort; the history render is emitted as collected, unsorted and uncapped); `ruleset-wod/src/economy/prompt.rs:237` — the second revision bump **deleted** (E5c-C90: `expire`'s one caller, `evaluate::step`, bumps once at `:180`, so the lapse was counted twice) and the reason left in its place; `evaluate.rs:53` "one `Option` `?` in `behind_tenancy`; cannot panic"; `contract.rs:467` (the adjudicator **replaces** the condition's attestor set) and `:2228` (E4b2's collateral refinement); `mush-org/tests/e1b.rs:327`; `ruleset-wod/src/economy/mod.rs:86` (`+career` renders haven); `mush-faction/src/econ.rs:23` and `data.rs:279` (E3-D47: faction loads after org and reports at boot); `contract_live.rs:19` (E4b2-D31, the plan-first terminal-transition rule); `mush-server/src/persist.rs:1460` (C9: the batch is spooled to the quarantine and replayed at recovery only when every op is an idempotent absolute post-image); `api.rs:656` (E2-C159 holds by usage, not structurally); `splats/kinfolk.rs:50` (T5: the widening is done). |
| A13 | T8 — 08a refresh | done (`docs/design/08a-economy-build-status.md`: hashes, the E5 rows, the fold row, the gate totals, the rustfmt line, the TacticsModule clause struck). |

**Deviations, with reasons.**

1. *Pass 0 reads the projections, so they are built first.* Part B says pass 0 runs "before any derived index";
   08h says the projections are "rebuilt at boot after pass 0" and that "a `Conflict` entry is exactly what pass 0
   quarantines". Building a projection that *represents* a conflict trusts nothing, and it is the cheapest way to
   find (a) and (b), so the boot order is: per-document loads → `reindex_open` + `rebuild_projections` → pass 0
   (quarantine, rent, pages) → passes 1–7. The `open` index (E5b's T7) is a derived index too and always was built
   in `load`.
2. *The housing idle mark carries every cadence's position, not one.* Tenancies name their own cadences, so the
   WoD's `(position, version)` becomes `(every cadence's (id, position), version)` for housing — at most
   `MAX_CADENCES` (32) pairs, read from the snapshot each fire.
3. *A drafted lease keeps the scan awake.* A `Pending` exact tenancy is the tick's restart-safe catch-up for the
   binding edge, so a fire that visits one never arms the idle mark (it is not "nothing due").
4. *Option labels cap at 256, not 64.* The shipped labels are longer than 64 bytes; `MAX_LINE_BYTES` is the cap
   a label gets, every other text field gets `MAX_TEXT_BYTES`.
5. *"Non-open" means terminal.* `resolve_plain_job` refuses a `Resolved`/`Denied` job; an `InProgress` (assigned)
   story job still closes on recovery — the alternative would leave the ST holding a ticket about a home that is
   fine.
6. *`vf_s5` re-pinned.* With the sweep forgiving each suspended period as it falls due, `@obligation/resume` has no
   residual gap to report; the test now pins `forgiven_total == 3` and the `Suspension` cause before the resume.
7. *The tick-budget populations are injected documents.* A thousand tenancies (and a thousand profiles) are
   written as documents and booted, not created by three thousand commands — the invariants are the constructors'
   (`Property::new`, `Tenancy::new`, `accept`), and the boot is the real one.
8. *Rent after `@tenancy/resolve <id>=end` on an exact tenancy* — **was a confirmed money hazard in shipped E5b,
   fixed in this fold (2026-09-04).** As shipped, every path into `flow::end` (`@tenancy/resolve <id>=end|waive`,
   `+home/leave` by the last tenant, the eviction decision) cleared the homes and the latch and touched nothing in
   the economy: the lease contract stayed `Active`, its `Contract(id)`-caused rent obligation stayed `Active`, and
   the sweep kept debiting the former tenant (the probe watched 1000 → 990 → 980 across the end). The economy's
   `land_terminal` already cancels a contract's obligations, but nothing a module could call reached it — a live
   cancel is `@contract/cancel` by staff (E4b2-D21) or a staff attestation, `offer_lease` had no counterpart, and
   boot looked only at live and pending tenancies (economy `boot.rs` suspends a `Contract(id)` obligation only when
   its contract is *not* live). Built, game-neutral: **`mush_economy::end_lease(ctx, contract, &LeaseKey { lessor,
   premises }) -> Result<LeaseEnd, ContractError>`** (`contract_live.rs`, next to `offer_lease`; `LeaseEnd::{Ended,
   AlreadyEnded, Retrying}`) — the ordinary `unwind`: `pending: PendingTerminal` written, the refund plan posted under
   `contract/{id}/refund/{n}` under `ContractAuth::unwinding` (for a lease, the `Lease` encumbrance comes off the
   premises; there is no escrow), then `Cancelled`, which is the step that cancels every `Contract(id)` obligation;
   the key is the lessor + premises `offer_lease` was called with, so a wrong id, a non-lease, a missing contract
   and a frozen economy are refused with nothing written; a terminal contract is `AlreadyEnded` with nothing written;
   an `Offered` lease ends with an empty plan (state write alone); a replay converges with no second post. Housing:
   `flow::end` calls `flow::end_lease` first, before the property lookup, for every `Funding::Exact` tenancy (all
   three callers, plus a `Pending` draft's `=end`), logs the outcome and pages (`@contract/cancel` filled in) only
   when it could not be done (no property record, or the seam refused); `Retrying` is left to the economy's own
   page. Boot **pass 8** (`boot.rs` `reconcile_ended_leases`) ends a still-live lease behind an ended tenancy and
   pages once per finding, never fatal. Housing still moves no money. Tests (`// ---- look-back fold:
   rent-after-end ----`): `lb_rent_after_end_resolve_end_stops_the_rent_and_releases_the_pledge`,
   `…_the_last_tenant_leaving_ends_the_lease_and_a_co_tenant_leaving_does_not`,
   `…_an_eviction_end_ends_the_lease`, `…_boot_ends_a_live_lease_behind_an_ended_tenancy_and_pages`,
   `…_a_terminal_lease_is_a_no_op`, `…_ending_a_pending_tenancy_ends_the_offered_lease` (e5b.rs);
   `lb_rent_after_end_end_lease_posts_under_the_key_once_cancels_the_rent_and_refuses_a_foreign_key`,
   `…_end_lease_on_an_offered_lease_writes_the_state_alone` (e4b1.rs). E4b1-C62 stays noted for E6a on its own.
