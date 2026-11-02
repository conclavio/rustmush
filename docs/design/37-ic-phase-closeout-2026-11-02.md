# 37 — The IC phase: close-out (2026-11-02)

**What the IC phase landed, and what it leaves open**, at `f1ea7715` (origin/main). Written when the owner called
the phase a stopping point. The phase is six programmes that ran between 2026-10-27 and 2026-11-02:

| Programme | Plan of record | Cuts |
|---|---|---|
| IC sorties (seal epochs) | [23-ic-sorties.md](23-ic-sorties.md) | S.0–S.5 |
| M8 contraband and procurement | [17h-m8-contraband-procurement-plan.md](17h-m8-contraband-procurement-plan.md) | M8a–M8h |
| IC field operations (the survey) | [27-ic-field-operations-design-and-survey.md](27-ic-field-operations-design-and-survey.md), [27a](27a-field-unit-survival-and-towing.md) | rulings; H1/H2/H3 scoped |
| The IC combat field | [32-ic-combat-field.md](32-ic-combat-field.md) | CF.0–CF.7, CF.6b/6c, the review fixes, G-4, 512 sites S.0–S.2 |
| Ship free flight, combat and mining | [35-ship-free-flight-combat-and-mining.md](35-ship-free-flight-combat-and-mining.md), [35a](35a-space-flight-review.md) | SF.1–SF.20, then SF.R0–SF.R8 |
| Ground combat after the field | [36-ground-combat-after-the-field.md](36-ground-combat-after-the-field.md) | GC.0–GC.13 |

Method: five independent read-only audits at `f1ea7715`, one per programme and one over the rest of the tree
(open PRs, CI, code markers, gates, the other cut-notes files, deploy records). Every cut was checked against the
tree by its own anchors (verbs, modules, the gate tests its as-built section names), not by its docs. An item
is marked closed only where the tree shows it. Line numbers are those files' at `f1ea7715`.

## 1. The verdict

