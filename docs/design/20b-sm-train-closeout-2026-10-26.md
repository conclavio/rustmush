# 20b — The surface-mining train: close-out (2026-10-26)

**What the SM train landed, and what it leaves open**, at `8ae600f8` (origin/main). Written when the owner called
the programme concluded. Sources: [20-cut-notes.md](20-cut-notes.md) (every cut's record),
[20-surface-mining.md](20-surface-mining.md) (doc 20, Draft 0.14, as built through the whole train; its §13 cuts,
§15 owner questions, §16 risks, §17 reserved work, §18 amendments owed) and
[20a-sm12-implementation-plan.md](20a-sm12-implementation-plan.md) (doc 20a, the SM.12 plan of record; its body is
the plan as written, not amended). Method: five independent read-only audits, one per slice of the cut notes and
one over doc 20's and doc 20a's registers; an item is marked closed only where a later record says it was done.
Line numbers below are those files' at `8ae600f8`.

## 1. The verdict

The programme is **complete as designed**: every row of doc 20 §13 has landed, doc 20 reads as built through the
whole train (Draft 0.14), and every cut-level hand-off between SM cuts was later delivered or ruled unneeded. What
remains is of five kinds: owner decisions (§4), features deliberately not built (§5), known defects and risks
(§6), CI and operations debt (§7), and design-document text owed outside doc 20 (§8). **Nothing of the train has
been deployed**: no deploy or rate-24 live smoke is on record (the last recorded dev2 build, `3aa8238b`, predates
SM.11), and every note says "no deploy without the owner".

## 2. What landed

| Cut | Commit | Date | What it delivered |
|---|---|---|---|
| SM.0 | `11c50e84`, `f3df3205` | 10-19 | Doc 20 itself (Draft 0.3, then 0.4 with the owner's review: Q-20-1..13 accepted) |
| SM.1 the ground | `61324cba` (+`6f797974`) | 10-19 | Keyed HMAC geology over a fourth durable key family, its fingerprint pinned; reading rule v1; the site-ground LRU; `@geology`; the atlas `i16` validator |
| SM.2 sites and landing | `bd718fb2` | 10-19 | Lane v9: six mining sites (five on Ceres, Rheasilvia on Vesta), the Freeport Registry, the landing law and ascent term, life support at lift, the leg-home reserve and its throttle hint, `+ship/lift` |
| SM.3a the vehicle | `96cc7303` | 10-20 | Goods v3 (four ores), the Vehicle class and tools, the TV and the Dray-P with a two-slot bay, lane v10's rig lessor, `+coop/refit`, `+tv/lease` with an engine-owned deposit |
| SM.3b the clock | `eb99c9be` | 10-20 | The excursion family and projection, the 1 s surface tick, authoritative `+tv` with launch, recover and dump, the departure gates |
| SM.5 legal and market | `64cd1f8e` | 10-20 | The prospecting licence (`+licence`), the ore bids and their intake drip, Occator's capital raise, `+market/sell`, lane v11 |
| SM.4a knowing | `354832bf` | 10-20 | The chart family (coverage and provenance), `+tv/scan` and `+tv/sound`, heard soundings, `+tv/quiet`, `+tv/forget`, the ground warmed on the tick |
| SM.6 industry seams | `70c2cd12` | 10-21 | Deposits opened by each site's clerk, `ExternalDraws`, the pocket cursor, `transfer_many`, the tenure machinery |
| SM.4b driving and the map | `d367bf3b` | 10-21 | `+tv/drive` by A*, a real trip home in the reserve, recover from anywhere, `site.map` and `+tv/map` |
| SM.7 mining | `dd48ed45` | 10-21 | `+tv/mine` with its preview, rights per cycle, pocket locks, the `tacsim:mined` ring |
| AP the path cost | `74c5801f` | 10-21 | The atlas A* about nine times cheaper in release; planning a release gate |
| SM.8 unload, sell, the story | `9922c9c7` | 10-21 | The timed unload, queued after a laden recover; the hopper refused at sale; the twelve-beat story test |
| SM.9 the quarry screen | `16b0ca4e` | 10-22 | `+chart/ground`, the TUI `q` screen and the web quarry screen, the `tv.status` keyframe |
| SM.10 claims | `fed09da2` | 10-22 | `+claim/stake|renew|release|contest|list`, `+registry`, the hand-made evidence rule, fees, seeded claims, lane v12 |
| SM.11 plans and offline | `09854d6a` | 10-22 | `+tv/plan`: durable fill plans stepped by the tick; `via: Plan` provenance; `+ship/order await tv` |
| SM.11r | `8cc0aa64` | 10-24 | Two defects of the owner's review: legacy chart rows keep their discovery; plans read the hold's room in every measure |
| SM.12a the mill | `9df93246` | 10-25 | Lane v14's two mills (5 : 2), the batch and restock on the Ceres clerk's arm; the relay seed re-signed at v3 |
| SM.12b Occator's drones | `1b5dd371` (+`f171b36c`) | 10-25 | Lane v15's yard: two house rigs, a dispatcher filing plans and delivering ice in kind; the relay seed at v4 |
| SM.12c-bot | `d2f4ee7c` (+`d842fdd0`) | 10-25 | `mush-bot --mine`; the four-bot SSH cohort (run by hand, passed) and its every-build canary |
| SM.12c-lane the cost lane | `389383f1` (+`8ae600f8`) | 10-26 | The ticks' own clocks, the write census, the release p99 gate (0.77 ms at ten TVs), the restart twin, the 30-TV month; the `NothingLeft` refusal and the month's fixes |

Doc folds: Draft 0.5 to 0.14 (`5cdcd3f2` … `6cd5fb29`). Doc 20a: Drafts 0.1 to 0.4 (`cdb243df` … `a818f03a`).

## 3. What the programme proved

The loop runs end to end on the production root: four `mush-bot` miners over SSH each mined and sold 12 t
(735.00cr net, loops of 14.7–25.0 min). The surface tick holds a release p99 of 0.77 ms at ten TVs and about
1.6 ms over a 30-TV IC month; every mining act's writes are pinned by a census. The month (by hand,
`scripts/cost-lane.sh`) passes with no breach after the 2026-10-26 follow-up.

## 4. Owner decisions still open

**OD-1 — the per-party contract cap (O-19c), the one fork the record carries as open.**
`mush_economy::MAX_LIVE_CONTRACTS_PER_PARTY` (32) stops a 64-TV world at its kit-up and binds in play at 30 TVs:
with 30 rig leases live, Occator settles two sales at once and a third holds its sailing plan until resumed. The
lead's default: the cohort runs at 30 and the cap is not raised. Raising it is a mush-economy change, owed before
any 64-TV month (doc 20 §15, `20-surface-mining.md:2602-2614`).

**OD-2 — questions put to the owner with no ruling yet:**
- SM.10's (a) a contest fee withdrawn after the transfer: should the loser's claim be restored? (b) an
  expired-unaccepted renewal fee lapses the whole claim, losing a term already paid — no test pins the renewal
  case; (c) an exact per-block heard log versus the 512-row scan (doc 20:2499-2506).
- The site pad's passage home: `+chart/ground` from a site's pad reads "no passage from <site>"; show the Freeport
  leg, or route site to site (doc 20:2475-2479).
- Q-20-10 Rheasilvia: reachable only throttled and in season (0 of 365 days at full throttle, the first fit 123 IC
  days from the epoch); a cheaper reserve or a changed site is the owner's (doc 20:2665-2670).
- Q-20-12: whether the production box's protected backups include the geology-key directory — losing the key
  disables mining for the world's life (doc 20:2682-2688).
- SM.11r's N5 (with bulky cargo the site map's hold line, by mass, and the plan's preview, by family, disagree) and
  N6 (`PLAN_ENDED`'s wording once volume binds): raised in the cut note, never put in §15 (20-cut-notes.md:3914,
  3917).
- O-15 (the drones' soundings): doc 20a's original figure stands as measured (4 heard lines, at most one an IC
  hour); flagged for the owner to re-see after a wrong correction (doc 20:2562-2566).
- AP's default that debug planning timings never gate the push shard reads as the lead's (doc 20:2453), though the
  owner's provisional O-25 took the same shape for the cost lane.

**OD-3 — the owner's provisional rulings of 2026-10-24, to revisit when the economy asks:** O-1 (mills 20 t a
batch, 40 t an IC day), O-5 (a mill's ratio fixed for its ledger's life), O-7b (milled matter enters by credit, no
lineage — the expensive one to reverse), O-8 (the yard's names, two rigs, the Dray-P), O-9 (F and G worked; F 271 t,
G 256 t, J 252 t, K 252 t of fair ice), O-10 (drones share the mill's rate), O-14 (the house re-arms), O-19 (content
bounds 32/64), O-25 to O-28 (the push-shard ceilings, Vesta refused at the epoch, the month's STOP rule, run by
hand). **Any lane-data change to O-1, O-8, O-9 or O-12 now needs its own lane v16 "owner's numbers" cut**, with the
relay seed re-signed again (doc 20a:466, 537).

**OD-4 — lead defaults never ruled.** Every cut took owner-critical defaults the owner never ruled on; doc 20 §15
records each as "the lead's default, standing until the owner rules". The owner may accept them as a body or rule
item by item. The ones most expensive to reverse are bolded.

| Cut | Defaults standing |
|---|---|
| SM.1 | decision 13 (a shift never lifts a province-barren family) |
| SM.2 | decisions 4 (life support at lift), 6 (surface figures on eight bodies), **7 (the leg-home reserve)**, 8, 10, 16, 17, 22 |
| SM.3a | **D1 (a bay on every programme hull)**, D8/R1 (term settle at the next lease act), D11/D17 (the counter, the rig marks), **D16 (the repossession money rule)**, **D18 (`tacsim.tv.deposit` in mush-economy's frozen engine table)** |
| SM.3b | decisions 9 (`RECOVER_S` 300), 10 (`DUMP_S` 0), 11, 13, **14 (the control lease read as the lessee aboard)**, 25 |
| SM.4a | D7/D8 (who hears a sounding), D25 (cold-site and dark-layer texts) |
| SM.4b | P4, P6 (the 5 % floor waived for the way home), P9, P14; the 122-column legend |
| SM.5 | **16–18 and the forfeit (licence policy: grace authorizes, one an account, no cap ahead, the fee to the Registry)**, 19–22 |
| SM.6 | D1, D2, D9, **D14 (only ore-bearing rows open; barren deposits need an industry record change)** |
| SM.7 | R5, R6, R7 (`+tv/mine stop` commits without `go`), R15, R16, every mining text with `RUN_DARK` |
| SM.8 | **R5 (an unload works only on a site's pad)**, R6–R8, about 27 new texts |
| SM.9 | the rig key `t`, the two-family `V`, the glyphs, `z`'s cycle, the TUI's interpolation step |
| SM.10 | **OC1–OC17 (the fee arithmetic; account-wide cooling)**, the planner's rulings 1–13 and D1–D20 |
| SM.11 | D2, D4–D8, D13, `await tv` per rig, the 16/64 caps, the `Markers` retry text |
| SM.12 | O-2, O-2b, O-3, O-4, O-6, O-7, O-11, O-12, O-13, O-16, O-17, O-18, O-20 to O-24, O-29, O-30; SM.12b's player-visible bound change (a parking plan keeps its reason) |

Every new player- and staff-facing text since SM.2 is in the same position.

## 5. Features deliberately not built

**DF-1 — staff remedies the texts already promise.** No staff verb closes an excursion or brings a rig in
(`@tacsim/tv close|home`, O-17, deferred to "a named follow-up cut" that was never named), though `STRANDED`,
`TV_AWAY` and `RIG_LOGGED_OUT` point to staff and `HOPPER_LOADED_AWAY` says "staff must empty it". Also not built:
a staff ring clear (O-22), clearing another pilot's plan, and a remedy for a queued unload with no plan whose
operator lost the helm (it retries forever) (doc 20a:549; `docs/ops.md:1144`).

**DF-2 — player features owned by no row:** the licence's "4 d left" on `+chart`'s site line; the desk's `look`
listing the `+tv` verbs; a quarry-screen key for `+tv/plan show`; gifted or bought readings (`Reported`); clients
rendering `site.claims` beyond the grid's `#`; SM.3a's R1 at-dock term settle, R2 a graduate's refit or lease, R3 a
recalled Dray-P re-lent, R5 Occator's counter room, R8 a second rig lessor.

**DF-3 — reserved phases** (doc 20 §13's phase 2 and §17): Phobos as a second jurisdiction; survey trade;
landing anywhere; the crewed cab (Q-20-4); org-held claims and plans; Occator's expansion onto J and K and operator
agreements; asteroid fields, drone bays and drones; doc 17's `mining` role; the fight.

**DF-4 — engine work measured and left:** the plan path's double A* (about 6,600 plan-drive landings a 30-TV month);
a committed cost-lane baseline and delta step (D-H-8); the off-helm `mine_stop` refusal; the orphaned plan at a
lease's end; `hold_figures`' single stowage tag; a manifest-only relay re-latch for kept worlds; the optional lane
v16 owner's-numbers cut.

## 6. Known defects and risks

**RK-1 — defects worth a small cut:**
- `crates/mush-market/src/trade.rs:812` maps any `accept_contract` error, an insufficient balance included, to
  "That purchase needs an approval your account rule has not granted yet." (doc 20:2823-2825).
- Doc 18's planner prices the passage as laden at departure: a mining captain who bunkers to the planner's line is a
  crew-day short for the way home (SM.8; doc 20:2946-2953).
- A lapsed pilot's past-term rig is neither re-debited nor forfeited until they relicense (SM.5 decision 21 with
  SM.3a's unbuilt R1).
- `mush-bot`'s `bid_for` takes the first wanted row of the ore whether or not it is buying (SM.12c-bot F7/T11).
- `mush-atlas`'s `octile_floor` lacks the `.min()` that caps `Cost::MAX` saturation (AP).
- SM.5's R5: the port applier and the drip still disagree in two arms, and `wanted_spec`'s doc is stale; the owed
  `drip_zero_warning` world test was never written.

**RK-2 — risks the record accepts:** the ops rule "staff must never `@account/unfreeze` a Frozen entitlement with
no record" has no guard (doc 20:1753); a kept world's relay latch stays at its old version, which a future
"latched requirements consistent" check would read as skew; a mill row removed and re-added makes drift page until a
second withdraw; back-to-back Draw accepts can defer a restock indefinitely; `choose_route` lets one route's refusal
sink the others; boot installs the shipped geology tables on a world latched lower; a world that latched lane v10
before SM.3b reads its site lines Missing (harmless pre-launch); a rollback past SM.12c-lane cannot decode a hopper
ring carrying `keep_s`.

**RK-3 — the economy's size, from the month:** at 30 TVs the open ground the plans can file over runs out within
an IC month, metal first; the drones alone supply about 0.74 real days of the draw; the Exchange's intakes close on
capital in about a real month, mill or not (O-1's consequence); every economy number is provisional (OD-3).

## 7. CI, tests and operations

- **CI-1 — suites over the 90 s debug budget:** `drones` about 182 s, `cost_lane` 105–175 s; the canary adds about
  9 min to the bot shard; `mining` about 60 s, `knowing` about 45 s, SM.2's landing sweep about 34 s.
- **CI-2 — figures refreshed by hand only:** the month and the bot cohort run by hand (O-28 (i)); the scheduled
  stanzas exist as text; a weekly run needs a day-of-week guard now that the house cron is daily.
- **CI-3 — load flakes seen at the train's gates:** `boot_parity`'s `stop_ledger_archive_joins_the_pass_rather_than_detaching_it`,
  `server_panic_relaunch`, `observer_rule`, `arena_story`, `module_api` (each passed alone).
- **CI-4 — test gaps the notes leave open** (the larger ones): no end-to-end test takes the published lane v13
  through `@atlas/pack` to the shipped lane with the mills and the yard working; the mill's prepare `Unauthorized`
  and `Integrity` arms; the yard's licence-refusal branch and preflight sentence; `RECOVER_FAR`, `RECOVER_DARK` and
  `NO_WAY_HOME` in a world; the month's contest check is vacuous at 30 TVs; SM.1's T7/T9/T10; SM.3a's and SM.3b's
  listed gaps; the server's `rigs[].leg` gate.
- **CI-5 — operations:** no deploy or rate-24 smoke of the train; the relay track's live-gate scripts still expect
  lane @13 and relay @2; `cargo fmt --all` exceeds the Windows command line on the build box (os error 206; CI is the
  real check); the build box's disk filled during SM.12c-lane's gate because the daily target prune skips while a
  long-running `cargo run` is up.

## 8. Design-document text owed outside doc 20

Doc 20 itself is fully folded. Doc 20 §18 still owes text to other documents: **doc 10** §5 (the surface arm of the
loop); **doc 10b** §3.3, §15, §15.5 (`tacsim.tv.deposit`); **doc 11** §4.1, §4.12, §5, §6, §7 (the Syndicate,
Occator's claims and yard, the ore intakes); **doc 14** (region placement, the `i16` rule, the grid caps, P10);
**doc 16** §12; **doc 17** §12, §22; **doc 18** (the SM.2, SM.3a, SM.5 and SM.6 items, D-18-24's fixed reference,
Q-18-22 for `await tv` and the house's re-arm, the planner's cargo mass); **doc 19** §3 (`+chart/ground`); and
README rows for 09a–09c, 11-tacsim-setting, 16c, 17e, 17f and 18-cut-notes. Doc 20a's body is the plan as written
by design: its O-9 blanks and its 2026-10-24 snapshot stand, and the later rulings live in doc 20 §15.

## 9. Suggested order, if the owner wants a tail

1. **Rule**: OD-1 (the contract cap), SM.10's (b), the passage line, the geology-key backups; then accept or amend
   the lead defaults of OD-4 as a body.
2. **A small fix cut**: `trade.rs:812`'s refusal, the staff close/home verb with a hopper empty at a port (DF-1),
   doc 18's planner cargo mass, `bid_for`, the octile cap.
3. **A deploy and a rate-24 smoke** on a dev box, with the owner, including the real lane v13 → shipped upgrade.
4. **CI**: schedule the month and cohort or accept their staleness; split `drones` and `cost_lane` to their budgets.
5. **The §18 fold** into docs 10, 10b, 11, 14, 16, 17, 18 and 19.