**Every planned cut of every programme has landed except doc 23's S.4b**, which every later doc names as its
blocker (licensed minds in a field, the production Open/Close of a mind's claim). Every gate test the as-built
sections name exists; none was renamed without a note except `cf7_the_field_story` → `gc0_…` (noted in doc 32
§17/§20, not §19).

The tree is **not in good shape to stop on** in four respects, all of them cheap:

1. **Main is red** at `f1ea7715` (§3.1). Three of the four failures are SF.R8's and PR #324 fixes them; the
   fourth (`f6`, a field-carrier departure) is a load flake: 3 of 8 fail pinned to two cores on conclav.dev.
2. **The nightly has failed three nights running** (16b cut H harness ×3, M7 market commands ×2). Nobody
   owns it.
3. **The docs lag the tree** (§4): doc 35 has no as-built record beyond "(Built)" rows, doc 32 has no as-built
   for CF.0/CF.1, doc 23's header stops at S.3, M8 has no gate sign-off, and the README index is behind on
   eight rows.
4. **Nothing of the phase is deployed.** dev1/dev2 run `830f2265` (2026-10-31), 217 commits behind main;
   every IC-field, GC, SF and S.* cut is undeployed, and the space track has **no enable flag of its own** (§5.3),
   so the next redeploy switches it on wherever TacSim is enabled.

What remains beyond that is of four kinds: work the programmes themselves marked owed (§5), docs text owed
(§4), owner decisions (§6), and adjacent areas the rulings deferred to (§7).

## 2. What landed

### 2.1 Doc 23, IC sorties

| Cut | Merge | What |
|---|---|---|
| S.0 | #36 `a477e9c3` | the doc |
| S.1 | #41 `59006edd` | the surface epoch: `SortieId::ic`, `open_epoch` (`surface/black_box.rs:984`), `OpenSortie` |
| S.2 | #45 `b4103c96` | the ship epoch (`port/epoch.rs:59`) |
| S.3 | #49 `3071c1f3` | abandonment and recovery (`excursion.rs:1273`) |
| S.4a | #93 `913e29dd` (as M8e) | `+tv/<n> launch mind=` (`surface/dispatch.rs`) |
| S.5 | folded into M8g | the `m8-gate-stories` CI job (ci.yml:125), nightly + dispatch |
| **S.4b** | **not built** | production Open/Close, the pool seat, the `abi` drop on resume, long-leg snapshots. `fixture_start`/`fixture_close` are still the only producers (`minds/ic_sortie/participant.rs:500/514`) |

### 2.2 Doc 17h, M8

M8a–M8h all merged (#67, #78, #99, #77, #98, #112, #93, #102/#105, #129, and M8g as #111/#106/#132/#156/#163).
The three M8 stories run under `m8-gate-stories`; the leak battery (`m8-leak-battery`) and house trials
(`m8-house-doctrines`) are dispatch-only. The last recorded leak dispatch is run 40880419607. **No doc records
the M8 gate as passed**: doc 17 §22 restates it, 17h §3 asked for a §24 as-built entry that was never written.
The unit-minds market is still effectively `NOT_OPEN`: the composition slot exists, but `launch_path` ships
commented out in `deploy/instance/mushd.toml.tmpl:413`.

### 2.3 Doc 32, the IC combat field

| Cut | Commit | What |
|---|---|---|
| CF.0 | #207 `f6cfee4c` | `PartyId` (`arena/party.rs:50`); the three-party practice |
| CF.1 | #202 `02b7471f` | `SiteRecord`, `PresenceBook`; the ground survives a restart |
| CF.2 | `c3cf0909` | the field host: `every_class` join, `DeployedMark::field` |
| CF.3 | `07a0dd9e` | the carrier as a unit; `refuse_tow`, `SHIP_DAMAGE` |
| CF.4 | `de568d57` | one picture: `campaign_layer`, `field/known.rs` |
| CF.5 | `2fd7d974` | crew and exit: `ArenaOut::Withdrawn`, `bay_gate_leaving`, `CrewLocation` |
| CF.6 / 6b / 6c | `d8d4a24c` / `2232c2c5` / `731c2ab1` | NPC pilots and raiders; the raider's carrier and withdrawal; the armed-rig lease (`LeaseGate::Escort`) |
| CF.7 | `92121920`, `24eaef17` | F-6 departure (`Then::Lift`); the whole field story at 256 |
| review fixes | `f5eab31e` | `FieldClosing`, `counted`, `owed_exits`; the crash stories |
| G-4 | `85977d29` | the bot in an IC field (`mining_cohort.rs:923`) |
| 512 sites S.0–S.2 | `df43691e`, `99c28f9c`, `9b51c6c0`… | `Scheme::of_side`, the 512 pad, Urvara Reach, the measurements |

### 2.4 Doc 35, the space track

SF.1–SF.20 shipped first (PRs #266–#291), were reviewed in 35a (an island: no tick, RAM-only, 11 authority
holes), and were re-cut as SF.R0–SF.R8, all merged on 2026-11-02 (#297, #302, #309, #311, #313, #316, #317,
#318, #321). Each has one story in `tests/shard_2/voyage.rs` (`sfr1_`…`sfr8_`). Of 35a's eleven holes, nine
are closed, one is half closed (7: parties are still `SpacePartyId` constants, one carrier per bubble) and two
are open (10: fake NPC ids; 11: tow constants and a literal `/ 24`). Of §5's twenty bugs, eighteen are closed.

### 2.5 Doc 36, ground combat after the field

All fourteen cuts merged between 2026-11-01 and 11-02: GC.0 #294, GC.7 #295, GC.1 #298, GC.2 #296, GC.13 #303,
GC.1b #300, GC.12 #306, GC.8a #304, GC.8b #310, GC.9 #299, GC.11 #307, GC.3 #308, GC.4 #312, GC.10 #315,
GC.5 #314, GC.6 #320. Every §7 cut has an as-built section and its anchors are in the tree (the audit cites
`field/exit.rs:80`, `arena/join.rs:101`, `arena/wreck_cover.rs`, `tacsim::carrier_damage`, `surface/tow.rs`,
`surface/salvage.rs`, `surface/claim.rs`, `port/bay_work/`, `field/standing.rs`, `surface/raid.rs`,
`FIELD_MAX_UNITS = 100`, `port/medical.rs`). Doc 36's own close-out list ("What the programme still owes") is
accurate but short; §5.4 below adds what it missed.

## 3. Where the tree stands

### 3.1 CI

- **Main `f1ea7715` is red** (run 41527199335): `cost_lane::sm12c_every_act_kind…`, `sm12c_a_restart…` and
  `drones::sm12b_a_delivery…` count a `tacsim:space:state` write that SF.R8 now makes on every tick; **PR #324**
  gates it to real changes and is green bar one cell. `field_carrier::f6_a_carrier_lifts_out…`
  (`shard_1/field_carrier.rs:316`) failed once on CI and passed on the four main runs before; SF.R8 touched
  nothing on its path. Reproduced at `f1ea7715` on conclav.dev, eight copies pinned to two cores beside four
  hogs: **3 of 8 fail**, at `:331` ("the open promotion names the carrier's exit") — the departure has filed but
  the site record has not yet marked the exit, so the two writes race under load; CI's `:316` is the same test
  running out of its 20 s wait. Doc 36's GC.11 note already says this test passes only because the first join is
  refused and that the root fix (await the think batch before joins) was set aside.
- **Open PRs:** #324 (above) and **#323**, the bot's retry after "Your earlier action is still committing"
  (4/4 → 4/4 with the push-back forced); its two red cells are #324's three tests, inherited from main.
- **Load flakes fixed this week:** cf5 (#319, test-only: open the field with a mark, not a fire order), gc11
  (#322, a real bug: the director now retries a raid's partially refused joins, 96/96 under load).
- **The nightly is red three nights running** (10-31, 11-01, 11-02): `tacsim-harness` ("16b cut H") all three,
  `mind-commerce-acceptance` (M7 market commands) two of three. The last gate.yml run (11-02, `eaefe6a0`) also
  failed. Untriaged.
- **Six jobs are dispatch-only and never run unattended:** M6 author acceptance, M6 full replay,
  `m8-leak-battery`, `m8-house-doctrines`, `bot-win-rate`, `bot-muster-rate`. The `#[ignore]` stories under them
  (m8_leaks ×3, house trials, field_scale) have no automatic owner.
- `#[ignore]`: 49 real attributes, 15 added since 10-20, all accounted for (dispatch-only stories, by-hand
  smokes, the release-only month). No `TODO`/`FIXME`/`todo!()` anywhere in `crates/`.

### 3.2 Deploy

The tree's last deploy record is 33-cut-notes:247 (`30d857ac`, 10-31 14:46Z); the later redeploy to `830f2265`
(10-31 20:31Z) is recorded nowhere in the tree, and 27a-cut-notes:303 still says `d4d81c89`. Main is 217 commits
past what runs. `devN-redeploy.sh` lives only on the boxes. PR #245 (the W-clock checkpoint) **is** inside
`830f2265`, so the "never arms on a fresh store" defect is deployed-fixed; #262 (the K3 lane) is not.

### 3.3 conclav.dev

96% full (22 GB free) after the cut sessions: `cf6b-target`, `cf7-target`, `cfrev-target`, `g4bot-target`,
`gcfix-target` and the `cib`/`f6` clones are all dead and can go.

## 4. Docs owed (text, not code)

| Doc | What is wrong | Fix |
|---|---|---|
| README index | eight docs 23–36 unlisted (24, 27a ×2, 28, 29a, 30/31/33 cut-notes); rows for 29, 30, 32, 33, 34, 35, 36 say "planned"/"docs only" though the cuts landed; Draft numbers behind on 20, 21, 26, 27, 29 | one pass |
| 32 | no as-built for CF.0/CF.1 (PRs #207/#202 touched no docs); §7 cites CF.5 as §8 (it is §10); "ceiling (24)" is now 100; §19 says raiders.json schema 3 (tree: 4) and `cf7_the_field_story`; §13 test paths predate the shard split; the §6 promise to give doc 27 §5 a field-engine row and mark W-clock built was never done | an as-built note for CF.0/CF.1 and a consistency pass |
| 35 | header still Draft 0.3 "2026-11-01"; §10.2's "(Built)" rows restate the plan and record no divergence (the audit found plenty, §5.3); no cut-notes file | a 35-cut-notes.md, or as-built sections, written from the eight PR bodies |
| 23 | header "S.0 to S.3 as built" (S.4a is in §14); "`Destroyed` reserved" (23:341/349/356) is stale since GC.3 closes epochs `Destroyed`; anchors at 23:796–799 drifted | header and three lines |
| 17h / 17 | no M8 gate sign-off; 17 §24 has no M8 as-built entry; M8d's four tests owed to M8g (17h:802–806) not recorded as closed, and no named tests found for them | a §24 entry that either signs the gate or names what is still owed |
| 36 | the close-out section misses the items in §5.4 below | append |
| 27 §5 | the field-engine row and W-clock "built" mark (promised by 32 §6) | two rows |
| 34 | header still "docs only"; CS.0a–CS.5c landed | header |

## 5. Outstanding work the programmes marked owed

P0 = before the next redeploy or playtest; P1 = the next cut in that area; P2 = backlog.

### 5.1 CI and tree (P0)

- Merge #324; rerun #323's two cells; merge #323.
- Fix `f6` (3/8 under 2-core load, §3.1): the exit mark on the site record should land in the same batch as
  the departure filing, or the test should wait on the record rather than the voyage; take GC.11's deferred
  root fix (await the think batch before joins) in the same change rather than widen the wait.
- Triage the nightly: `tacsim-harness` and `mind-commerce-acceptance`, three nights red.
- 20b CI-1 still stands: `drones` ~182 s and `cost_lane` 105–175 s against the 90 s budget.
- The matter pin `sm12a_only_the_clerk_the_foundry_and_the_mill_create_matter` (`shard_3/mill.rs:2508`) counts
  `.mint_origin(` only; GC.8b's scrap mints through `.mint_origins(` (`bay_work/act.rs:741`), which no pattern
  covers. GC.3 handed this to GC.8b; still open.

### 5.2 Doc 36, ground combat (P1 unless marked)

The doc's own list (confirmed open): `Relations::note_fire` tells a friend of both parties (`arena/party.rs:344`,
needs the next rebless); a towed casualty is not a kernel unit, so a hit on it is not modelled and a tow cannot be
released inside a promoted field (`tow.rs:49–54`); `stock_mind::line_to` cannot see wrecks (`stock_mind.rs:634`);
the ledger `max_bytes` at 100 units (`mush-config config.rs:1626`); GC.1b's refused arrival is never retried
(`carrier.rs:567`); a restart between a raider's join and the next snapshot stands it at the pad; a DT-flight
epoch is left open at close-destroyed or a load (`provider.rs:269`); no shard_1 story for SCREEN/SUPPORT as
delivered orders; a vacated hull is not returned to its title holder; `Right::Salvor` permits only Repair and
Berth (`bay_work/mod.rs:103`); `documented_repairs_at_risk` is always empty and the recovery-authority grants are
hard `false` (`claim.rs:598–618`); no crate into a hold and no crate dropped by play; a written-off rig's crew
aboard the carrier; the hopper's ore not in a casualty's laden mass; the built bay's per-unit grant, scrapped-shell
disposal and typed order family; the bot's escort vocabulary stops at tow/load/release (`mine/escort.rs:820`);
`SpacePartyId` (51 hits), `CarrierPart` unread outside tacsim, `file_salvage_claim` with no space caller.

What the doc's close-out missed (the audit's additions):

- **The §7 "stories grow" rule is unmet**: no story runs whole at both 256 and 512. Story (a) runs on Ahuna
  Flank only, (b) on Urvara only, (c) at a port only.
- **Story (c) against its gate** (`bay_gc8.rs:227`): the engine is rebuilt in place (no removed engine becomes
  a lot with parent provenance); the hull is a staff fixture, not story (a)'s casualty; "fields again" is
  asserted as `Operational`, not by fielding.
- **Story (b)**: the contest runs on the title holder's sensor record, not its black box; GC.6 says black-box
  lines are never read as evidence. There is no verb for a defective-procedure or superior-right contest; an
  org-titled hull with no lessee cannot be claimed; `claim_window_s` is not settable from config.
- **Q-36-35/36 are narrower than ruled**: the bay works only docked at a port with a workshop
  (`bay_work/mod.rs:32, :223`); field repair and rearm from carried munitions are not delivered.
- **Stories (b) and (c) over SSH** (Q-36-3): no bot vocabulary for `+bay`, `+salvage`, `+field/void`,
  `+sortie/condition`. Story (a) over SSH also still owed. GC.11's "captain story runs once with a director raid"
  has the knob (`FIELD_RAID=director`, `mining_cohort.rs:924`) but no recorded run.
- GC.5: a write-off runs no term settle. GC.4: every tug is `TowRigProfile::Standard`; a unit must already be on
  a site's index to be towed; the friendly-load half (Q-36-52) is tested as a pure rule only. GC.2: a jammed or
  damaged non-firing part changes nothing in the match yet. GC.8a: no part market or price. GC.8b: goods drawn
  then refused within one fire are lost. GC.10: the fire tally and void offers are RAM; an NPC leader cannot
  void. GC.12: the gate.yml field_scale sweep has never been run; a cap above 100 needs its own measurement.
  GC.13: beds are authored but not a limit. GC.3/4: the close's black-box line is covered only by review.
- **Expensive to reverse (owner, §6):** the workshops live in existing rooms (GC.7/GC.8a); moving them now means
  re-homing berthed hulls.

### 5.3 Doc 35, the space track (P0 before any redeploy)

- **No space-only enable flag.** `space::tick::register` and `+helm`/`+tac`/`+mine` register unconditionally
  (`lib.rs:1023`, `space/commands.rs:470–510`), gated only by `[tacsim].enabled`. Add a `[tacsim.space].enabled`
  defaulting false before the next dev1/dev2 redeploy, or accept exposure.
- **`tacsim:space:state` is one unbounded document** written whole on change: `completed_space_mine_draws` is
  push-only (`mining.rs:1589`), `derelicts` never removed, `tape.entries` grows without bound.
- **D-35-1 violated in code**: `SpaceShipState::new_dray` hand-typed constants are the base for every hull
  (`custody.rs:201`); the bubble thrusts at 25 m/s² (`bubble.rs:345`) where `dray.json` gives 0.49; the corvette
  and raider are `tacsim.unit.corvette`, which has no unit file and never goes through custody.
- **Holes 10 and 11 open**: NPC hulls are `ObjId(900_000+domain)` etc. (`tick.rs:1452`, `picket.rs:1248`),
  contacts `hull+1000`, and these key derelict and salvage records; `/tow` uses `prior_owner = derelict+1000`,
  `base_obj = ObjId(domain)`, errors swallowed with `.ok()`; the real-seconds fallback is a literal `/ 24`
  (`commands.rs:3468`, `tick.rs:907`), breaking D-35-3.
- **Hole 7 half open**: `SpacePartyId` constants, not arena `PartyId`; **one carrier per bubble** (`by_hull`),
  so two PCs at one boundary get private bubbles — this contradicts the multi-party ruling that governs the IC
  field.
- **Clients unwired**: TUI (`tui/pixel.rs:1338`) and web (`PictureView.svelte:814`) send bare `DOCK …` lines
  that no verb accepts; `DockingClientSession` and `encodeDockingTape` have no callers, so no client submits a
  docking tape; the space view pushes feeds only after a player command, never on the tick; `+tac/decoy` is
  referenced but not a switch.
- **D-35-17 not carried out**: cluster rocks and Rhai doctrines are Rust constants; `outer_catalog()` is
  `include_str!` + `unwrap_or_default()`, not in `RulesPack`. Bug #17 (Tk decode default, no record of the
  path) open.
- **§9 acceptance by fiat**: `sfr6` disables the raider by hand before `+tac/beam`; `sfr2`/`sfr7` set
  `DisabledDerelict` directly; `+helm/run` has a unit test only; story 2's claim-jumper duel, refit and
  Neptune/Triton burn have no test.
- Mining at rate 24 fills an 86 t hold in ~20 real seconds (tuning, not a hole).

### 5.4 Doc 32, the field (P1/P2)

- F-1 relations: GC.10 did shared membership only; declared hostility and alliances between organizations open.
- F-4 reinforcement: open by ruling (the launch refusal stands; Q-36-5 wants a launch rule designed for it).
- F-7 known-hostile memory: waits on the Epistemic Engine.
- A settled carrier is drawn as the pad on the presence picture; its one-cell footprint.
- NPC scavengers (objects are not in the mind API); ejection; capture.
- CF.6c: an armed hull for sale; Occator recovering a written-off wreck; the mixed bay's payload headroom check
  at lease time.
- The cockpit's line text gives no relation (§16 finding, not taken up by doc 36).
- The bot's address token reads 1–3 digits (`mine/lines.rs:328`) against 512's four; the pocket cursor's 64 rows
  against a 512 block (`surface/cursor.rs:35`); a `frontier` zone word; the owner's 512 SSH hand smoke and the
  mush-pixel encode of Urvara's window.
- `SalvageClaimBook::apply_world_clock_event` (`field_ops/salvage.rs:1431`) has no subscriber (Q-F3).
- 27a §9 forensics: `record_arena_notices_to_black_box` and
  `mark_stripped_component_contraband_on_vacated_claim` have no caller outside tests. 27a §10 field engineering
  (tender in the field: unjam, stabilize, resupply) never built; kept out by Q-36-35.
- Q-R5 wages: open, as intended.

### 5.5 Docs 23 and 17h (P1 for S.4b; P2 the rest)

- **S.4b** (P1): the blocker named by docs 32, 36 and GC.1. Its precondition "custody for moving anchors" has no
  owning doc.
- S.1/S.2/S.3 owed: black-box retention for a full box (`black_box.rs:36`), the 4096 cap tuning, the `Missed` /
  from-the-field / helm-fallback pages' integration tests, what a lessee or recoverer may do with a foreign rig,
  the third-carrier re-target, a carrier wreck writes nothing to `tacsim.ship_epoch`.
- M8: the M8a delegation pin; M8d's four tests; tier 1 as one story; M8c `org=` on resale/charter accept and
  recall notices; M8d3 physical core removal and a Registry custody place; M8e no recall bit at the seal; 17h §4
  waiting list — testimony has no producer (`earned_contacts.rs:4` says so even though the field now exists),
  cored ship (`dray.json:198` still Class 0), the Tether's back room as vendor, hull cover live, seizure of hulls,
  the region sim task and `MIND_ABI` window, `Legality` on tacsim content.
- 17g C4 was never built; its ledger items (`next_key`, the Registry-status panel, the forever-retrying renewal
  draft) have piled up.

### 5.6 Standing debt outside this phase (P2, for the record)

33-cut-notes: nothing in the IPC window is gated for real; the web `hello` manifest after a boot into the gap;
the same-process reconnect race; `Attach`/`Detach` bookkept but unsent; `dev2-redeploy.sh` not in the tree.
29a: no per-account connection cap (B-4); `mushd` still `IpGuard::new()` with the default deadline; no
`/metrics` refusal counter; the ufw/ACL closure of 6790/6793. 31-cut-notes: D-31-5 mounts/muzzle locators,
`MAX_CONFIGURED_PACKS` counting designs, the studio never seen against a live server, a core refit while in a bay.
24: parallel frames off by default, the production sight cache unbuilt, both cohorts over the frame budget.
21a: 118 open items; G.6b, G.9, U.5 unbuilt. 16c: T-89/92/93/96/97/98/100/101/103/106 and **T-112 backups**,
still none. 20b: OD-2/OD-4 defaults unruled, DF-1 O-22, CI-1..5, §8 doc text in 16/17/18.

## 6. Owner decisions waiting

1. **Space exposure on redeploy**: add `[tacsim.space].enabled` (recommended) or ship the track live.
2. **Workshop rooms** (GC.7/8a): leave in existing rooms, or move to dedicated rooms now, before hulls are berthed
   on a live box.
3. **Repossession damage charge** (GC.9 Q): `repossess` (`tv.rs:4996`) takes none today.
4. **Restart mid-countdown** (27a-cut-notes:295): resume or restart the W-clock countdown in full.
5. **M8b-2**: does the Class III entitlement apply to an heirloom. **M8h**: 365-day term vs 90.
6. **The M8 gate**: sign it passed on the three CI stories plus run 40880419607, or name what is still owed.
7. **`launch_path`** (mind commerce live on the boxes) stays an owner decision (deploy/README:722).
8. 20b OD-2 (SM.10 contest-fee restore, the pad's "no passage", SM.11r N5, O-15, the AP default) and OD-4 lead
   defaults — provisional since 10-26.
9. **The first routine redeploy of the phase**, and the 512 SSH hand smoke that waits on it.

## 7. Next adjacent areas

In the order I would take them; each is a doc-and-cuts programme of its own.

1. **Space-track hardening (doc 35b).** The §5.3 list is the space side of what docs 32/36 built properly on the
   ground: real units through custody (D-35-1), real ids, arena `PartyId` and multi-carrier bubbles, a bounded
   persisted document, the enable flag, and the client docking game wired. It is a prerequisite for the space
   tow seam (`file_salvage_claim`, `CarrierPart`) that GC left for "space's own cut". Owner-flagged because the
   fake-id keys will be expensive to migrate once anything is persisted on a live box.
2. **S.4b: licensed minds in fields.** Blocks Q-36-44 (such a hull stands as a wall), M8g's remainder, and the
   testimony producer. Needs the "custody for moving anchors" note first.
3. **Capture and prisoners** (Q-CF-38). The stub "until capture is built" appears in Q-36-45/52 and CF.5's
   extraction; ejection sits with it.
4. **Organization stances** (Q-36-10): declared hostility and alliances, the faction relationship matrix; F-1's
   other half. Small, and GC.10 left the seams.
5. **Field engineering** (27a §10, Q-36-35/36): the tender in the field and the bay working away from a port
   (field repair and rearm from carried munitions). Closes the gap between what was ruled and what GC.8a built.
6. **The bot over SSH for stories (a), (b), (c)** and the director-raid captain run: vocabulary for `pickup`,
   `+salvage`, `scrap`, `+bay`, `+field/void`, `+sortie/condition`. This is the acceptance proof doc 36 §7 asked
   for and nothing else gives.
7. **The stories at 256 and 512**, and the field_scale sweep on gate.yml (the cap above 100).
8. **The second kernel rebless** (Q-36-56's two deferred items: `note_fire`'s friend-of-both and the think-batch
   await before joins) together with whatever f6 turns out to need.
9. **Reinforcement launches** (Q-36-5), **DT.4 sub-leaders** (Q-36-43), **a crime system** over the evidence
   GC.6 records (Q-36-30), **a technician and labour market** (Q-36-32), **a carrier-wreck tug** (Q-36-8).
10. **Operations**: the routine redeploy, T-112 backups, the ufw closure, the nightly's owner, the dispatch-only
    stories given a weekly slot.

Out of scope by ruling and still so: the 10 km ground, infantry and battle armor, a mission generator, persistent
known-hostile memory (the Epistemic Engine), facility assaults, space-only work beyond the three seams.
