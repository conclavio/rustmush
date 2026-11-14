# 44. Minds, wired through the game — cut notes

One section a unit of [doc 44](44-minds-wired-through-the-game.md) §7. A unit's session replaces its
own "Not built." line with what it built, under these headings: **Built**, **Tests**,
**Divergences**, **Not built**, **Not pinned by a story**, and the open Q-44 defaults it was built
on. Each session edits only its own section, so two units' notes never touch the same lines.

## MW.0

Licensed minds composed on a live server (doc 44 §7.1, Q-44-39; second review, §4c row 1). Built
with MW.22 below as one change: the owner ruled that neither reaches main without the other.

**Built.**

- **Every Active launch composes the claim path.** `tacsim::launch::LaunchComposition::prepare`
  (`crates/tacsim/src/launch.rs`) now ends its Active branch with
  `with_ic_minds_in_bays(Vec::new())`, after the House chain and the player market, for a
  format-1 and a format-2 deployment alike. An `Initialize` launch composes none of it, as
  before. So on a production boot `claim_path_composed` is true once the launch is Active: a
  licensed `+tv/<n> launch mind=filed:<n> go` opens its claim in the bay before the ramp, the
  field's `claim_seal` finds the claim and admits the rig under its own seal, a licence's seats
  are counted at `go`, and the reader refuses an Open while a resale is pending.
- **One function, shared by construction.** `mush_server::boot` already called `prepare` and
  nothing after it. The switch the rigs alone could reach,
  `LaunchComposition::with_ic_minds_in_bays`, is deleted, and the M8 world's `factory`
  (`crates/tacsim/tests/domain_field/m8_world.rs`) calls `prepare` and nothing after it. No
  world can now compose a launch without the claim path or with something production lacks, so
  every M8 story runs on the production composition. `m8_world_with_claims` and `factory_for`
  are gone; `m8_world_field` remains as the M8 world on a chosen site.
- **Nothing else had to be composed with it.** Doc 17g §4 deferred "IC claims and hull cover in
  live play" for want of an IC region host. The host exists since doc 23's S.4b (the surface
  tick's claim arm and the IC field). `with_ic_minds_in_bays` asks only for an Active commerce.
  Hull cover asks for the claim path, not the other way about, and stays uncomposed.
- **The shared story helpers follow the claim.** `m8_world::launch` and `recover` pump the
  claim arm where a launch opened a claim, so doc 17h's stories read as before.
- **A rig's second claim in one boot closes** (found on the CI dispatch once doc 17h's tier-2
  story ran with the claim path composed; a defect of doc 23's S.4b-2, live as soon as the
  path is). The surface tick's claim arm (`surface/mind_claim.rs`) kept each rig's Open and
  Close tickets in memory keyed by the rig alone, and never visited a rig again once its Close
  had landed, so the spent Close ticket stayed. The rig's next claim in the same boot then
  resubmitted that ticket for its own Close, which the owners do not offer: the claim never
  closed, its licence's seat stayed held and the rig could not fly a licensed mind again until
  a restart. Each ticket is now kept with the claim it is for (an Open with its epoch's
  instant, a Close with the claim's open token), a ticket kept for another claim is
  acknowledged and replaced, and a landed Close gets one more visit that releases it.

**Tests.** Fast, on the production composition (`domain_field::m8_world`):
`m8g_the_combined_world_boots_active` now asserts the claim path is composed, and every M8
world asserts, at its own `Initialize` boot, that none is composed there. **The server's own
boot shows it too**: `mush-server`'s player-market tests boot the server's module list from a
launch document (`game_modules` with `[tacsim.minds].launch_path`), and their pinned count of
read services and grants rose from 41/44 to 42/45, which is the claim path's admission reader
and TacSim's grant to it (`player_market_commands.rs`, `player_market_typed.rs`; the same pin
in `tacsim`'s `hardware::flow::launch` rose from 40/43 to 41/44; the caps are 64/64). The acceptance stories are `#[ignore]`d (each takes a genuine Registry rating) and run on
the CI dispatch: `s4b3_a_licensed_mind_flies_a_field_at_256` and `_at_512` (a filed mind on a
rig its pilot holds title to opens a claim, is admitted to a field under its own seal and card,
and the one seat refuses the second rig), and
`s4b2_a_licensed_rig_opens_its_claim_in_the_bay_and_closes_it_home`, rewritten for MW.22 (its
step 3 is the launch refused while the licence's resale is pending). Results are under MW.22.

**Divergences.**

- **A leased rig and a House hull are not refused.** The row's last clause ("a leased rig and a
  House-titled hull are refused by name") describes MW.0 alone. Under Q-44-39 it never reaches
  main alone, and MW.22 admits both. What is refused by name is a hull the pilot has no
  authority over (MW.22).
- **No change to `mush-server`'s code**, only a comment at the call: the function it calls
  changed.

**Not built.** Hull cover and paid Follow stay uncomposed (17g §4). The DT launch
(`+sortie/launch`) still takes no `mind=` word, so a detachment's flight opens no claim (doc 23
§17.9, 23-10). A cored ship opens no claim (D-23-12).

**Not pinned by a story.**

- **A changed mind changing what the rig does.** The field stories pin that the rig flies the
  claim's own seal, mind and card and not the ROM's, with no drop and no fault. No story flies
  two different filed minds on one rig and compares what it did; MW.7's filed-mind story is the
  first that flies a mind written to behave differently.
- **A rig launched on a world the server binary booted.** `mush-server`'s tests boot its own
  module list over an Active launch and count the claim path's reader, and the M8 world flies
  rigs on what `prepare` returns, but no one test does both. MW.1's bring-up is where a rig is
  first launched on a box.
- **A definition-set change at deploy.** The claim path adds its owners' definitions to an
  Active launch's manifest, so a store written by an earlier build does not boot under this
  one. The dev boxes are wiped at every step; nothing migrates.

## MW.1

The launch switch and the bring-up script (doc 44 §7.1, §8 steps 2 to 8; Q-44-3 to Q-44-8,
Q-44-41, Q-44-47, Q-44-48; §4c rows 3 to 7 and 19). Every question it rests on is ruled.

**Two things the owner should read first.**

1. **A rating is long at any pulse, so the seed line comes after go-live.** Q-44-41 and MW.6
   say a dev box at 50 ms rates a mind in minutes, from the acceptance stories' rig (a lease a
   third of a second, a certificate about six minutes). On a world brought up by the runbook,
   measured on conclav.dev (32 cores, release build): **a copy of Standard Doctrine rated in
   1,948 s, 32.5 minutes, at `registry_pulse_ms = 50`**. A lease took 1.27 s at 50 ms and 1.10 s
   at 10 ms, with the server at about 6 per cent of one core: below about a second the pulse is
   not what paces a rating. A named House source is longer: the Meridiani source rated in 4,757 s, 79 minutes, at 50 ms, and in 4,084 s at 10 ms. So the bring-up does not hold players off for the
   rating: the unit is started after the Active boot's checks, and the seed line and the House's
   purchase run on the running world.
2. **No House can be given hulls on a live world.** A House's first licence needs three titled
   hulls. The only spare hulls a fresh world has are the Ceres Co-op's eight pool Drays; three
   Houses need nine; and a Dray titled to a House closes `+coop/enrol` for every player
   (`port/hull.rs`: the Co-op's readiness wants every pool Dray with the Trust or a graduate).
   No staff verb makes a hull, and the yard commissions one only from goods delivered out of a
   docked hold. So the script **lends** three Drays to one House for the purchase and hands
   them back. Every House has its policy, agent, grants and funded treasury; none keeps a hull,
   so none buys again. Giving the Houses hulls of their own needs content (the lane pack titles
   them hulls) or a staff commission verb. Neither is in this unit's files; it is the follow-up
   this unit leaves.

**Built.**

- **`MINDS_LAUNCH` in `instance.env`** (`on` or `off`; empty is off; anything else refuses the
  render). `deploy/instance/render.sh` reads it itself: it is not a `__TOKEN__`, because what it
  decides is whether a commented line of the template is live at all. On, **and with the box's
  launch document at `<DATA_DIR>/launch.json`**, the `[tacsim.minds]` header and
  `launch_path = "<DATA_DIR>/launch.json"` are rendered live. On with no document, or off, the
  line stays the template's comment. So `launch_path` is out of the rendered file until the
  document exists, a deploy onto a fresh or wiped store boots without the launch whatever the
  switch says, and no state renders an empty path. dev1 and dev2 ship `MINDS_LAUNCH=on`
  (Q-44-3); the reference instance ships `off`.
- **`REGISTRY_PULSE_MS`**, read by `render.sh` the same way: not empty, it renders the header and
  `registry_pulse_ms = <n>` live. dev1 and dev2 ship 50 (Q-44-41, provisional); the reference
  instance ships it empty, which is the server's 1,000.
- **`RAID_DIRECTOR`**, an ordinary token, in a new `[tacsim.field]` block of the template with
  that one key (the block's other keys keep their compiled defaults). dev1 and dev2 ship `true`
  (Q-44-7); the reference instance `false`. `mushd.toml.example` gains the block.
- **`[llm.routes.content_assist]`** in the template and the example: `z-ai/glm-5.3`, 2,048
  tokens (the pass asks for no more), 60 s, reasoning low. Provisional; it is the class the other
  structured authoring routes use.
- **`scripts/reset-instance.sh`** comments a live `launch_path` in the rendered config when it
  wipes the world, after the wipe and before `--bootstrap-staff` and the start, both of which a
  live path with no file refuses. It is the one line of the rendered config a reset touches.
  `--no-start` leaves the unit stopped after the wipe.
- **`SERVER_HOLD=1`** in `deploy/dev1-split/deploy.sh` and `deploy/dev2-split/deploy.sh`: a
  server unit that is not running is installed and enabled and not started.
- **`deploy/instance/launch-bringup.py`** (Python 3, standard library; root on a box). It runs
  the world itself for three boots, on a private IPC socket with a private gateway on
  `127.0.0.1:<SSH_PORT + 20000>`, and types as #1 over SSH at that gateway's welcome screen:
  1. the plain boot: the Republic, the three system accounts, the three Houses, the Basin
     importer and its fitting shop, the Tether back room, the Registry, six service logins, and
     each House's procurement agent, grants, faction membership and funded root-scope treasury;
     then `launch.json` in mode `initialize`, with `hardware.ungovern` and three `procurement`
     entries, and a render;
  2. the Initialize boot: the seven Registry grants on the review scope to every operator, the
     four initialize actions, the document's mode set to `active`;
  3. the Active boot: `@mind/launch` must name every operator `ready`; the 18 major NPCs
     enabled with `@agent/enable`, read from the cast packs in the tree (not by editing them);
  4. the unit started, `@mind/launch` read again on it, and the LLM check;
  5. on the running world: one line imported or copied, saved, rated, filed by #1 and listed
     on the player market; three Drays lent to one House; the unit restarted (a House's month
     that found an empty counter stays closed until a restart); the House's purchase read from
     its procurement ledger in the store; the Drays handed back. `--seed-only` runs this step
     alone and takes a stopped one up; `--no-seed` leaves it out.
  A second run on a box that is already live stops nothing.
- **The LLM check** (§8 step 8): #1 asks a major NPC a question and starts a simulator match
  against an AI squad, then reads `@llm/log`. It passes only on an `ok` row for an
  `npc_dialogue_*` route, an `ok` row for `tacsim_deliberate`, and an `@agent/show` that says the
  NPC's last turn was the model's. `@llm/budget` and `@llm/dashboard` are written to
  `<DATA_DIR>/bringup/llm-check.txt` (Q-44-48). `--llm-check` runs it alone.
- **`scripts/launch-bringup-acceptance.sh`**: the proof below as one script, on a scratch
  instance with its own Redis, driving the real `reset-instance.sh` and the real bring-up as
  root over a `systemctl` shim. `scripts/tests/render-launch.sh` (in CI's script step) holds the
  switch against the shipped template; `scripts/tests/reset-instance.sh` gains the launch line
  and `--no-start`; `scripts/tests/llm-stub.py` answers the LLM routes for the acceptance run.
- `deploy/README.md` describes the switch, the script, how its steps are typed as #1, how
  players are kept off, what a rating costs, and what a House's purchase needs.

**How players are kept off.** The instance's world unit is stopped before the first boot and
started only after the Active boot's checks. With it stopped nothing answers on the instance's
IPC socket, so the public gateway generations have no world to log in to or register on; the
private gateway is on loopback. A stop on a refusal, or an interruption, leaves the unit
stopped and the private boots stopped. Fresh store to players let in is about two minutes.

**What the proof found that the runbook did not say**, each handled by the script and written
into the README:

- **The Roll's initialize action must be typed last.** Once `roll-initialize-commerce` has landed
  the world boots only Active: the Initialize boot's preflight refuses
  (`commerce initialization preflight: commercial records disagree`). The runbook and the tests
  type it first. A bring-up interrupted between it and the three industry actions left a store
  that booted in neither mode. The script types the three industry actions first, and treats
  that refusal of the Initialize boot as "all four landed". The README's step 10 now says so.
- **A typed line is flood-limited for #1 too** (ten a second, thirty in a burst), and the world
  answers "busy" while the last action commits. The script paces itself and types a
  turned-away line again.
- **A boot more than 30 s after the store's last clock checkpoint runs the five-minute
  post-outage countdown**, with every world action refused. A second run of the bring-up after a
  pause meets it. The script waits it out and says so.
- **A House's month that found nothing stays closed until a restart** (the flag is in memory),
  and its first look is at an empty counter. Hence the restart in step 5.
- **`@account/list` stops at twenty rows with no cursor**, so accounts are found by
  `@account/show` one id at a time.
- **A backgrounded shell hands a child SIGINT ignored**, and a closed terminal sends SIGHUP: the
  script sets its own handlers for INT, TERM and HUP so its private boots are always stopped,
  writes their pids down, and stops any a killed run left behind.

**Tests.** The proof is `scripts/launch-bringup-acceptance.sh`, run as root on conclav.dev
(Ubuntu 26.04, Redis 8.10 on a spare port, a release build of `mush-server` and `mush-gateway`
at this branch), each run on its own scratch instance. Never on dev1 or dev2.

- **The fast steps** (`SEED=0`), passed in five minutes:
  - run 1, a fresh store, interrupted by SIGINT 40 s in. It was inside the Initialize boot,
    with the first initialize action landed and three to go. Four attempts to register at the
    public front door during it were all kept out, and no private boot was left running;
  - run 2, the same store, to the end: it took up at the Initialize boot, landed the other
    three actions, and `@mind/launch` read `Registry operators: #810 ready.` in the Active boot
    and again on the started unit. Probes were kept out until the unit started, and one
    registered once it had;
  - the LLM check, with no provider answering, failed as it must;
  - the real `reset-instance.sh --staff keeper --no-start`: the rendered config read
    `# launch_path`, and the data directory held no document;
  - run 3, the second bring-up on the reset store: plain, Initialize and Active boots (1.0 s,
    1.5 s, 1.5 s to come up), `ready`, five probes kept out;
  - `scripts/tests/render-launch.sh`: off, on without and with the document, the pulse, the
    refusals, and the plain substitution byte for byte.
- **The LLM check's passing path** (`LLM=stub`, the providers pointed at
  `scripts/tests/llm-stub.py`): `npc_dialogue_major ok` for the first major NPC,
  `@agent/show` "the model answered and did nothing", and `tacsim_deliberate ok` from a
  simulator match #1 started and boarded against an AI squad. With no provider the same check
  reads `disabled` or `error` rows and fails.
- **The purchase, in the script's final shape** (the whole acceptance, the long step last, on
  the second bring-up's running world): twice. At the defaults (50 ms): `Seedline@1`
  rated in 1,949.2 s; listed as `59809cd4  Seedline Market  sealed  class I  400cr  90+28
  days`; Meridiani lent Slack Water, Cold Comfort and Ninth Ledger; the unit restarted;
  `Meridiani's agent #812 bought licence 2 of charter 828 for 40000 minor credits (15s after
  the boot); the root treasury holds 160000 minor`, read from the procurement ledger; the three
  Drays handed back, and `@tacsim/port status` read every programme's hulls ready;
  `acceptance: PASSED`. At 10 ms the same, with the rating at 1,666.1 s.
- **The purchase of a House source** (`--seed-line meridiani`, 50 ms): the Meridiani source
  rated in 4,757.2 s and was listed as `a92edfa1  Meridiani Field Series M  sealed  class II
  400cr  90+28 days`; Meridiani's agent bought it 15 s after the restart. This run was
  stopped and taken up with `--seed-only` three times on the way (a name the holdings list
  cuts at twenty characters, which the script now knows by its start; an older rig's shim), and
  each time went on from where the line stood.
- **After the hand-back**, on that running world, `@tacsim/port status` read `ceres_port hulls:
  8 of 8 made, titled to Co-op First Captain Trust and laid up. [ready]`. The script now reads
  that line itself and stops if any programme's hulls are not ready.
- **Ratings timed** (queued to finished, release build, conclav.dev): a copy of Standard
  Doctrine 1,948.3 s and 1,949.2 s at 50 ms, 1,666.1 s at 10 ms; the Meridiani source 4,757.2 s
  at 50 ms and 4,084.3 s at 10 ms.
- An earlier shape of the script, with the seed step inside the Active boot, kept 184 attempts
  to register out over the 37 minutes of its rating.
- Locally: `cargo test -p mush-config -j 2` (the new
  `mw1_the_launch_switch_renders_the_path_only_beside_its_document` among the 15 of
  `shipped_configs`) and `cargo test -p mush-contract-tests -j 2` green; `cargo fmt -p
  mush-config --check` clean. On Linux: `scripts/tests/reset-instance.sh` (the launch line,
  `--no-start`) and `scripts/tests/render-launch.sh` pass.

**Divergences.**

- **The seed line and the purchase come after the unit is started**, not in the Active boot
  (first paragraph). The unit's row asks that no player can log in before the Active boot; that
  holds. It does not hold players off for the hours a rating takes.
- **The Houses are lent hulls, not given them** (second paragraph). `--keep-house-hulls` leaves
  them, with enrolment closed.
- **Byte for byte, precisely.** With `MINDS_LAUNCH` off and `REGISTRY_PULSE_MS` empty the render
  is the plain token substitution of the template, byte for byte, and both the shell test and
  `shipped_configs.rs` assert it. Against main's render of the same env the file differs by the
  template's own additions: the `[tacsim.field]` block, the `content_assist` route, and comment
  lines about the switch. No live line that main rendered is changed.
- **The service characters each have a login of their own**, where the runbook hangs the clerk
  and the operator off `LaunchAgent`'s login with `@newchar`. One `create` makes login and
  character together, so no run stops with a login half made, and no password is kept: each is
  random and thrown away. To play one, #1 types `@newpassword <name>=<password>`. (The first
  draft kept the passwords in a state file; CodeQL flagged it, rightly.)
- **The seed line is filed personally by #1**: by default charter `Seedline Market`, a copy of
  Standard Doctrine named `Seedline` (`--seed-line standard`: the quickest to rate, and a House
  buys it). `--seed-line meridiani`, `coprates` or `safe_conduct` imports that named source
  instead, as the zero-player story does. Whether a House filing of the same revision is still
  possible after #1 has filed it personally was not tried.
- **Procurement policies name the Republic as `standing`** and a budget of 200,000 a month
  (the config's default, written into the document so the grant's limit and the policy agree).
  Each root-scope treasury is funded with one month's budget from `launch.faucet`.
- `hardware.ungovern` carries no `fee_minor`: the catalogue's fee applies.
- `deploy/instance/launch.json.example` is unchanged: it is the format-2 document two tests
  read, and the dev launch's document is the one the script writes.

**Defaults that would be costly to undo.** None. The switch's shape (a key `render.sh` reads,
the document's fixed place in the data directory) is the one other units will lean on; changing
it is an edit to `render.sh`, the contract test and three env files.

**Not built.**

- Hulls for the Houses (above).
- Anything that makes a rating shorter. Below a second the pulse is not what paces it.
- The House chain's own filings and counter listings of the three named sources. The runbook
  does not list them, and procurement does not read the House counter.
- A staff way to end the post-outage countdown early. A re-run after a pause waits five minutes.
- The acceptance script is not in CI: it needs root and a Redis with the JSON module, and its
  last step is hours. MW.19 stands the bring-up up in the bot's rig (`--no-systemd`).
- Nothing was run on dev1 or dev2. MW.3 is the go-live.

**Not pinned by a story.**

- The LLM check against a real provider. The acceptance run proves it fails with no provider
  and passes against a local stub that answers every request with one line; what a real model's
  `ok` row and a real fallback look like on a box is MW.3's to see.
- Co-op enrolment after the Drays go back: the port's readiness report reads the pool's hulls
  ready again, and the script checks that line, but no player was enrolled afterwards in the
  proof.
- A House's purchase of the same line a second time, or by the other two Houses: one House,
  one licence.
- The all-in-one path (`mushd` alone, where the split is not built) and the two older unit
  shapes: written, not run.
- The script against a store that already has players.

## MW.2

The two market defects ruled to be fixed before the go-live (doc 44 §7.1; Q-44-14, Q-44-15;
second review, §4c rows 8 and 9; doc 17g §7). Every Q-44 question it rests on is ruled.

**Built.**

- **A read the Roll owns says whether a listing can still be sold.**
  `crates/mush-roll/src/commerce/withdrawal/sellable.rs`, reached only as
  `EntitlementPurchaseWriter::sellable` (the writer the market already holds). Given what a
  listing froze of its seller's standing (`ListedSubject`), it answers `Sellable::Yes` or
  `No(Unsellable)` for a batch of up to 32 listings:
  - `Withdrawn`, `Recalled`: the listed revision's status row (recall is told first).
  - `SellerLicence`: the seller's owner licence is past Grace, has moved to another term, is
    revoked, or its holder is barred by an inspection for now.
  - `ParentLicence` (a derived listing): the seller's qualifying parent licence (the one the
    child was filed under, or a committed replacement) is not theirs, is past Grace, is not an
    open-resale term, or is revoked; or the parent source is recalled.
  - `CharterMoved`: the charter's title is not the seller's at the listed title revision, or the
    seller is no live character.
  - `Unproved`: a row the answer needs is missing or does not match. The market treats it as
    unsellable.
  It reads at most 4 rows for an original listing and 12 for a derived one, in at most 12
  batched reads a call. The rows are read through the market's own read lens: `install` adds
  seven read-only family grants to `market` (`status`, `licences`, `revoked_terms`,
  `operations`, `owner_parents`, `owner_qualification_heads`, `owner_qualification_operations`).
  The market holds no handle to any of them.
- **The counter and the House agent's list ask it.** In `crates/mush-market/src/mind_commands.rs`
  "current" is now two tests, the policy (`current_policy`) and then the Roll's answer, for
  `Listings`, `ListingsAfter`, `ListingByCode` and `ProcurementListings` (which now shares
  `Listings`' page loop, `page`). A dead listing leaves `+minds`, the `mind.market` panel and
  the agent's list; its code is no longer found (`+minds <code>`, `+minds/buy <code>`); it still
  serves as a `MORE` cursor, as a policy-stale listing does. The read's descriptor
  `mind_market_commands` is version 3.
- **The Roll's answers are paid for inside the read's own allowance.** Core gives an owner read
  256 reads and 512 rows and fails the whole read past either; the listing scans alone could
  spend every row. The page loop counts each scan and each listing asked about (`Budget`) and,
  when what is left cannot pay for the next, ends the page with the last listing it decided as
  the cursor. A page that meets a long run of dead listings can therefore be short, or empty,
  and still say `MORE`; following the cursor reaches what lies behind them.
- **The Roll's renewal readers carry a lasting-refusal verdict.** `RenewalStop` in
  `crates/mush-roll/src/commerce/renewal/reader.rs`, used by `house_renewal/reader.rs` as well:
  `Recalled`, `Withdrawn` (the pinned revision's status row; one more row a licence) and, for a
  personal licence, `OwnCharter` (the holder now owns the charter it is sold under, so the
  holder would be buyer and seller at once; from the economy's title, no row). It is derived at
  every read and never stored. `Inventory` leaves a stopped licence out; `Status` answers it.
- **Both renewal services stop on it.** In `mind_renewal/service.rs` and
  `mind_house_renewal/service.rs`: the scan admits no stopped licence; a draft entry admitted
  before the stop arose ends at its next turn; the player's `request`, `accept` and `retry`
  answer the lasting line and submit nothing (a decline of an open offer still goes through; a
  funded offer still runs to its grant or refund). Nothing in the service's memory decides it,
  so the next scan and a restart's first scan are told the same by the Roll.
- **`+minds/renew` says why**, in the ruled words (`player::RENEW_STOPPED`): "This licence cannot
  be renewed: <reason>. Nothing further will be tried." The reasons are "the Registry recalled
  this mind", "its author withdrew this mind" and "you now own this mind's charter".

**Tests.**

- `mush-roll`, `commerce::withdrawal::sellable::tests` (six): each of the four ways a listing
  dies and `Unproved`, the Grace boundary, the parent choice (bound licence, replacement, a
  later owner with none), and the declared row costs. `commerce::renewal::reader::service_tests`
  (two new): the three lasting causes and their order; an entry that is not stopped serialises
  as before.
- `mush-market`, `mind_commands::tests` (four new): a listing the Roll calls unsellable leaves
  `Listings`, `ListingByCode` and `ProcurementListings`, is never asked about when the policy
  already dropped it, and still serves as a cursor; a dead row does not count toward a code's
  ambiguity; over 200 dead derived listings no read spends more than core's allowance, every
  cursor moves forward and the one live listing behind them is reached; a full page followed
  only by dead listings is the last page. `mind_renewal::tests` (one new): the ruled sentence
  for each cause, exhaustively, and the House type is the same type.
- The two stories are rewritten (`player_market_story.rs`, `player_market_typed.rs`). The typed
  story: after the typed `Withdraw` the `mind.market` panel has no rows and the empty counter's
  notice. The command story: both listings read on the counter within the owner term's Grace;
  after `@mind/attest` they are gone and the code is not found (see Divergences); after
  `@mind/withdraw` the counter is still empty, no quote row is added, and `+minds/renew` answers
  the withdrawn line with no renewal operation opened; after the recall and the restart it
  answers the recalled line, again with none opened. Both run on the CI dispatch only.

**Divergences.**

- **An owner's attestation takes that owner's listings off the counter.** A listing freezes the
  owner term it was made under, and a quote has always refused a listing whose frozen subject
  differs from the Roll's current facts (`mind_entitlement::offers::current_listing`). So once
  `@mind/attest` opens the owner's next term, every listing made under the earlier term could
  already not be quoted; it used to stay on the counter and now leaves it, and the author has to
  list again. The same holds for any change of the charter's title revision. The command story
  pins it. Whether a listing should survive its owner's attestation is a rule of the quote, not
  of this unit, and is the owner's to decide.
- **"Expired" is not a terminal state here.** The brief named recalled, withdrawn and expired.
  The renewal protocol renews an expired predecessor on purpose (the reader's descriptor says
  `expired_predecessor_permitted`), there is no renewal deadline anywhere in the Roll, and the
  service only begins once a term has ended. Leaving lapsed licences out would turn the service
  off. `OwnCharter` is added instead: it is the second cause doc 17g §7 recorded for the fault.
- **A lasting refusal is known before a draft is sent, not learned from one.** A refused draft
  tells the service nothing (the provider answers one uniform refusal and the observation is
  `NoIntent` with no payload), so the lasting causes are read from the Roll's rows. The service
  therefore tries a recalled licence not once but never; an entry it already held ends at its
  next turn.
- **The reason phrases are a default.** The sentence is the owner's ruling; the three phrases
  that fill "<the Registry's reason>" are this unit's wording.
- **The counter's answer is slimmer than a quote's.** It proves no ancestry, root, rating or
  royalty evidence, so a listing it shows can still refuse a quote (a re-conformed rating, a
  product the policy now computes differently, a buyer-specific refusal).
- **Fingerprints move.** The seven read grants enter the state-family registry's fingerprint and
  the descriptor's version enters the mechanics registry's. Neither is pinned in the tree;
  `tacsim/src/hardware/flow/launch.rs`' check of the read's version is updated to 3.

Two of these would take real work to undo: the budgeted page loop (the old loop is gone, and
`ProcurementListings` now shares the new one), and the choice to read the Roll inside the
market's read rather than from the caller, which is why the answer has to be slim.

**Not built.**

- No back-off for a passing refusal. A draft that refuses for a cause that is not one of the
  three (a derivative waiting on its parent author's renewal, a short wallet, a pending resale,
  an officer without the grant, and also any fault this unit does not classify: a charter an
  organisation holds, a retired House purchase template, an integrity mismatch) is still retried
  once a real second, as before, and the holder still reads the uniform line.
- The House agent keeps no memory of a dead listing it had already quoted; it is simply not
  handed the listing again, and its open quote expires as any quote does.
- No change to the quote path, to `+minds/holdings`, or to tacsim's commands.

**Not pinned by a story.**

- No engine-level test drives the renewal service over a recalled or withdrawn revision; the
  command story on the CI dispatch is the only run that does, and only for the personal service.
  The House service's stop has no story at all.
- "A derivative waiting on its parent's renewal is still retried" holds by construction (it has
  no `RenewalStop`, and the retry path is untouched) and has no test of its own.
- `ParentLicence`, `SellerLicence` by revocation or bar, and `CharterMoved` are pinned by the
  Roll's unit tests on the decision alone; no story lists a derived mind and then lapses its
  parent.
- The House agent's list is pinned over the in-memory listing family; the procurement stories
  (`m8_world`) run it over real rows without a dead listing among them.

## MW.3

Not built.

## MW.4

Docs only (2026-11-11). Each ruling is written where its question was first asked, as "Ruled
2026-11-11 (doc 44 Q-44-n)"; no question was deleted and no surrounding text rewritten.

- `17-unit-minds.md`: D-UM-56 (Q-44-9); Q-UM-12, Q-UM-14, Q-UM-18 (Q-44-10).
- `17g-m7-completion-plan.md`: §3 C4 (Q-44-15); §4's paid Follow and §5 item 4 (Q-44-11); §5
  item 3 (Q-44-5); §5 item 5 (Q-44-9, Q-44-10); §7's renewal-retry finding (Q-44-14).
- `17h-m8-contraband-procurement-plan.md`: the heirloom (Q-44-12); the House term (Q-44-13).
- `36-ground-combat-after-the-field.md`: Q-36-41 (Q-44-7); Q-36-43 (Q-44-21).
- `03-llm-integration.md`: the status line, from `LlmFeature` and its callers (§4 G), with Q-44-29
  and Q-44-6.
- `27-ic-field-operations-design-and-survey.md`: Q-H5 (Q-44-35 to Q-44-38).
- `42b-surrender.md`: the yielded hull that keeps firing on a busy exit (§4c row 22), checked in
  `field/raiders.rs`.
- `23-ic-sorties.md`: §14 S.2's owed list, a wrecked ship's epoch (§4c row 26), checked in
  `port/epoch.rs`.
- `README.md`: the 17g row.

## MW.5

Doc 17g's C4 acceptance (doc 44 §7.1; §4b row 7; Q-44-15). Started 2026-11-12 on main
`9d8fc96b7`. This section opens with the inventory the unit was asked to start from, checked
against the code and not against docs 17f and 17g; what is built follows it.

### Inventory (2026-11-12, at `9d8fc96b7`)

**What "the production composition" is.** `mush_server::boot::game_modules`
(`crates/mush-server/src/boot.rs`) reads `[tacsim.minds].launch_path`, decodes the deployment,
calls `tacsim::launch::LaunchComposition::prepare` and hands the result to
`modules::server_modules_with_launch`. There is no function named `compose_launch`; the brief
and two notes use that name for this path. Three test rigs stand a launch up through `prepare`:

| Rig | Where | Roster under the launch | Restart | A genuine rating |
|---|---|---|---|---|
| **P1**, the named launch | `crates/tacsim/src/hardware/flow/launch.rs` (`modules`, `reboot`) | tacsim's `domain_modules_with_launch` | `Rig::restart` over the kept store | about 55 min a story |
| **P2**, the server | the rig is `crates/mush-server/src/world_task/submission_disposition/tests/player_market.rs` (`launch_world`, `initialized_launch_with`, `launch_active`) over `Running::boot(config, store, None)`; its stories are `player_market_story.rs` and `player_market_typed.rs` beside it | **the server's own**: `game_modules`, `server_modules_with_launch`, the world task and the persist path | `server.stop()`, then `active(..)` over the same store | about 35 min at `MIND_PULSE_MS` unset |
| **P3**, the combined M8 world | `crates/tacsim/tests/domain_field/m8_world.rs` (`factory`, `m8_world*`, `M8World::reboot`) | a test twin (`excursions::Roster::launch_modules`: `domain_modules_with_launch` over the test world's layer with the shipped content) | `reboot_with` over the rig's store | about 8 to 25 min a rating (CPU-bound; no wall-clock pulse) |

Only P2 runs the server's roster. P1 and P3 run `prepare` under tacsim's own domain roster, so a
story on them proves the launch composition and not the server's boot around it. The rigs that
do **not** count: every `minds/bulk/*_integration_tests` fixture (`PurchaseFixture`,
`PlayerWorld`, `IcWorld`: hand-built `MindCommerce::prepare` chains), `paid_purchase` in
`mush-server` (`game_modules_with_mind_commerce`), `player_market.rs`'s `player_market_commerce`
chain, the hardware flow's own rig (`hardware/flow.rs`: `GovernedHardware` with no launch) and
the excursions' `reboot_with_inspection`.

**Item 1: 17f §9's stories 1, 2, 3, 5, 6 and 7, each with a restart.**

| Story | On the production composition today | Not there |
|---|---|---|
| **1, commercial author** | P2, `player_market_commands_story_two_accounts_alt_stale_tiny_and_restart` (dispatch): A files by command over a genuine rating, publishes sealed, B on another login buys at the counter and reads it in `+minds/holdings`, B resells with one balanced split; three restarts. The typed story repeats quote, pay, complete and the panels on the typed surfaces, with a restart | B selecting, sealing and flying a **player-authored** mind (P3 flies House Pins only: `m8g_story_tier2_held_minds_seal_aboard`, `s4b3_*`); cards and trace readable while source and memory never are, on every surface (`assert_private` checks digests and account ids only); the OOC run posting nothing; the old holding refused at the next seal **after a restart** (tier 2's step 3 refuses it, with no restart and no split check) |
| **2, private and derivative** | P2 only as far as a draft: an open listing bought, `@mind/fork`, one `roll_mind_draft_parent` row | every other clause: a fork filed privately and refused a listing; an open-resale derivative paying its parent chain; parent lapse, transfer, revocation and recall sealing the dependent; restoration; an existing customer keeping their term. All of it is proven only on `PlayerWorld` and `IcWorld` fixtures (`dependent_owner_integration_tests.rs`, `authored_fork_integration_tests.rs`, `paid_resale*_integration_tests.rs`) |
| **3, seat and term** | P3, `s4b2_a_licensed_rig_opens_its_claim_in_the_bay_and_closes_it_home` (dispatch): a one-seat Pin's live claim, the second rig refused the seat, a resale pending while the claim is open and the seller refused by name, the close, a restart with a House claim open. P2's part 3: the renewal after the term, revocation, recall, a charter transfer. P3 tier 2 step 4: the lapse fallback heard by the operator alone | **a two-seat licence cannot be made on a launch**: `@mind/publish .. seats=2` answers the provisional-terms refusal (`market_commands/market.rs`, `seats == 1`) and the House Pin is one seat (`launch/content.rs`). So "two-seat licence, one live claim" has no production form. Also absent: the pending sale settling at the last close and at the deadline, once; the buyer refused while pending; a duplicate close; a refund on revocation asserted; a free owner's parallel dispatches; a title transfer with a claim open; a restart **during** the pending sale |
| **5, authority and adversarial** | P2: the alt pair at the counter, a foreign target reading as a missing one, stale terms, an org form without the holder grant (`player_market_commands.rs`, default sweep, no restart), a rating queued and cancelled (no restart). P1: the resale privacy fence | same-account licence and charter **transfers**, the missing account oracle, forged receipt, grade and lineage, the generic-title bypass, malformed retained records and queue saturation are each proven on a fixture only |
| **6, launch catalogue** | P1, `named_coprates_real_save_rate_house_sale_and_membership_fences` (dispatch): three named Houses' sources rated, filed, listed and sold, the treasury's filing fee, Coprates' membership fence lost after the plan, after the stage and at the take; four restarts. P3: the same three lines in the combined world, and the procurement budget (`m8g_story_zero_pc_procurement_*`) | Safe Conduct's role run (`@conduct/run`) on a launch: the Conduct owner is composed (`with_conduct_trials`) and no launch test drives it. **Hull cover is not composed by a launch at all** (`prepare` calls neither `with_hull_coverage` nor `with_qualified_hull_coverage`; doc 17g §4 says "still deferred", correctly), so the priced discount, the premium, exposure, claim and wreck-title records and the zero-charge refusal cannot run on the production composition. Free Doctrine staying free is asserted on the doctrine library's own rig |
| **7, the fact interface** | nothing. `with_minds_facts` is composed by every Active launch (`launch.rs`) | all of it: the filing, lapse and transfer-receipt facts are read only on `PlayerWorld` and `IcWorld`. Nowhere at all: a renewal after a lapse leaving the lapse on record, and a front consuming a mind fact |

So the row's "moved onto the production composition" is the wrong picture for four of the six.
Stories 1 and 6 mostly exist there with restarts. Stories 2 and 7 do not exist there in any
form; story 3 exists in part and one of its premises cannot be built on a launch; story 5 is
scattered across unit tests of the owners.

**Item 2: story 4, the hardware story.** Before this unit: on the hardware flow's own rig only
(`hardware/flow.rs`, no launch, the Class III entitlement minted by a test fixture), and on P3
as far as delivery (MW.23's `m8_end_user.rs`). No launch world fitted a Class III core, read it
after a restart or replaced it; the M8 stories that fly a cored rig bind the root by staff
(`ic_sortie_surface::fit`: `@unit/remove`, `@unit/create`).

**Item 3: the bot's sentences.** The bot keeps substring needles in
`crates/mush-bot/src/mine/lines.rs` and `arena/lines.rs`; unit tests there read tacsim's own
`pub const` templates through the dev-dependency and assert each needle is inside the game's
sentence, so a changed sentence fails `cargo test -p mush-bot`. `mush-market` is not a
dev-dependency, so its sentences are copied text that no test reads. Of what this programme
added, the bot holds `MIND_UNTITLED` only. Not held: MW.23's twelve `+enduser` lines
(`tacsim::launch::end_user`), MW.2's `RENEW_STOPPED` and the other `RENEW_*`
(`mush-market/src/mind_renewal/player.rs`), the claim refusals (`surface::excursion`'s
`CLAIM_*`, `surface::dispatch`'s `MIND_PENDING`, `MIND_SEATS_HELD`, `MIND_NO_LICENCE`,
`MIND_GOVERNED_DRAFT`, `MIND_NO_CORE`, `MIND_UNAVAILABLE`), MW.7's seven lines
(`field::absent`), and the whole `+minds` family (about a hundred constants in
`minds/market_commands/words.rs`, a `pub(crate)` module). MW.1 and MW.24 added no player
sentence.

**Item 4: the cross-host cases.** D-UM-28's corpus (`domain_arena::minds_rhai`,
`m2_the_twin_corpus_hashes_the_same_on_every_host`, with the pool and twin cases beside it)
runs on Linux only: `ci.yml` has no Windows lane and says Windows is the blessing host, by
hand. MW.9 re-pinned its four rows at `api=8` from a Linux run, so **since MW.9 the pins have
not been run on Windows**. Nothing under `minds/registry/` or the Roll's `rating/` pins a hash:
a Registry trial's result, a rating's execution digest and a Conduct verdict have never been
compared across hosts.

**Item 5: Registry load beside live arenas.** Never measured. A rating runs on its own
`mind-bulk-N` threads (`minds/bulk/executor.rs`, `proving_workers`, default 1), each lease
committed by the world thread at `registry_pulse_ms`; arena thinks go to the `tacsim-mind-N`
pool (`[tacsim.minds].workers`). They share the cores and the world thread. MW.1's figures are
a rating alone on an idle server. No counter exists for the Registry's queue depth or a
lease's latency; the timing hooks are test-only prints (`M7_LAUNCH_REGISTRY_TIMING`,
`M7_STORY_RATED`).

**The two owed stories.** MW.7's filed mind in a field: pinned at the arena only
(`domain_arena::minds_rhai::mw7_a_sealed_mind_that_fires_first_is_refused_until_it_is_fired_on`).
The P3 field stories (`s4b3_*`) fly a House Pin on a prospector with nobody in its cab, which
is not an absent player's unit (the gate binds a unit with a seated pilot; AP.5 and AP.7 own
the empty cab). MW.23's refit and restart: item 2.

### Built: item 2, story 4 on the launch (PR #558)

`crates/tacsim/tests/domain_field/c4_hardware.rs`,
`c4_story_4_a_class_three_core_is_fitted_read_after_a_restart_and_replaced`, in the default
sweep (about 22 s). It is also MW.23's owed "refit and restart of a fitted Class III". On P3,
with nothing minted but credits:

1. the pilot registers as an end user by their own verb (`+enduser/apply go`);
2. the importer's stock agent issues a certified Class III die and offers its integration,
   the pilot pays the catalogue's price, the work runs its 96 hours, and the core is
   delivered with a title born of that integration;
3. the pilot's own coreless rig is fitted at the chartered shop: the root is the rig
   family's governed Class III, and the fitting fee goes to the shop;
4. the server restarts. The unit is the same unit; the Registry's inspection reads that
   core, Class III, governed, certified by the importer; the rig's next launch seals over
   it and the seal names the core and its class;
5. with a dent in the plating, a Class II core is fitted in the first one's place: the root
   is the family's Class II, the dent and every cell and component outside the core are as
   they were, and the displaced core is back in the pilot's hands, titled to them,
   unencumbered;
6. a core is fitted once. The core fitted in the first rig is in that hull and not in the
   pilot's hands, so the shop will not offer it for the second rig (the kernel's head for
   it reads `mounted`). The displaced core is in the pilot's hands and the shop offers it,
   and the kernel refuses the acceptance: its head reads `detached`. The story asserts both
   heads and the step each refusal comes from; nothing is charged. A core fresh from the
   shop goes into the second rig, so the second rig was never the reason;
7. a second restart keeps both rigs.

**What staff do in it**, none of it the thing under test: a wallet top-up; the two leased
rigs titled to the pilot (`@asset/release`, `@asset/transfer`); the pilot and a rig stood in
the fitting shop and back in the bay; the dent (`@unit/damage`).

**Not in it.** Story 4's last clause, "existing OOC pods show no owned-hardware
dependency": the M8 rig runs no simulator match. The hardware flow's own rig
(`hardware/flow.rs`) and the simulator's reset stories cover the pods; nothing joins the two
on a launch world.

**Found.**

- **A displaced core is never fitted again.** The kernel admits a core to a fitting only if
  its head is absent or `released` (`mush-core`,
  `action_state/terminal_capacity/core_refit/transition.rs`, `admission_heads`); a core
  that has been mounted reads `mounted`, and one displaced by a later fitting reads
  `detached`. Doc 17f §9 says "displaced core remains owned, and a second install cannot
  reuse it", so this is the rule as written and the story pins it. It makes a displaced
  Class III core, 25,000cr of hardware, an object its owner holds and can never use again.
  **For the owner**: if a displaced core should go back into a hull (or be sold to someone
  who can fit it), the change is in that admission and in the `detached` head's meaning,
  and it is a kernel change, not a tuning.
- **The story's rigs are leased, and the refit asks for the titleholder.** So staff title
  them to the pilot, as every M8 hardware story does. A rig a player holds title to comes
  from the yard (`+yard/collect`, doc 39 cut IN.9, `port/commission.rs`), and a hull from
  the Co-op buyout. No story builds a rig at the yard and then has a core fitted to it, and
  whether the refit's roots (`payment.fits`: the line, rig and coreless-TV families) admit
  a yard design is unchecked. **Left for the unit that joins the yard to the hardware.**
- **The fitting shop is not a place.** The launch's `factory` is an object named in
  `launch.json`; the refit asks that the requester and the hull stand in it, and nothing in
  the game moves a rig there. The story stands them there by staff. On a dev box the
  bring-up's shop is the same kind of object. **Gap, with the one above.**

### Built: item 4, the cross-host cases (PR #558)

- **A Registry trial pinned** (`crates/tacsim/src/minds/registry/trials/physical_tests.rs`,
  `c4_a_registry_trial_ends_the_same_on_every_host`, default sweep, 2 s). Four cases of the
  Duel trial at seed 1 run to their real terminals: a script that stands and fires by the
  stock rules (its fixed and its cycling variant), a script that closes on what it sees, and
  the stock profile at Class II. The test asserts each case is conformant and that the
  arena itself ended the match (a case stopped by a budget fault also answers a result, and
  is not one). Each case's pin holds why the arena ended and what the Registry files that
  as, the stop millisecond, the arena's final state hash, the black box's digest and the
  SHA-256 of the case result's JSON (the record a rating's page carries). All four end by
  elimination, the candidate lost, at 58,500 ms: the opponent's fire decides the moment,
  and what the candidate did shows in the state and the black box, which differ case by
  case. Blessed on Linux (conclav.dev, x86-64) and run on Windows
  (x86-64) at the same tree: all four rows equal. It is not `#[ignore]`d, so every pull
  request runs it on Linux and every local run of tacsim's library tests runs it on Windows.
  A change that moves a row on purpose (a new mind ABI, a rule's numbers, the proving
  content) re-blesses it from the `C4_TRIAL_PIN` lines the test prints. CI has no Windows
  lane, so the run on the other host is by hand and nothing enforces it; that is true of
  D-UM-28's corpus as well. **A Windows lane for the two corpora is the missing piece, and
  no unit owns it.**
- **D-UM-28's corpus run on Windows at `api=8`.** On this Windows box at `9d8fc96b7`:
  `minds_rhai::m2_the_twin_corpus_hashes_the_same_on_every_host` (the four rows and the float
  probe MW.9 re-pinned from Linux), `twin::m2_whole_match_hashes_are_equal_between_the_runners_over_the_pool`
  and `minds_pool::m4_real_tasks_over_the_twin_s_thirty_hash_as_the_library` all pass. So
  the pins MW.9 took on Linux hold on Windows, and MW.7 with MW.9 together on main moved
  none of them.
- **Not pinned:** a whole rating's receipt (a certificate takes about 1,070 leases; its
  digest folds every case result, so one case result differing across hosts would move it,
  and four are pinned), and a Conduct verdict. A third host family (aarch64) has never run
  either corpus.

### Built: item 3, the bot's sentences (PRs #561 and #563)

`crates/mush-bot/src/mine/minds.rs`: one needle for each sentence constant of six game
modules, 149 rows (148 new needles, and `MIND_UNTITLED`, which the bot held), named as the
game names them.

| Family | Game module | Rows |
|---|---|---|
| `end_user` | `tacsim::launch::end_user` (MW.23's `+enduser`) | 11 |
| `renewal` | `mush_market::mind_renewal::player` (MW.2's `RENEW_STOPPED`; the five status labels) | 15 |
| `claim` | `tacsim::surface::excursion` (`CLAIM_*`) | 4 |
| `dispatch` | `tacsim::surface::dispatch` (`MIND_*`) | 9 |
| `absent` | `tacsim::field::absent` (MW.7's lines, and AP.5's `ADMITTED_AWAY`) | 8 |
| `market` | `tacsim::minds::words` (the `+minds`, `@mind/*` and `@registry/*` family) | 102 |

The tests are what make "green on every new sentence" a standing gate and not a one-time
check:

- **in step**: every needle is a literal fragment of the game constant of the same name, so a
  reworded sentence fails `cargo test -p mush-bot`;
- **complete**: a census reads each game module's source and fails naming any `&str`
  constant with no row and no listed exclusion (five: a typed verb, two store keys, a
  refusal's code word, the engine's generic refused line). A `const` or `static` of a `str`
  type the census cannot attribute fails it too, so nothing is skipped silently;
- **distinct**: within a family and across all six, no needle is found in another row's
  sentence, except listed pairs with their reasons;
- seven needles too short to identify their sentence alone are listed as `PARTIAL`, each
  saying what a reader must use beside it.

Changes in game crates are visibility only: `tacsim::minds::words` is public (the module was
`pub(crate)`), two renewal lines in `mush-market` are public, and its five status labels,
which were match arms, are `pub const`s that `label()` returns. No sentence's text changed.

**The gate fired on main within the hour, on another session's merge.** #561 merged at
15:54Z; doc 45 cut AP.5 (#560) merged just after and added
`tacsim::field::absent::ADMITTED_AWAY`; main's `test (bot)` failed on `absent_census`, naming
it. #563 (merged 17:55Z) added the row. A note for the sibling sessions is in the shared notes
folder (`mw5-bot-sentence-census.md`): a new sentence constant in one of the six modules needs
its row in the same PR. **This will meet AP.8 and every later unit that speaks through
`field/absent.rs` or the market's words.**

**A process fault of this session, for the record.** #561 was merged at its first commit
while a second agent's review fixes were being verified; the fix commit was pushed to the
branch five minutes after the merge, which the standing rule forbids (nothing is pushed to a
branch whose PR has merged). It reached no PR and nothing on main. The fixes went to main by
the fresh branch and #563, as the rule has it.

**What the census cannot see.** Sentences built inside functions are not constants: the
renewal status details (`personal_status_line`, `house_status_line`), the reasons
`RENEW_STOPPED` is filled with (`stop_reason`), the reasons `GONE_HOME` is filled with
(`Why::clause`), and the excursion's `word` renderings. And only these six modules are
censused; a sentence in any other module of the market (the Registry's operator lines, the
hardware's refusals) has no row. **MW.19**, whose lane will read the lines, is where the
table earns a reader and where the next modules join it.

### Built: the filed mind flying an absent player's unit in a field (PR #567)

MW.7's owed story, and the first story in which a **player-authored** mind is written, rated,
filed, launched and flown on the production composition.
`crates/tacsim/tests/domain_field/c4_filed_mind_field.rs`, two entries over one script:

- `c4_a_filed_mind_flies_an_absent_pilots_escort_and_is_taken_home` (`#[ignore]`d: one genuine
  rating; the `m8-gate-stories` job; about 340 s on conclav.dev, of which the rating is 165 to
  175 s).
- `c4_the_house_mind_takes_an_absent_pilots_escort_home_on_the_launch` (default sweep, 42 s):
  the same story with no mind chosen. MW.7's stories run on the driving rig, which composes no
  launch; this is the rule over the launch composition for the mind every hull has.

**Runs.** conclav.dev (Linux, over main `d2be044c5`): the filed story green twice (339 s
before the review's fixes, 262 s after), the house-mind entry 42 s. The CI dispatch on the
branch (run 43284568648, at `3dcd278d7`): the story's job, "M8 MW.5 a filed mind flies an
absent pilot's escort", passed on the hosted runner in 744 s with its build.

The story, on P3 (`m8_world_manned`), through the production command paths:

1. the pilot leases Occator's escort at Ceres (`+tv/lease escort go`), takes the operational
   loan, and sails to Ahuna Flank; two more captains stand on the pad;
2. the pilot writes a script with the workshop's verbs (`@mind/new`, `@mind/line`,
   `@mind/save`), has the Registry rate it (`@mind/rate`, its real trials), and files it from
   their own wallet (`@mind/file`, twice: the charter's shell, then the filing). The script
   is written to fire first: at every think it asks for a shot at a contact that is not a
   friend, whether or not anyone has fired on it;
3. the pilot boards the escort in the bay and launches it (`+tv/1 launch mind=filed:<the
   owner's licence> go`). The launch waits behind its claim: the Roll opens the claim in the
   bay (MW.0), on a leased hull (MW.22), and the launch's seal is the claim's own;
4. a second captain's fire order promotes the site: three parties, the escort admitted with
   its pilot seated. The pilot marks the raider's party Hostile, sets fire free, and quits.
   The unit passes to its mind at once (Q-44-46);
5. for five seconds, with the raider Hostile in range and fire free, nothing happens. Then
   the raider's captain puts one round on it (fifteen seconds of fire until 2026-11-13: "The
   raid is one round", below);
6. the unit goes home: its carrier's ramp, out of the field (at about 74 s), into the bay by
   the host's recover; the claim's Close lands;
7. read from the field's black box once the field has closed, in order: the script had
   locked its target before the raider's first round arrived on it; no round of its left
   before that arrival; its rounds after it went at the raider and none at the third party;
   no line says the mind faulted or was dropped, so it was the script that flew; and the
   field's seal line for the unit names the filed mind, the pilot's owner's licence,
   `active`, and the claim's card. The hull's own record agrees: it came home with rounds
   spent and plates struck. **What the box does not hold is the refusal itself**: a refused
   intent writes no line (and a lawful arming writes none either), so "it asked and was
   refused" rests on the first mutation below, in which the same script, with the gate off,
   fires before it is fired on. MW.7's arena test reads the refused intent directly.

**Mutations** (on conclav.dev, in a throwaway copy of the tree): each run against both entries.

| Mutation | The filed story | The house-mind entry |
|---|---|---|
| The gate off (`Arena::absent_denies_fire` answers nothing), the mind's view still narrowed | **fails**: "its first shot follows the raider's first arrival"; the escort fired 12 rounds, 4 at the raider and 8 before anyone had fired on it | passes: the narrowed view alone keeps the stock mind from firing first, as MW.7 found |
| The gate off and the view not narrowed (`Arena::answerable` answers nothing) | not run | **fails**: "the house mind fired on nobody"; the cannon's rounds 40 to 36, the machine gun's 2,000 to 1,810 |

So the filed story is the one that holds the gate on a launch: a player's script is bound by
the engine's refusal and by nothing else. The house-mind entry holds the hand-over, the going
home and the taking aboard; it does not hold the gate. "It fired no round at the third party"
is what the unit did (the script prefers a contact that reads hostile); it is not the gate
tried against the third.

**What staff and the story do by hand.** The story moves the IC clock to the due instant of
the recover the field's host starts (the recover is the host's own), and it connects the
three players and drops the pilot's connection as a clean QUIT. Staff: a wallet top-up for
the filing fee; the escort stood at its cell on
the ground (the combined world has no geology key, so its ground reads dark and no rig drives
in it: every M8 field story stands its rig by staff); the two other parties' hulls and seats
(the field stories' fixture, CF.2's `armed` and `seat`). The unit under test is play's from
the lease to the bay.

**What the story found.**

- **A rating that fails tells the author nothing, and a story that waits on the receipt
  waits forever.** The first script never started its hull up; the second asked for a shot
  from a cannon that had been shot away, on one think of three trials out of thirty-six.
  Each ended the rating as `Failed { reason: Execution }` within two minutes of work. An
  intent the hull cannot carry out is a mind fault, and one fault in one think of one trial
  fails the whole rating; the job row says `Execution` and no line says which case, which
  think or which intent. The M8 rig's `certify_saved` waits only for a receipt, and so did
  this story's first draft: it spun for eighty minutes. The story's wait now reads the job's
  state and fails at once. **Left for MW.19 and the owner:** a player whose script fails
  conformance needs to be told the case and the fault (the trial's diagnostic exists, as
  test-only prints); and `certify_saved` should fail fast the same way.
- **AP.5 changed what a field story's other captains must be.** Merged to main during this
  cut: a seated pilot who is not connected when a field opens is handed to their unit's mind
  at the admission. In this story's world the two other captains read as away until they
  connect, and the raider's captain then has no cockpit to type into; the story connects all
  three. MW.7's stories seat their other captains the same way, connect only the pilot, and
  pass on main; why the combined world differs (it has two restarts behind it, and a restart
  seats only a connected pilot) was not run down.
- **A quiet field's black box reaches the host late.** With nobody firing, the box arrives as
  its chunks fill or when the field closes, and the stock field closes after two quiet
  minutes. A story at the stock tempo cannot assert on the box inside a short window; this
  one reads order from the whole box at the end, and the hull's own record for the fast
  variant. The M8 world cannot tune the field (a tuned layer would run the Registry's trials
  at the same tempo).
- **The combined world has no geology key**, so a rig cannot drive in it (`+tv/1 drive`
  answers that the instruments cannot read the ground). The escort's drive out is the one
  step of the unit's journey that is not play's. A keyed M8 world needs the key installed at
  every one of its boots; nothing does that. **Gap in the rig, not the game.**

**Not in it.** A restart with the mind holding the unit (MW.7 has it for the house mind on
the driving rig; `d23_18_*` has a restart between two fields for a licensed mind with nobody
in the cab). The five-minute link loss, in place of the QUIT: the stock field would close
during it. A bought mind, in place of the author's own: **MW.21**.

**The raid is one round (2026-11-13).** The filed story failed about one run in three on
main, under load and on the hosted runner alike, always at step 6: "the mind takes the escort
out at its carrier's ramp", the escort still on the cell it was stood on. The cause was the
story's own raid, not the game.

- **What happened.** Step 5 was fifteen wall seconds of `FIRE` from eight cells behind the
  escort, against the cannon's six-second cycle: two rounds or three, by how the lock and the
  cycle fell against the wall clock. A 105 mm round does 120 and every round landed on the
  rear band: the hull's rear plate (150) with the hull (200) or the engine (120) behind it,
  or the turret's (140 over 150). Two rounds finish no section there. Three can: the hull
  (`destroy_unit`), the engine (`disable_motion`) or the turret (`destroy_core`, the drop
  the story's `troubled` already allows for), and the dice chose. A wreck thinks nothing, a hull with
  no drive goes nowhere, and the kernel lets neither out at a ramp
  (`WithdrawRefusal::NotUnderOwnPower`), so step 6 waited its 420 s for a unit that could
  not come.
- **Evidence.** Sixteen runs at main `f5b1efa68`, four at once on conclav.dev: five failed.
  Two of the five were read in full from the black box, and were the same wreck: the first
  round on the engine through the hull's rear plate (30 of plate left), the second through
  the plate into the hull (110 left), the third `destroy_unit`, 23.3 s and 23.6 s into the
  field. No run seen with two rounds failed. Seven of the eleven passing runs came home with
  the engine at 30 of 120, one more round on it from stranded. The match seed differs run to
  run (the same shot's facet draw does), so the rate is the dice's and moves between batches:
  eight more runs at main `4efa11521`, unpatched, all passed, five on two rounds and three
  on three, two of those with the hull at 110 of 200.
- **The fix.** The raider's captain puts one round on the escort (`RAID_ROUNDS`), counted on
  their own cockpit frame and not in wall seconds. One arrival is all the absent player's
  rule asks before the unit may answer, and one round finishes no section on any band. The
  story checks the bound on the box ("the raid kept to its bound") and its step 6 line now
  says what the box holds of the unit's losses. No assertion was loosened and no wait
  lengthened. With it: twelve of twelve at main `4efa11521`, four at once on the same box,
  the raider's one round arriving in each.
- **Not verified.** The other three failures were not read in full (the first probe's filter
  matched nothing). Their field had closed `disengaged` 120 s after the escort's last shot
  with the escort on its cell, which is what an engine shot out would leave; that reading
  was not confirmed.

### Item 5: Registry load beside live arenas

**Measured, PR #566** (a subagent's slice; the method and the figures are its own runs).
`crates/mush-server/src/world_task/submission_disposition/tests/registry_load.rs`,
`c4_registry_rating_beside_live_simulator_matches_is_measured`: `#[ignore]`d, on the by-hand
tier at `9c625bdfe`. One server process a configuration, in P2 (the server's own roster, world
task and writer; an in-memory store). The author types `@mind/rate` on a fresh copy of
Standard Doctrine and the run ends when the Roll's job reads Completed. Matches run beside it,
started as a player starts one (`SIM TEAM red`, `SIM AI n`, `SIM START`, `BOARD`, `EXIT`), all
hulls on the native Standard Doctrine, the next match started as one ends. A server has one
simulator centre and one match at a time, so the load is sized in units, not in matches.
Leases are timed by the store's commit of every `rating_grant` and `rating_settle` (a
test-only lease clock); think-wait is the arena's own `pool_wait_us` trace; the world's
responsiveness is a barrier round trip every 250 ms; CPU and memory are `/proc`.

Host: conclav.dev, AMD EPYC, 32 cores, 62.8 GB, shared with other sessions. Release build.
`registry_pulse_ms` 50, `proving_workers` 1, `[tacsim.minds].workers` 2, eight tokio workers,
each run pinned to eight cores.

| Config | Rating, queued to finished | Lease ms p50 / p95 / max | Gap between leases ms p50 / p95 / max | Think-wait ms p50 / p95 / p99 / max | World round trip ms p99 / max | CPU s (cores) | Peak RSS |
|---|---|---|---|---|---|---|---|
| no match | 1,942.6 s | 1,124.8 / 1,131.8 / 1,178.5 | 153.1 / 156.5 / 179.7 | none | 0.2 / 15.8 | 64 (0.03) | 349 MB |
| 2 a side (4 units) | 3,105.9 s | 1,125.5 / 3,113.7 / 13,152.2 | 153.3 / 3,171.1 / 18,172.2 | 0.1 / 0.2 / 0.8 / 5.3 | 0.2 / 106.6 | 84 (0.03) | 824 MB |
| 6 a side (12 units) | 3,108.7 s | 1,125.2 / 3,116.2 / 10,172.2 | 153.2 / 3,169.5 / 12,116.8 | 0.1 / 0.2 / 0.3 / 4.7 | 0.2 / 157.4 | 89 (0.03) | 1,072 MB |
| 2 authors, no match | 4,991.0 s and 4,992.0 s | 1,124.6 and 1,117.6 (p50) | 2,146.9 and 2,153.8 (p50) | none | 3.3 / 14.7 | 151 (0.03) | 391 MB |

**What it shows, and how far.** A second agent read the method adversarially; the limits it
found are in the wording here.

- **A rating is not a CPU load.** The process averages 0.03 of a core. The Registry's worker
  spends a few milliseconds a lease against a lease of 1.12 s (the story's per-lease split
  is approximate: it counts the preparation thread with the workers, and misses threads that
  have exited). A rating is **1,517 leases**, not the "about 1,070" on file.
- **A lease of 1.12 s is itself unexplained.** The rig's own note expects about a third of a
  second a lease at the 50 ms pulse. Read against `minds/bulk/registry_service.rs`, every
  scheduled continuation is pushed 1,000 ms out (`RETRY_MS`) and only three paths reset it;
  an observation that resolves without an acknowledgement goes back to unsubmitted with no
  reset. That would put the retry's second in **every** lease, match or no match, and a
  rating alone would be about three times longer than it need be. **Not proven: nobody
  counted the continuations that made no progress.** MW.6 and MW.1 saw the same from the
  other side (a 10 ms pulse saved 15%).
- **With a match running the rating took 60% longer, and the run cannot say why.** The
  median lease does not move; the tail and the gaps grow in whole seconds. But the no-match
  run had no driver: with a match, the story also types `SIM STATUS` every two seconds and
  the lobby's lines at every restart, each a round trip through the world. So the 60% is "a
  match and its driver's commands", and the two are not separated. **A "driver, no match"
  control is the missing run.**
- **Whether the matches were slowed was not measured.** The fill flies the native doctrine,
  so the think-wait figure (the Rhai pool's) covers almost nothing, and nothing recorded the
  arena's step time or its pulse lateness. What was measured is that the world's round trip
  stayed at 0.2 ms p99.
- **The queue is fair and unbounded.** One lease is in flight at a time and two authors'
  leases alternate strictly; there is no per-author quota and no refusal. The second author
  is told "Registry preparation queued.", then "Registry work retained and queued." and has a
  first grant 3.8 s after asking. Both ratings finish together at 2.57 times a lone one (the
  excess over twice is not explained).

**A rating talks to its author all the way through.** In diagnostic runs (at `3da18e483`,
after the recorded ones) the author was sent `Queued.` and `Done.` for the grants and settles
of their own rating: 750 lines over 186 leases in 240 s, which would be about 6,000 over a
rating; and the second author in a queue was sent "That did not go through. Nothing
changed." about every 3.3 s. Read against the code this is not the rig's doing: the
Registry's continuation is submitted as the author, the engine answers a submission with
`Queued.` to its actor (`mush-core`, `drain_authoritative_submit`), and the world task
answers the commit with `Done.` (`mush-server`, `world_task.rs`). **Not observed on a live
connection.** If it is as read, it is a defect in what a rating says to its author, and the
fix is in the output path for a service's continuations. **For the owner and MW.19**, whose
lane rates a mind over SSH.

**What it cannot show.** No real players typing; an in-memory store, not Redis (the RSS
column is the process's high-water mark from boot and includes everything the rig retains;
its rise with matches is not explained); a shared box (the one-minute load ran between 8 and
26 across the runs); the 50 ms pulse, not production's 1,000 ms; a match no production world
runs as configured (`ai_fill` on and `unattended_ms` 0, so a pilotless match runs on); one
centre, one match at a time, twelve units at most; and no IC field arena beside the rating,
which is the "live arenas" a deployed world will have most of. **MW.20** measures minds at
size; a rating beside a field, the driver-only control and the arena's own step time belong
with it.

**Q-44-48.** The budget stays as it is. Nothing here says the Registry's work crowds a match
for CPU. What the measurement adds is two questions about the Registry's own pace: the
second in every lease, and what a busy world adds to it.

### Item 1: what this unit did not build

Nothing of item 1 was moved onto the production composition by this unit. The inventory
above says what stands there and what does not; doc 17g's "C4 as built" has the table with a
reason and an owner for each missing piece. In short:

- **Story 2** (private and derivative) and **story 7** (the fact interface) do not exist on
  a launch in any form. Story 2 is a second command story on the server rig with two genuine
  ratings; story 7 is a contract with a `minds` condition drafted, accepted and settled on a
  launch (the economy's fact table is crate-private, so there is no shorter read). Neither
  has a unit.
- **Story 3**'s first premise, a two-seat licence, cannot be made on a launch.
- **Story 6**'s cover clauses cannot run on a launch: hull cover is not composed there.
- **Story 5**'s fault-injection clauses stay on the owners' fixtures, by decision.
- **Story 1**'s simulator half and its privacy battery need a simulator match driven in a
  launch rig. This unit's field story covers a neighbouring clause instead: a player-authored
  mind sealed and flown on a launch, IC, by its author.

### Defaults taken

- **A script fault fails the story's rating, so the story's script is defensive.** It checks
  that its cannon holds a round and that its target does not read dead before it asks. The
  alternative, a Registry that tolerates an occasional illegal intent, is the owner's.
- **QUIT, not link loss**, hands the unit over in the field story (see above).
- **The cross-host pin is not `#[ignore]`d.** It costs every pull request two seconds and
  fails the one that moves it.
- **The bot's census covers six modules**, the ones this programme and C0 to C3 added
  sentences to. Adding a module is one `include_str!`, one family and its rows.

### Left, and for which unit

| What | Whose |
|---|---|
| Stories 2 and 7 on a launch; story 3's settlement half and a restart during a pending sale | no unit; a "C4 second cut" if the owner wants C4 whole |
| A two-seat licence on a launch (per-listing terms) | the market's terms work; no unit |
| Hull cover composed by a launch, then story 6's cover clauses | needs a ruling; no unit |
| A bought mind flown by its buyer, the simulator half of story 1 | MW.21 |
| The bot reading the table; more modules censused; the author told why a rating failed | MW.19 |
| A Windows CI lane for the two cross-host corpora | no unit |
| A rig built at the yard and fitted with a core; a fitting shop a rig can be taken to | no unit |
| A displaced core's fate | the owner |
| `certify_saved` failing fast on a rating that ends without a receipt | whoever next touches the M8 rig |

### After the merges: the unit at `api=9` (2026-11-12, main `0d52a87d2`)

#566 and #567 merged at 19:42Z in one batch with doc 45 cut AP.3 (#565), which moved the mind
API to 9 (`MIND_ABI` is `rhai=1.26.0;api=9;stock=3`). Neither of this unit's two heads had
AP.3 as an ancestor, so the pair first met `api=9` on main. Checked the same hour:

- **Neither #566 nor #567 pins a literal that folds a seal.** The load story prints figures
  and asserts none; the field story compares the field's seal line with the claim's own seal,
  read at run time, and the hull's cells, out and home.
- **The one pin this unit added is #558's**, the four `C4_TRIAL_PINS` rows
  (`minds/registry/trials/physical_tests.rs`). AP.3 had #558 under it and re-blessed them
  itself (`ed5593464`, "re-pin what folds a seal or the mind ABI"): each case still ends in
  the same elimination at 58,500 ms, and the state hash, the black box digest and the filed
  result's digest moved.
- **Run at `0d52a87d2`.**
  - This Windows box: `c4_a_registry_trial_ends_the_same_on_every_host` passes against
    AP.3's rows, all four equal to the Linux values; D-UM-28's corpus as AP.3 re-pinned it
    (`m2_the_twin_corpus_hashes_the_same_on_every_host`) and its pool and twin cases pass.
    So both corpora hold on Windows at `api=9`.
  - conclav.dev (Linux): the trial pin, story 4 (`c4_hardware`) and the house mind's field
    entry pass; the filed-mind story passes (`--ignored`, 341 s: rated in 172 s, out of the
    field at 74 s, the escort fired 5 rounds, all at the raider and all after the raider's
    first arrival).
- Nothing of this unit was committed after the two merged heads; there was nothing to
  re-raise but this note.

## MW.6

The Registry's pace as a file key, and the two notice drains that stopped on one bad holding
(doc 44 §7.1, Q-44-41; second review, §4c rows 4 and 10). Built on Q-44-41's default, which is
open: the key exists with today's value as its default, and a dev box sets it low.

**Built.**

- **`[tacsim.minds].registry_pulse_ms`.** `mush_config::TacsimMindsCfg.bulk_maintenance_ms` was
  `serde(skip)`, so no file could set the pace of a Registry lease; only the acceptance stories'
  harness could, in code. It is now a file key under the name `registry_pulse_ms` (the Rust
  field keeps its name and is not a key). Absent, it is 1,000 ms, production's second, as doc
  43c records was decided. `MindsConfig::validate` (`crates/tacsim/src/config.rs`) refuses a
  value outside 10 to 60,000 by the key's name, ahead of the bulk model's own check. The key is
  documented, commented, in `mushd.toml.example` and `deploy/instance/mushd.toml.tmpl`, and in
  the key table at the head of `crates/mush-config/src/config.rs`. `tacsim::MindsConfig` itself
  still skips the field in its own serde; the host's mapping (`mush_server::config_ext`) is the
  path a file takes.
- **Both notice drains skip one bad holding.** In `crates/mush-roll/src/commerce/registry_recall/notice.rs`
  and `registry_revocation/notice.rs`, `tick` returned the first error from `own` before the
  cursors were written back, so the next tick read the same page, met the same holding and
  stopped again: every notice behind it waited for ever. Now an error that is one holding's own
  (`one_holding`: `Missing`, `Mismatch`, `Invalid`) skips that holding, emits nothing for it,
  logs it once (`Runtime::note_skipped`, bounded like the remembered notices) and lets the
  cursor pass. An error that is the inventory's (`Unavailable`, `Stale`, `Capacity`) still stops
  the drain where it is, before anything is emitted, to be tried again. `page` had the same
  fault one level up: a row that failed `validate` failed the whole page. Such a row is now
  carried with no recipient, so the cursor passes it.

**Tests.** `mush-config`: `m1b1_the_minds_block_parses_absent_written_and_refuses_a_typo` gains
the key (absent is 1,000; `registry_pulse_ms = 50` parses; `bulk_maintenance_ms` is an unknown
field). `mush-server`: `config_ext`'s refusal table gains 5 and 60,001, each refused by the key's
name, and the mapper carries 50 through to the layer's config. `mush-roll`: one test in each
notice module, `a_bad_holding_is_skipped_and_an_unavailable_inventory_is_not`. Local: `cargo test
-p mush-config` 141 + 14 passed; `cargo test -p mush-roll notice` 3 passed; `cargo test -p
mush-server --lib config_ext` 18 passed; `cargo test -p mush-contract-tests` green; clippy with
warnings denied on mush-roll, mush-config, tacsim and mush-server.

**Divergences.** The unit's row asks for "the measured time a rating takes at both paces". It
was not measured in this cut. The figures on file are `ci.yml`'s (about 1,070 leases a
certificate, about three hours a story at 1,000 ms) and the acceptance stories' (minutes at
their 50 ms). MW.1's bring-up is where a dev box's rating is first timed.

**Not built.** Registry pacing beyond the pulse: a lease still takes 3 to 7 s of work at the
stock pace, and nothing here shortens the work a lease does. The key is not set in
`deploy/dev1` or `deploy/dev2`; MW.1 sets it when it renders the launch.

**Not pinned by a story.** The drains end to end: no rig corrupts one holding and shows the
holder behind it getting their notice. The tests pin which errors skip and which stop, and that
the skipped keys stay bounded. A row that fails `validate` in `page` is not pinned at all.

## MW.7

The field's backup mind and the absent-player gate (doc 44 §7.1, §4 C, §4b rows 2 and 3, §4c
rows 11 to 13 and 22; Q-44-16 to Q-44-19, Q-44-22, Q-44-35 to Q-44-38, Q-44-42, Q-44-43,
Q-44-46; doc 35 §11.4's `FB.1` to `FB.3`; doc 27 Q-H5). PR #549, stacked on MW.14a (#545).

**Built.**

- **The absent player's rule, once** (`crates/tacsim/src/arena/absent.rs`). `permits(facts,
  ask)` is the whole rule and the only copy: for a unit a mind flies for an absent player, a
  shot is let through only at a unit whose fire has arrived on this one, and not at all while
  a hold on its fire stands; mining and trading are refused. It reads no arena, no clock and
  no dice. The arena asks it at the one door every shot no seat typed goes through
  (`Arena::resolve_fire_by`, `arena/weapons.rs`): a mind's own cited shot, a standing fire
  rule's and a machine's alike, so the stock mind's cornered stand and a player's sealed
  script meet the same refusal (`unwarranted`, the word a held shot already reads). A seat's
  typed `FIRE` is never asked.
- **The three facts, on the unit** (`arena/state.rs`). `UnitState::absent`, written only by
  the host's word (`ArenaMsg::Absent`, applied at a pulse's head by `Arena::set_absent`).
  `UnitState::fired_on_by`, the units whose fire has arrived on this one, hit or miss, written
  at every impact (`arena/damage.rs`) in a field whose end is its host's and never in a
  practice match. The hold: the seat's own master gate (`weapons_hold`, the hold the pilot
  left) or a standing order's `hold_fire`. Both new fields are carried by the snapshot and
  folded into the state hash only when set, as `coupled_to` is, so `SNAPSHOT_FORMAT` stands at
  19 and every practice match hashes as it did.
- **Going home reaches every mind.** `Arena::set_absent` lays a standing `Withdraw` with no
  deadline on the unit's mind row (`lay_go_home`), and a `Released` that hands over a unit
  already marked lays it too. The stock mind obeys it as it obeys a leader's: it drives for
  its party's home and never rallies while the order stands. A player's sealed script reads
  it as `v.order` (objective `withdraw`). An order to advance or attack that stood is
  replaced; its `hold_fire` is kept. A detachment command that reaches the unit meanwhile
  can only add `hold_fire` (`arena/detachment.rs`).
- **What the mind is shown** (`Arena::mind_view`). While the flag stands, a contact its party
  holds Hostile reads Neutral unless the rule would let the unit answer it. Without this the
  stock mind's cornered stand stopped the unit beside any marked party in range and it never
  got home. The view is narrowed, never widened, and the gate does not depend on it.
- **The field's presence record and the one door** (`crates/tacsim/src/field/absent.rs`).
  `Away { why, since_wall_s, handed }` rides the unit's row of the live record, so a restart
  keeps it. `hand_over(ctx, unit, pilot, why)` is the door: it marks the record, tells the
  arena (the flag first, then the seat's `Released`), tells the pilot, and tells those the
  unit was with. Three callers today: the field's own five-minute clock
  (`FIELD_LINKDEAD_GRACE_WALL_S`, 300 wall seconds from the lost link, checked once a world
  second), a clean QUIT (the seat's `Disconnected` hook with no clock running, at once), and
  `crew::on_frames` when a pilot inside a hull is incapacitated (at once, connected or not).
- **The host boards it** (`field/home.rs`, `field/exit.rs`). `home::ask_out` has a rig's exit
  ask (the ramp its own carrier stands with) beside the DT one, and asks quietly: nobody is
  told the ask or its refusals. Once the kernel lets the unit out, `exit::tick` runs its
  recover as its pilot (the rig's recover act, or the DT recover), and
  `absent::retry_recovers` tries it again every ten wall seconds until the unit is off the
  site's ground.
- **The way back** (`absent::take_back`, from the seat's `Connected` and `Revived` hooks and
  from the tick). A pilot who is connected, fit and still seated in the unit is bound and
  seated again; the arena is told the player is back. The seat's `Live` control discards the
  mind's pending think and its order (`Arena::suspend_mind`, unchanged), and the task recalls
  a ramp exit it has not yet applied (`WithdrawRefusal::Recalled`). A unit already out at the
  ramp stays out, and the pilot is told.
- **A restart** (`field/boot.rs`, `absent::at_resume`). A unit its mind held is not seated;
  the tick tells the resumed task again. A seated pilot who is not connected is bound, the
  seat `Linkdead`, and the five minutes run from the boot unless the record carries an
  earlier loss. A pilot who is connected is seated as before.
- **The lead is told** (doc 35 §11.4's open point). The leader of the unit's own party and the
  leaders of the parties that stand its friends hear one line (`GONE_HOME`), queued for one
  who is offline. The unit goes.
- `docs/help/field.md` has a new section, "If you lose your link, or quit". Doc 35 §11.4 and
  doc 27 Q-H5 read as built.

**What a caller in space must provide** (for MW.9's follow-up; one rule, not a copy). Call
`tacsim::arena::absent::permits(&Facts { absent, holds_fire, fired_on_by }, &Ask::Fire { at })`
(or `Ask::Mine`, `Ask::Trade`) where the seat accepts an intent, and act only on `Ok`.

- `absent`: the hull is flown for a player who is not at her controls. Space has it:
  `SpaceLinkdeadState::backup_has_helm()`.
- `holds_fire`: a hold the crew left stands. Space has no such order today; pass `false`
  until it does.
- `fired_on_by`: the set of hulls whose fire has arrived on this hull since the bubble
  opened, keyed by the same id the `Ask`'s target uses (`ObjId` fits). **This is the one
  thing space does not have**: the bubble keeps no record of who fired on whom (doc 35
  Q-35-B3). It is a set a hull, written where a round's arrival is resolved, carried with the
  bubble's state. Until it exists the seat's present rule, refuse every lock, shot and
  arming, is the same rule with an empty set.

**Tests.** In-source: the rule's arms (`arena::absent`), the rule at the door for each source
of a shot, the go-home order, the narrowed view and the hold (`arena::weapons`,
`mw7_an_absent_players_mind_fires_only_on_a_unit_that_fired_on_it`), the hash fold
(`arena::hash`), the record's wire form (`field::absent`). Arena
(`crates/tacsim/tests/domain_arena/minds_rhai.rs`): a sealed Rhai script written to fire
first is refused every shot, fires once it has been fired on, reads the order and drives
home, and the kernel lets it out at the ramp; a script that faults three thinks running is
dropped to the Standard Doctrine, which drives home and fires on nobody. Stories in
`domain_field` (`crates/tacsim/tests/domain_field/field_mw7.rs`), each a field of three
parties, each passing on Linux (conclav.dev, six at once):

- `mw7_the_house_mind_takes_an_absent_pilots_rig_home_and_never_fires_first` (54 s). A
  miner's prospector, a leased escort flown from its cab, an intruder. The escort's pilot
  marks the intruder Hostile, sets weapons free and loses their link. Twenty seconds short of
  five minutes the seat is still theirs; at five minutes the mind has the unit. It drives to
  the ramp, the kernel lets it out, the host's recover puts it in the bay, and it has fired
  nothing since the link was lost.
- `mw7_an_absent_pilots_mind_answers_only_the_unit_that_fired_on_it` (41 s). A clean QUIT
  hands over at once. With the raider marked Hostile eight cells off and weapons free the
  unit fires on nobody for twelve seconds; once the raider's fire arrives it answers the
  raider, after that first arrival, and never the third party. No carrier of its party is on
  the site, and it holds where it stands.
- `mw7_a_hold_fire_left_by_the_pilot_holds_through_an_attack` (110 s). The seat left at
  weapons hold: three of the raider's rounds arrive and the unit answers nobody.
- `mw7_a_pilot_who_reconnects_ten_minutes_later_is_back_in_the_seat` (25 s). Link lost, the
  session ended, ten minutes gone. The pilot connects, is told, is bound again, takes the
  focus, and their `DRIVE` is the hull's goal with no order standing.
- `mw7_a_pilot_knocked_out_while_connected_stops_steering` (20 s). A pilot inside a walker
  whose cockpit is destroyed: connected, incapacitated, the seat released at once; a
  reconnect is refused the controls.
- `mw7_a_restart_while_the_mind_holds_the_unit_resumes_the_mind` (44 s). The server restarts
  with the mind holding the unit. The boot seats nobody; the resumed mind fires on nobody
  until the raider's fire arrives, then answers; the pilot connects and is seated.

**Verified, and how** (Linux, conclav.dev, at `654031bf`). The six stories and the two arena
tests pass. The whole `domain_field` binary passes (253 run), with one older story changed:
`field_crash::cf_review_an_extraction_out_of_a_running_field_releases_the_seat` asserted that
an incapacitated pilot keeps the seat until extraction, which Q-44-43 ends. `domain_arena`
(1,125) and the tacsim library's tests (2,222) pass; `cargo doc -p tacsim` is clean under
`-D warnings`; clippy is clean on the library. Five mutations in a second throwaway clone,
each run against the stories and the arena tests:

| Mutation | What failed |
|---|---|
| The field takes no part in the seat's hooks (main's behaviour: a drop is a plain `Released`) | all six stories |
| The gate off and the view not narrowed | the four fire stories (the unit fired first, 11 to 38 rounds) and both arena tests |
| The gate off, the view narrowed | the sealed-script arena test only: in a field the narrowed view alone keeps the stock mind from firing first |
| The view not narrowed, the gate on | story 1 (the unit stood where it was, facing the marked party, and never reached the ramp) and the arena test's view assertion; the other fire stories passed on the gate alone |
| A returning pilot is never taken back | the reconnect, restart and knocked-out stories |

So the stock mind is held by either half, a sealed script only by the gate, and the unit gets
home only with the view narrowed. The stories cannot be compiled on main (they read
`tacsim::field::absent`), so the first row stands in for "fails on main today".

**What the survey had wrong, or left out.**

- **A `Released` already sends a unit home.** §4 C says the released unit "never goes home".
  The kernel seeds the Standard Doctrine `Withdraw` at every release, so a released unit does
  drive for its party's home. What was true is that it rallies after a lull and fights, that
  only doctrine is seeded (a sealed script is told nothing), and that nothing puts it aboard.
- **The IC weapons hold is the "HOLD FIRE left by the pilot".** A seated hull holds no
  standing order at all (`apply_detachment_command` refuses a pilot's hull, and a possess
  clears the row), so the only hold a pilot can leave on their own unit is the seat's master
  gate. Every IC seat opens at hold (Q-CF-41). Before this cut a hull a mind took over
  ignored that gate by design.
- **No intent in the arena mines or trades.** The rule has both arms for space; in a field
  there is nothing to refuse. Mining in a field is refused as an order already
  (`field/orders.rs`).
- **The cornered stand needed more than a refused shot.** With the shot refused the stock
  mind still stopped and faced the nearest marked party, and stayed there. Hence the narrowed
  view.
- **"Five minutes" was the server's session grace**, `[game].linkdead_grace_s`, not a field
  rule: an operator who shortened it shortened the owner's five minutes. The field now runs
  its own clock, and a session that ends on a lost link no longer releases the seat.
- **A cockpit command is answered from the seat's last frame**, so a returning pilot's first
  command waits for the first frame of the new tenure (one pulse). The story waits for it.

**Defaults taken.** None needs a refactor to undo unless it says so.

- **The pilot's weapons hold binds the mind.** A player who never set `FIRE FREE` leaves a
  unit that answers nobody, even under fire. It is the ruling's "the stricter rule wins"
  read on the only hold a pilot has, and every IC seat starts there. If the owner would
  rather an absent player's unit always defend itself, it is one clause in
  `Arena::absent_denies_fire` and one in `Arena::answerable`. **For the owner.**
  (Reversed by doc 45 unit AP.6, 2026-11-12, on Q-45-5 and Q-45-7 to Q-45-9: the posture is
  what a player leaves standing and the seat's switch binds the seat alone. The third story
  below is rewritten there as `ap6_a_seat_left_at_fire_hold_does_not_bind_the_mind`.)
- **The field's five minutes are a constant**, as space's are. `[game].linkdead_grace_s` no
  longer decides when a field unit is handed over. A story steps the world's wall second, so
  none waits five real minutes.
- **A session that ends with no clock running is a quit.** The `Disconnected` hook carries no
  reason, so the field tells a quit from a grace's end by its own record, as space does. A
  pilot whose link was already down when the field opened and whose session then ends is
  therefore handed over at once; they had been gone the whole grace already.
- **A sealed script that ignores the order is not driven home.** The order reaches it and
  the engine binds its fire; if it never moves, the unit holds. This is doc 35 Q-35-B4's
  default for space, taken for the field too. The alternative, the engine driving any absent
  unit home whatever its mind asks, would be a second controller beside the mind and is the
  one change here that would be a real refactor. **For the owner**, with Q-35-B4.
- **A mind falls to the house default after three faults running**, the kernel's standing
  rule, not at the first. For up to three thinks the unit does nothing.
- **Who is told**: the unit's own party's leader and the leaders of parties that are its
  party's standing friends (owner or organization), never a party that only marked it
  Friendly. Queued at trip level for one who is offline.
- **A restart starts the five minutes again** for a pilot who was flying when the server went
  down (doc 35 Q-35-B8's recommendation, taken for the field). An earlier loss on the record
  is kept.
- **The tick reconciles presence toward "here" only.** A pilot who is connected and whose
  hook never fired (a restart's re-attached connection) is seated by the next tick. Absence
  is opened only by a hook or a boot, never by polling, so a story that sets a session's
  facts by hand is not handed over.

**Not built, and where each lives.**

- **The filed mind through the claim path, as a field story.** The row's second story is
  pinned at the arena with a sealed Rhai script, not through the Registry, the licence and
  the claim seal MW.0 and MW.22 composed. A field story needs the M8 world, a governed core
  on an armed rig and a rated script of the story's own, and none of the rigs flies that
  combination yet. **Owed to MW.5**, whose stories run on the production composition; MW.21's
  story then covers the same in space. PR #546's fix did not matter to anything built here.
- **`FB.4`, the retrieval job** for a unit with no ramp in reach (Q-35-B6): the unit holds
  and says nothing more. **Doc 41's retrieval contract; needs the owner's call on who pays.**
- **The recover owed across a restart.** The retry list is RAM. A unit let out at its ramp
  whose recover had not landed when the server went down stands beside the ramp, out of the
  field, and its pilot's own recover passes, as for any exit today. **AP.7**, which gives a
  carrier's mind the taking aboard.
- **Absent at admission** (a pilot logged off in the seat when the field opens) and **the
  posture record**: **AP.5** and **AP.3**. Both call `hand_over` with a `Why` of their own;
  AP.3's `HOLD` and `GUARD` change what `lay_go_home` lays and what `answerable` admits.
- **A player's rig on a fill plan when a field opens** keeps to today's rule. **AP.7.**
- **A unit nobody sits in is not an absent player's unit yet.** A rig launched with a filed
  mind and no pilot in its cab is its mind's from admission, with no record and no flag, so
  the rule does not bind it and nothing takes it home. The door is there (`hand_over`); what
  calls it for a unit with no seated pilot is **AP.5** (admission) and **AP.7** (the plan).
- **A unit put back into the field because its carrier was lost at the ramp**
  (`field/boarding.rs`, `carrier::rejoin`) comes back with a fresh row and its pilot's seat.
  The host's recover list hands it over again while this boot holds the list; after a restart
  in that window nothing does, since absence is opened by a hook or a boot, never by polling.
  **AP.5**, whose admission rule asks the connection at a join too.
- **A gate in space.** The rule is callable; the bubble's fire record is not built. **MW.9's
  follow-up.**
- **A staff read of why a shot was refused.** `Denied` has a sentence for each limit, and
  the mind is told `unwarranted` as for any held shot. No verb shows which limit it was.

**Reviewed.** A read-only review of the engine diff by a second agent found six points, all on
the host's side; it found nothing in the gate, the snapshot, the hash or the simulator's
paths. Five are fixed in the PR's last commit:

- a restart did not reconcile the arena's flag with the live record, so a unit handed back
  after the last snapshot came up still under the absent player's rule (`field/boot.rs` now
  sets the flag from the record before the task starts);
- the recover list never drained when a recover could never land (it now gives up after
  thirty tries, five minutes, and queues the pilot a line saying the unit stands beside the
  ramp);
- a unit put back into the field after its carrier was lost stayed on the recover list and
  was nobody's (the list now hands it over again);
- the recall took any waiting withdrawal of the unit, so a returning pilot's own recover,
  typed before the next pulse, was dropped without a word (the task now recalls only what was
  offered before the hand-back's word);
- a detachment's authority control arriving after the hand-over cleared the go-home order
  (`apply_detachment_control` now leaves an absent player's unit its order; pinned in the
  kernel test).

The sixth stands, a race of one pulse: a pilot who reconnects after the kernel has let the
unit out and before the host has heard so is told they are back, then that the unit is up the
ramp; the recover runs and the unit is put aboard. Nothing is lost.

**Not pinned by a story.** A refused recover tried again and given up (the first try passes
in story 1). A restart between a hand-back and the next snapshot.
A player leader's DT roster unit taken home by the DT recover (story 1 flies a rig). The
telling of a friend party's leader (no rig stands two friendly parties with different
leaders). The recall of a ramp exit at the hand-back (`WithdrawRefusal::Recalled`). A pilot
who was flying at a restart and does not come back within five minutes. A reconnect inside
the five minutes (the hook's path is `Revived`, unchanged from before, plus the record
cleared). A detachment command tightening an absent unit's order.

## MW.8

Not built.

## MW.9

One applier and one view for every space mind, the trace-string and memory hooks removed, and
the interdictor's stand-down hull by hull (doc 44 §7.1, Q-44-24; first review finding 6, second
review row 16). PR #551. Paths are under `crates/tacsim/src/space/` unless given.

**Built.**

- **The one applier (`applier.rs`, new).** `apply_intents` turns one think's intents into
  bubble commands for every actor: the customs corvette and the raider (`picket.rs`), the
  interdictor (`tick.rs`) and the backup mind of a hull whose crew has no link (`backup.rs`).
  Those were four hand-written mappings. What each intent means in a bubble is one table, at
  the head of the file:
  - `lock(id)`: her lock is the ship that row names.
  - `helm(word)`: the direction of this think's burn. `speed(n)`: its strength, `n` cm/s², as
    far as her drive gives; below nought, the other way. With no helm word the burn is toward
    her lock; with neither, nothing. `speed(0)` and `hold()` cut the drive. A burn stands
    until a later think changes it.
  - `emcon(true)` is COMBAT; `emcon(false)` is the quietest posture the seat allows.
  - `fire(id, slot)`: the mount `id` fires at her lock. Each `fire` of a tube is one torpedo of
    one salvo. A shot with no lock is dropped: the applier never picks a target.
  - Every other intent is dropped.
- **Seats.** The applier is one function; a `Seat` says what its actor may do. `Seat::PATROL`
  and `Seat::RAIDER` fire as their doctrine decides. `Seat::ABSENT` is a hull flown for an
  absent crew: every lock and shot is dropped (`FireRule::Never`), and every burn but one
  that takes her drift against the ship stopping her off (`BurnRule::Toward`), so doc 35 §11
  `B-4` holds whatever source thinks: she never fires and never runs a picket under power.
- **The one view (`view.rs`, new).** `space_mind_view` builds what every space mind reads.
  Beyond the arena's view it carries a `SpaceView`, hung on `MindView::space`:
  - her own velocity as a vector, her pitch, what her drive gives now, her reaction mass, the
    change of speed her tank holds and the reserve she must keep, and which of the four
    emission postures she holds;
  - the stop a picket or an interdictor has on her (`none`, `hailed`, `hove_to`, `cleared`,
    `ran`), the contact that holds it, and the furthest of those words a picket has spoken on
    open radio in the bubble (a hull that holds no contact still hears it);
  - for each contact: its elevation, its velocity as a vector, the posture its emissions read
    as, and the stop it is under.
- **Contact ids.** Each observer has a `ContactBook`, kept in its mind's record
  (`corvette_contacts`, `raider_contacts`, `interdictor_contacts`, `mind_contacts`). A ship is
  numbered the first time that observer's sensors hold it and keeps the number for the life of
  the bubble; its designator letter is the number's. A ship she has never detected has no
  number and moves no other. The view's `lock` is now the hull's own lock; it used to be the
  nearest contact whatever she was locked on.
- **Four kernels, and the mind ABI at `api=8`** (`crates/tacsim/src/minds/kernels.rs`,
  `mod.rs`): `space(v)` and `space_contact(v, id)` answer the rows above as maps, `()` on the
  ground; `course(bearing_mdeg, elevation_mdeg)` and `course_of(dx, dy, dz)` answer the helm
  word for a direction. No getter, intent, refusal or snapshot format moved.
- **The hooks are gone.** No Rust reads a trace word. `sync_corvette_cleared_stage_in_memory`
  (the write) and `read_corvette_stage_from_memory` (the read) are deleted: the corvette reads
  each contact's stop from its view, and the picket's own rules (hove to under the heave-to
  speed, run past the boundary) are settled on the bubble's state in `settle_picket_stops`.
  With them went what Rust decided for the minds: the raider's uncloak, the interdictor's
  forced posture and its fixed burn, a default target for a shot with no lock, the raider's
  press attack and the interdictor's tube gate. Each is in its doctrine now.
- **The doctrines** (`crates/tacsim/content/minds/`), all four rewritten to steer and fire by
  intents alone:
  - `kirkwood_picket.rhai` reads each contact's stop; it keeps one word of memory, that it has
    hailed.
  - `belt_raider.rhai` lurks dark until the radio says customs have finished with a carrier,
    lights up, and on the next think opens on the nearest hostile customs have finished with.
    It presses only a prey that is radiating for a fight, as the Rust hook did.
  - `interdictor_patrol.rhai` hails on its first think of a hull that is not radiating for a
    fight. After that it reads the hull's velocity: one whose change of velocity is against
    its drift past the interdictor is slowing, and the interdictor holds its fire and its
    drive for it. Any other hull it closes on, along the line to it, and fires at.
  - `linkdead_return.rhai` computes its own brake from the view: the way the stopping ship
    moves against her, sized to end her drift in the step ahead.
- **The stand-down, hull by hull, on state** (`tick.rs` `settle_interdictor_stand_down`;
  `crates/tacsim/src/arena/party.rs` `Relations::stand_down`). A carrier under the heave-to
  speed against an interdictor that still stands is stood down: the interdictor's party and
  hers clear their marks of each other and forget that the other fired, both ways, and no
  third party's book moves. `ActiveInterdictionBubble::stood_down` records each hull once. It
  no longer sets the bubble's `cleared`, which let every carrier go.
- **The contract test** (`crates/mush-contract-tests/tests/engine_contracts/space_mind_memory.rs`):
  nothing under `space/` names `encode_memory`, `decode_memory` or `mush_mind::memory`, and
  every assignment to a mind's memory field has its own think's `memory` on the right.

**What the survey had wrong, and what it did not see.**

- "Three older appliers" were three NPC mappings and MW.8's seat, so four. The drone has
  none: its think's intents are discarded (`mining.rs`, `commands.rs`) and its yield is a
  timer. Not changed here; MW.12.
- The memory hook was two hooks, a write and a read. The read drove the inspection status on
  a path only the unit tests reached (`carriers` empty); production always passed carriers.
- **Nobody could heave to against the interdictor.** It un-ghosted, burned at the carrier at
  10 m/s² from its first think and never stopped, and a carrier enters its bubble 45 m/s off
  it with a drive that gives under 1 m/s². Its doctrine stood down a target that was not
  moving, which never came true. "A Dray can heave to against the interdictor" therefore
  needed the interdictor's doctrine as well as the Dray's: it hails first and holds for a
  hull that is slowing.
- The detection of a contact never depended on its index (the object ids handed to
  `evaluate_space_contact_detection` are not read by the resolver). Only the ids and the
  designator letters leaked.
- **What an ABI move costs.** A seal's digest covers its ABI and its source, and the arena
  folds each seal into its state hash. So an ABI move re-pins: the nine default seal
  identities (`match_/seal.rs` `m5rules_default_seal_identities_are_pinned`); the stock
  mind's digest, because `tests/domain_arena/content.rs` holds its manifest to `API_VERSION`
  and its header says the API; and the arena pins that fold a Standard seal (six tests in
  `tests/domain_arena/` `minds.rs`, `motor.rs` and `reflex.rs`: three state hashes and the
  `hash` of 84 pinned ledger lines), 58 values re-pinned from one Linux run by script; and
  the four rows of the cross-platform corpus (`minds_rhai.rs` `M2_CROSS_PLATFORM_PINS`),
  whose test is `#[ignore]`d and runs only in CI's `tacsim-minds` cell. Nothing an arena
  does moved: the ledger lines differ in `hash` and in the `mind_sealed` lines' `source`
  alone, and the corpus's four stopping times are as they were. The three house minds' envelopes and manifests and the
  acceptance mind's source name the ABI too.
- A changed burn is applied by the bubble as a one-second impulse on the step it changes, on
  top of the thrust it then integrates (`bubble.rs` `command_burn_mmps2`). A doctrine that
  re-aims every think therefore gets about twice what it asked. Not changed; the interdictor
  and the raider ask for their closing burn once and let it stand.
- Relation changes (`set_party_relation`, and now `stand_down_parties`) are made on the bubble
  directly, not through the tape, so a replay from a snapshot does not reproduce them. That
  was so before this unit.

**Defaults taken.**

| | Default | Cost to reverse |
|---|---|---|
| 1 | **The ABI moved to `rhai=1.26.0;api=8;stock=3`**, once. Every sealed mind, compile cache key and resume check carries it | **Expensive.** Doc 45's AP.3 moves it again; nothing here needs another move |
| 2 | Space facts reach a script through kernels that answer maps, not through getters on `View` and `Contact`. A getter needs a row in the keyframe parity table and a field on the ground keyframe, and the ground has neither | Medium: it is API. A script reads `space(v).stop`, not `v.stop` |
| 3 | **The helm word.** `helm(mdeg)` carries one number; in space it is the bearing plus a whole turn for each tenth of a degree of elevation. A plain bearing is a level burn, and the arena, which reads a heading modulo a turn, sees the bearing. Scripts use `course` and `course_of`, never the packing. The cleaner shape is a pitch on the intent, which is a new field on a carried enum and a snapshot format move in the arena | Low for scripts; the format move is the cost of the cleaner shape |
| 4 | The stand-down is the host's rule, as a customs clearance is. A doctrine cannot refuse it, because no intent names it. Each hull is stood down once; one that fires afterwards is hostile again and is not offered a second | Low |
| 5 | The interdictor hails first and holds its fire for a hull that is slowing. A hull already radiating for a fight is not hailed. With nobody hostile in sight it shows itself at STANDARD, so it is never an unseen blocker. This changes play: a hull that brakes is not fired on, and its first volley is one think later | Low: it is the doctrine |
| 6 | `emcon(false)` is the quietest posture the seat allows: GHOST for a raider, STANDARD for the others. It used to be two meanings in two mappings. A mind cannot choose PASSIVE: the intent is a switch | Low |
| 7 | A hail and a clearance are open radio: every hull in a bubble reads the furthest word spoken (`space(v).heard`), whether or not it holds the ship spoken to. It is how the raider knows when to light up | Low |
| 8 | The absent seat lets through only a burn that heaves her to, and none when nothing is stopping her. Stricter than MW.8, which braked for her; never looser | Low: one rule in `backup.rs` |
| 9 | One heat reflex stays in Rust, the same for every seat that may fire: a hull over 650 per mille vents before she shoots. It was 600 for the corvette | Low |

**Left, and for which unit.**

- **Return fire in space (Q-44-38).** Not built, and not in this unit. The absent seat
  refuses every lock and shot. MW.7 is on main, so the follow-up can start; what it has to do
  is the section below. *Built since by doc 45 unit AP.9* (`45-cut-notes.md`, AP.9), for a
  hull her player left on `HOLD` or `GUARD` and flown by the house default: the record by
  ship, the third `FireRule` and the narrowed view are as the section below has them. On
  `RETURN`, and under a mind sealed aboard, she still never fires.
- **MW.10** (a doctrine by digest): every seal is still built with the compiled digest. The
  unit test selects a revision by building the seal with another digest and handing the think
  a store that holds its source.
- **MW.11** (thinks off the world thread): every think is still
  `MindPool::shared(1).submit().wait()`. The raider now thinks while she lurks, so a picket
  bubble has two thinks a step from the moment it opens. Relation changes are not on the tape.
- **MW.12** (drones): untouched; the drone's view has `space: None`.
- **MW.13** (a player's mind on a ship): the seat rules it needs are `Seat` and `BurnRule`.
  Q-35-B4 is still the owner's call.
- **No intent names** a stand-down, a coolant vent, one of four postures or the nose's
  attitude. Each needs a variant on a carried enum, so each waits for the arena's next
  snapshot format move. Deferred, with no unit.
- The view's mount rows are still placeholders (100 rounds, 850 per mille). They wait for the
  mounts' redesign.

**Return fire in space: what the follow-up has to do** (written after MW.7 merged, #549;
checked against `crates/tacsim/src/arena/absent.rs` and MW.7's "what a caller in space must
provide" above). The rule is `arena::absent::permits`, generic over whatever id a caller's
units go by, so space calls the field's rule and copies nothing. The seam left for it is
`FireRule` in `applier.rs`, which `apply_intents` asks at two places, a `lock` and a `fire`.
It still fits, with one change of shape (step 2).

1. **The record, which is the work.** `LocalSpaceBubble` gains, for each ship, the set of
   ships whose fire has arrived on her since the bubble opened, by ship id. It is written
   where the bubble resolves an arrival, hit or miss alike, as the arena writes at every
   impact: a beam's shot, the end of a slug's flight, the end of a torpedo's run. It is part
   of the bubble's snapshot, so a restart keeps it (doc 35 §11 `B-7`). A stand-down does not
   clear it: it is by unit, and what a stand-down clears is `Relations::fired`, which is by
   party.
2. **The seat.** `FireRule` gains a third variant that carries that set. `Seat` is `Copy` and
   borrows nothing today, so either it takes a lifetime or the set is handed to
   `apply_intents` beside it. `fly_in_bubble` passes `absent: true` (the seat is reached only
   when `backup_has_helm()`), `holds_fire: false` (space has no hold a crew can leave; when
   it has one, this is where it is read) and her set.
3. **The two doors.** At a `fire`: `permits(&facts, &Ask::Fire { at: her lock })`, and act
   only on `Ok`. At a `lock`: let through only a ship that is in the set. A lock is not a
   shot, but the bubble's fire commands take their target from it, so a lock on a hull she
   may not answer is the first half of one.
4. **The view.** `space_mind_view` shows such a mind as hostile only the contacts she may
   answer, and every other hostile as neutral, as `Arena::mind_view` does since MW.7.
   Narrowed, never widened, and the doors do not depend on it.
5. **Mining and trading.** The space seat has no intent that mines or trades. MW.12 gives a
   drone's mind intents that work a rock; where one is accepted for a hull whose crew is
   away it asks `permits(.., &Ask::Mine)`.
6. **Stories**, each failing before it: a hull whose crew has no link, fired on by the
   raider, answers the raider and nothing else; fired on by nobody, she answers nobody
   though her party holds the raider hostile; a restart keeps the record.

Two defaults it will have to take, flagged now. **What "arrived" means for a torpedo her
point defence stops**: counted above, because the arena counts a miss. **Whether the house
default asks to fire at all**: `linkdead_return.rhai` never does, so with the gate built the
house mind still fires at nothing, and only a mind a player sealed on her (MW.13) would use
it. That makes the follow-up worth building with MW.13 and not before.

**The acceptance, and what holds each.**

| The row asks | Held by | Before the cut |
|---|---|---|
| A doctrine changed to hold where it used to close makes its actor hold | `tick.rs` `interdictor_tests::a_doctrine_changed_to_hold_where_it_closed_makes_the_interdictor_hold`: the compiled doctrine closes along the line to a target abeam and above; a revision with `hold()` where that one has `speed(1000)`, named by its digest in the seal, leaves the same hull at rest with the same lock | The interdictor burned one fixed way whatever its doctrine asked, so the closing half fails |
| An escort that heaves to does not clear the runner beside her | The story `mw9_an_escort_that_heaves_to_is_stood_down_alone_though_she_fired_and_the_runner_is_not` (two chartered hulls, by commands), and the unit test `an_escort_that_heaves_to_is_stood_down_alone_though_she_had_fired` | One hull hove to cleared every party and marked the bubble cleared |
| After a stand-down a party that had fired is no longer hostile | The same story and unit test, and `arena/party.rs` `a_stand_down_ends_the_fight_for_the_two_it_names_and_only_those` | The marks were cleared and `fired` was left |
| A Dray can heave to against the interdictor, by a burn a mind computed from the view | The story `mw9_a_dray_whose_crew_has_no_link_heaves_to_against_the_interdictor_by_her_minds_own_burn`: her backup mind brakes her, she is never hit, she is stood down and takes up her passage | Her view said nothing of an interdictor; she coasted and was shot |
| Adding a ship the observer cannot detect leaves a mind's view the same bytes | `view.rs` `a_ship_the_observer_cannot_detect_leaves_her_view_the_same_bytes`, for three observers, with the unseen ship sorting first | Ids and designator letters were an index over every ship |
| No code outside a mind writes its memory | The contract test `space_mind_memory` | `sync_corvette_cleared_stage_in_memory` |

**Verified, and how.**

- **Mutation checks**, on Linux, each putting one removed behaviour back: the stand-down
  clearing every carrier; contacts numbered before the detection filter; the backup doctrine's
  burn given no direction; the picket's host writing the corvette's memory; a burn going one
  fixed way; the stand-down leaving `fired`. Each was caught by the test named for it above
  (the stand-down mutations by the unit test and by the story).
- **The whole of `tacsim`'s tests**, on Linux (conclav.dev), after the re-pin: 4,475 passed,
  31 skipped (the `#[ignore]`d ones).
- **The cross-platform corpus** (`m2_the_twin_corpus_hashes_the_same_on_every_host`, D-UM-28):
  the four re-pinned rows read the same on Linux and on Windows, byte for byte. The other two
  `#[ignore]`d steps of CI's `tacsim-minds` cell pass on Linux unchanged.
- **`tests/domain_world/`**, all 263 stories, among them. Three older stories were re-timed
  for the one think the hail and the raider's torch now take (`sfr6`, `sfr8`, `sfr20`), and
  one damage figure was re-pinned for the volley that no longer lands (`sfr15`).
- **`mush-contract-tests`**: 203 on Windows, 195 on Linux (eight are skipped there), with no
  budget re-blessed.
- `cargo clippy --all-targets -D warnings` on `tacsim` and `mush-contract-tests`, and
  `cargo doc -D warnings --document-private-items` on `tacsim`.
- **CI** run 43142475684 on `23b4ffcae`, the code as it stands, on main at `330a8b249`: every
  job green, the ten `tacsim` integration shards and `tacsim-minds` among them.

**The harness baseline, re-blessed by record** (doc 43 rule 8;
`crates/tacsim/tests/harness_baseline/arena.linux.json`, `mw9_deterministic_update`). The
dispatch-only harness job failed its delta on five fields of the deterministic block. A
deterministic-only update, as CF.0's and CF.6b's were; TS.21's timings and host stand.

| Field | Was | Is |
|---|---|---|
| `counters.ledger_bytes_streamed` | 1650150 | 1650916 |
| `minds.fill_authored.final_substep_hash` | 11296100550182435994 | 5028223047829299195 |
| `minds.stock_ci.final_substep_hash` | 10958828204349461749 | 2814354088987883261 |
| `minds.stock_authored.final_substep_hash` | 11620081165489262692 | 1771993770098865287 |
| `minds.ceiling_authored.final_substep_hash` | 13684314779734303388 | 1115581876497461230 |

- **Why the hashes moved.** Each folds the seals, and a seal's digest covers the ABI and the
  Standard source.
- **Why the byte count moved, and that nothing else did.** A ledger line writes the state
  hash as a decimal, and the hashes are different numbers with different lengths. Proved on
  conclav.dev, the blessing host, by dumping the arena lane's streamed ledger at main
  (`330a8b249`) and at this unit (`297e6161d`): 9,324 lines each, 9,323 hash values each, no
  line differs once `hash` and the `mind_sealed` lines' `source` are taken out, and the 766
  bytes are the digits of the hash values alone (180,132 to 180,898). The dump was a patch
  to the harness that was never committed. No count, outcome or stopping time moved: the
  other 101 fields of the deterministic block and every other field of the four minds
  blocks read equal.
- **Read on two hosts.** conclav.dev (by hand as `ghrunner`, nice 10, `HOSTNAME` unset) and
  GitHub's runner (dispatch run 43146168987) read the same five values. Against the updated
  file, on conclav.dev with the timing gate live: the deterministic block equal (106
  fields), the four minds blocks equal, p50 −0.5%, p99 +4.6% and the window p99 +6.1%, and
  the harness binary's 24 ordinary tests pass,
  `every_committed_baseline_agrees_on_the_deterministic_block` among them.

**The dispatch-only stories** (run 43146168987 on `297e6161d`; the pull-request CI does not
run them).

| Job | Result | Whose |
|---|---|---|
| 16b cut H TacSim harness | failed its delta | This unit's: the re-bless above |
| M8 tier 2, held minds sealed aboard | failed: `close_ic_mind` "is not currently offered" | Not this unit's. Main's dispatch at `bc52bf278` fails the same story; PR #554 (open) fixes it, and its branch's dispatch passes it |
| Release-profile build check (nightly) | failed: `deploy/instance/render.sh: INCOMPLETE … instance.env` | Not this unit's. Main's dispatch at `bc52bf278` fails with the same line; this unit touches nothing under `deploy/` |
| The M6 replay and author story, the M7 stories, the other M8 stories, the leak battery, the House doctrines' trials, the bot's round and win rates | passed, or still running when this was written | See the PR for the run on the head commit |

**Not verified.**

- **The `#[ignore]`d tests that no workflow runs** (the by-hand lanes: `minds_pool`'s gate
  lane in release, the cost lane's cohort, the field pulse at scale). Any of them that pins
  a hash that folds a seal has moved with the ABI and is not re-pinned here.
- Nothing was run against a live server or over SSH. The stories drive commands in the test
  rig.
- The think's cost. The raider's extra think a step is not measured; MW.11 measures.
- A sealed player mind on the backup seat: no verb seals one on a ship, so the seat's rules
  for it (`BurnRule`, `FireRule`) are pinned by the applier's unit tests only.
- An interdictor bubble with three or more carriers, and a picket bubble where the corvette
  is lost while a carrier is hailed (the stop then reads `none`; not pinned).

**Merged, after MW.7; nothing moved.** The owner merged main into the branch (`3b926fd35`:
MW.7 #549, the claim fix #554, the release smoke fix #555) and merged PR #551 as `9d8fc96b7`
on 2026-11-12. MW.7 landed first, and the two units met on main with no run on the
combination. Three runs on `9d8fc96b7` then read it:

| Run | What it covers | Result |
|---|---|---|
| Hosted CI, 43238006928 | `tacsim-lib` (the nine seal identities), `tacsim-minds` (the cross-platform corpus), the ten integration shards (the arena state hashes and pinned ledger lines, MW.7's two arena tests in `minds_rhai.rs`, its stories in `domain_field/field_mw7.rs`), lint, docs | green, all 25 jobs |
| Gate sweep on conclav.dev, 43238207078 | the workspace's tests on the blessing host | green |
| Gate harness on conclav.dev, 43238230594 | every harness lane under `TACSIM_ASSERT` and the delta against the baseline this unit re-blessed, timing gate live | green |

So no seal identity, arena state hash, pinned ledger line, corpus row or harness field
needed a second pass. The reason is in MW.7's note: it folds its new unit state (`absent`,
`fired_on_by`) into the arena hash only when it is set, and no practice match sets it. The
one conflict was in doc 35 §11.3, where MW.7's text stands for Q-35-B3 and this unit's for
Q-35-B4.

## MW.10

A space doctrine is named by the pack's cast with its digest; staff publish a revision with the
workshop's verbs and select it for an actor; the next bubble flies it (doc 44 §7.1, Q-44-25 as
corrected in §4a; first review finding 4). PR #562. Doc 35 §12 holds the rules. Paths are under
`crates/tacsim/src/` unless given.

**Built.**

- **The cast names a digest** (`content/space/picket_cast.json`, `space/picket.rs`
  `PicketCastEntry::doctrine_sha256`). The cast's actors are three: `corvette`, `raider`,
  `interdictor`. Each names its doctrine and the SHA-256 of that doctrine's source. A cast whose
  digest is not 64 lowercase base-16 characters does not load, like one that does not parse.
- **The store holds the shipped doctrines** (`minds/content/cast.rs` `boot`, called from
  `lib.rs` `on_world_loaded` before the space books are read back). The five shipped space
  sources are written as `tacsim:mindsrc:<sha256>` documents by the first boot that finds no
  document under the key, as Standard Doctrine is. Before this unit they were put in RAM at
  each seal site and never written.
- **One resolution** (`minds/content/cast.rs` `cast_doctrine`): staff's selection if the world
  holds one, else the cast's digest, and in both cases only if the store holds a readable
  source under it. Otherwise a `CastFault` whose text names the actor and the digest. A
  selection whose document does not read is a fault too, not "no selection".
- **Every cast seal site asks it, first** (`space/tick.rs` `build_interdiction_bubble`,
  `space/picket.rs` `cast_doctrine_in_world`). The doctrines a bubble needs (the corvette's
  and the raider's, or the interdictor's) are resolved before the carrier is checked out and
  before a hull is made, so a refusal touches nothing: no bubble opens and the carrier flies
  on. The seals are built with the resolved digests. No seal site of a cast actor names a
  compiled digest any more, and `KirkwoodPicketState::new` holds no seal until the spawn
  gives it the ones the world took.
- **A think runs the seal's digest and nothing else.** The three step functions passed the
  compiled digest as the one to use when the record held no seal. They now read the seal; a
  record with no seal does not think.
- **A reload keeps the seal** (`space/picket.rs` `reloaded_seal`, `space/tick.rs`
  `boot_read`). The bubble's record holds each seal's digest and the reload rebuilds a seal
  from it. The hull's attribute, which is the whole seal, is taken only when it is that same
  seal (unit, sortie and digest). The reload used to replace a missing attribute with a seal
  of the compiled digest, over whatever the record held.
- **Refusal at load.** `boot` resolves every actor once the store is up. Each refusal is one
  `error!` line and one entry of `TacsimStore::cast_faults`. The boot goes on.
- **`@mind/cast`** (`minds/content/cast.rs` `command`; the switch is in
  `minds/workshop/commands.rs`). Staff only, by `/live`'s gate (`SIM_ADMIN` or Royalty).
  Bare, it lists the three actors. `<actor>=<owned mind>@<revision>` selects a revision the
  asker saved; `=<digest>` any source the store holds; `=PACK` clears, and deletes a selection
  that did not read. It refuses a digest the store does not hold and a source that does not
  compile. The write is one pointer document, `tacsim:doctrine:cast.<actor>` (`store.rs`
  `put_doctrine_pointer`), or its deletion.
- **A closed bubble says what flew it** (`space/tick.rs` `CompletedInterdictionBubble::flown`):
  each NPC actor's doctrine name and digest, taken from the seals when the bubble closes.

**Where the selection lives, and why.** In the world's own documents, as
`tacsim:doctrine:cast.<actor>`: the key family and the shape (`{"sha256": …}`) of MW.8's house
default pointer, read by the same loader arm.

- It survives a restart: the boot reads it with the rest of the store.
- It survives a pack upgrade: it is not content. After an upgrade the selected digest still
  flies, and `@mind/cast` shows it beside the digest the cast now names.
- Rejected, the cast file: content is compiled into the binary, so staff could not change it
  on a running box, and an upgrade would overwrite it.
- Rejected, an attribute on the NPC hull or its commander: those objects are made when a
  bubble first opens, are reused by name, and are not there for the boot to check.
- Rejected, a Roll record (a filing or a licence): a selection grants nothing and is owned by
  nobody, and the Roll's records are both.

No pack, lane, relay or snapshot format version moved. Two documents gained a field: a cast
entry's `doctrine_sha256` (required in the cast file, which is compiled in) and a completed
bubble's `flown` (optional; absent on older records).

**What the survey had wrong, and what it did not see.**

- The survey's list of cast actors ("the picket, the interdictor, the raider, the escort") has
  one that does not exist. There is no escort in the cast: an escort is a player's hull.
- "Publishing other bytes under A's digest" cannot be said through any verb. The store's key
  is the SHA-256 of the bytes, so other bytes are another key. The only way other bytes come
  to sit under A's key is a document written from outside, and the store's answer to that is
  at load: it does not hold the document, and it never writes the key again. Both are proved.
- **A replay reads the tape and nothing else.** `BubbleInputTape::replay` applies the recorded
  commands to the first snapshot. It takes no store and runs no think, so no selection can
  reach it. That half of the row held on main already, by construction.
- **A closed bubble kept nothing to replay and did not say what flew it.**
  `CompletedInterdictionBubble::from_closed` replaces the tape with an empty one over the
  final state and drops the seals. `flown` is the record this unit adds.
- The compact seal does keep the digest. What it drops is the class, the principal and the
  channels, which are constants for a cast actor (class I, no principal, helm and sensors).
  That loss matters to a player's seal, not to these: MW.13.
- **The reload could change an open bubble's mind.** If a hull's seal attribute was missing,
  `boot_read` built a seal with the compiled digest over whatever the record held. And an
  attribute that was there won over the record without being compared with it, on a hull
  that is reused by name. Both fixed here.
- **The shipped doctrines were never in the persisted store.** `install_mind_source` is the
  RAM-only fixture path; production called it at every seal. So "the store" a space think
  read was the binary's bytes by another route.
- `source_lookup` still answers the binary's bytes for a shipped digest the store does not
  hold. That is what lets a rig with no world think, and an open bubble keep thinking if its
  document is lost. It can only ever answer the bytes that hash to the digest asked for. A
  new seal does not use it: `cast_doctrine` asks the store alone.
- **The carrier is checked out before the bubble's NPC hulls are made**
  (`build_carrier_ship_state`), and a spawn that then fails leaves her checked out with no
  bubble: nothing checks her back in. That was so before this unit for the rare failures
  (a hull that will not create, a unit that will not check out). This unit's refusal is not
  one of them: it comes first.

**Defaults taken.**

| | Default | Cost to reverse |
|---|---|---|
| 1 | **An actor whose doctrine the store does not hold does not sail, and the boot goes on.** No bubble opens for her; the carrier crosses unstopped. The alternative is to refuse the boot, as a pack that does not load does. Taken because the store's rule is that a document it cannot use never fails the boot, and because the fault is one actor's | Low: one call in `on_world_loaded` |
| 2 | A selection the store does not hold, or whose document does not read, is refused the same way; it does not fall back to the cast's doctrine | Low |
| 3 | The selection's key is `tacsim:doctrine:cast.<actor>`, by actor, not by doctrine name. Two actors that name one doctrine are selected apart | Low: no world holds one yet |
| 4 | The pointer holds a digest and nothing else: not who selected it or when. The selection is logged at `info` | Low: a wider document needs the loader arm to read it |
| 5 | `@mind/cast` writes its pointer in the handler, as the workshop's draft verbs write theirs; it submits nothing, so it does not end `Queued.` The handler note is in `legacy-handlers.toml` | Medium if selections must become a typed, audited family |
| 6 | `<owned mind>@<revision>` needs the revision named. A bare name is the working copy, which is not stored | Low |
| 7 | Selecting checks that the source compiles, and nothing more. A doctrine that compiles and faults at its first think leaves its actor inert for that bubble. A digest selects any source the store holds, whoever published it and whatever it was written for | Low |
| 8 | The seal of a selected revision still reads `origin: Doctrine { revision: 1 }` and the cast's doctrine name. The digest is what tells revisions apart | Low |

**Left, and for which unit.**

- **The backup mind's pointer has no verb** (doc 35 Q-35-B2). `put_doctrine_pointer` writes
  any role, and the listing would take a fourth row. Not built here: its rule is different
  (read on every think, with the compiled default behind it). Deferred, with no unit; MW.13
  touches that seat next.
- **MW.11** (thinks off the world thread): untouched. A seal is still resolved on the world
  thread when a bubble opens, which is one map lookup.
- **MW.12** (drones): the drone's seal is still built with `drone_mining_digest()`
  (`space/commands.rs`, and the reload in `space/tick.rs`). When the drone becomes a sortie,
  it takes a row in the cast and `cast_doctrine` resolves it.
- **MW.13** (a player's mind on a ship): the seal a reload rebuilds from the record is class
  I with no principal. A player's seal needs the hull's attribute, or a wider compact record.
  And an NPC hull that is salvaged keeps its `tacsim:mind_seal` attribute, which the backup
  mind tries first (`space/backup.rs`); that was so before this unit, and the seal it finds
  can now be a revision staff selected. Clearing the attribute at check-in is MW.13's.
- **A spawn that fails after the carrier is checked out** leaves her checked out (above).
  Not changed. Deferred, with no unit: it wants one check-in on every `None` of
  `build_interdiction_bubble`.
- **House policy and port arms** (Q-44-32, deferred): a role under `tacsim:doctrine:` and a
  digest in the store is the path they would use; nothing here is specific to space but the
  three actor names.
- **The cast is still compiled in**, not in the lane pack (doc 35 `D-35-17`, owed since
  SF.R30). Moving it is a lane format move and was not needed for this unit.

**AP.3 and the ABI.** No story or test of this unit pins a digest: each works it out from the
shipped bytes or reads it from the revision it saved. One pin moves with a shipped space
doctrine's bytes: the three `doctrine_sha256` values in `picket_cast.json`. If the `api:` line
of `kirkwood_picket.rhai`, `belt_raider.rhai` or `interdictor_patrol.rhai` is rewritten, the
unit test `the_packs_cast_names_each_shipped_doctrine_by_its_digest` fails and prints the
value to write. Whichever of AP.3 and MW.10 merges second merges main in and re-pins.

**The acceptance, and what holds each.** The two stories are in
`crates/tacsim/tests/domain_world/voyage.rs`; the unit tests in `minds/content/cast.rs` and
`space/tick.rs` (`interdictor_tests`).

| The row asks | Held by | Before the cut |
|---|---|---|
| Staff publish revision B and select it; the next picket behaves as B | `mw10_staff_publish_a_revision_and_select_it_and_the_next_interdictor_flies_it`: B is the shipped interdictor doctrine with `hold()` where it has `speed(1000)`, published with `@mind/new`, `@mind/paste`, `@mind/save` and selected with `@mind/cast`. The second hull's bubble is sealed B, and its interdictor takes her and holds for four thinks, where A closed on its second | Every seal named the compiled digest and no verb selected another |
| A bubble that was open behaves as A | The same story: B is selected after the first bubble's first think; on the next the interdictor closes, which only A does. The seal reads A to the bubble's end, and after a restart | — |
| A replay of an earlier one behaves as A | The same story: the open bubble's tape replays to the same burn and lock after the selection. Its closed record names A (`flown`). And `a_doctrine_changed_to_hold_where_it_closed_makes_the_interdictor_hold` | The replay held by construction; the closed record named nothing |
| Publishing other bytes under A's digest is refused | `mw10_other_bytes_under_a_digest_are_refused_and_a_cast_the_store_cannot_serve_does_not_sail`: published, other bytes take their own digest and A's document is untouched; written under A's key, they do not load and the key is not rewritten. `the_store_refuses_other_bytes_under_a_digest` | The store's rule was there; nothing proved it for a doctrine |
| The selection survives a restart | Both stories | No selection |
| A cast naming a digest the store lacks is refused at load, in a line that names the actor and the digest | The second story: `cast_faults` holds one line with `interdictor` and A's digest, `@mind/cast` shows it, the carrier crosses with no bubble and in nobody's custody, and selecting a held revision ends the refusal. `the_packs_digest_is_refused_when_the_store_does_not_hold_it`, `a_selection_wins_and_an_unheld_one_is_refused_by_name`, `a_selection_that_does_not_read_is_refused_and_not_taken_for_none` | The compiled digest was sealed whatever the store held |

**Verified, and how.**

- **Mutation checks**, on Linux (conclav.dev), each putting one behaviour back or taking one
  rule out, against the two stories and the unit tests. All eleven were caught:

  | Mutation | Caught by |
  |---|---|
  | A selection is never read (main's behaviour) | Both stories; `a_selection_wins_and_an_unheld_one_is_refused_by_name` |
  | A live bubble's think follows the cast, not its seal | The first story: "the open bubble's interdictor closes, as A does" |
  | The reload seals an open bubble again from the cast | The first story: "the open bubble comes back with the seal it took" |
  | A digest the store does not hold falls back to the binary's bytes | The second story; two unit tests |
  | The store holds bytes that do not hash to their key | The second story; `the_store_refuses_other_bytes_under_a_digest` |
  | The boot does not check the cast | The second story: no line in `cast_faults` |
  | A closed bubble does not record what flew it | The first story; the unit test in `tick.rs` |
  | A refused doctrine opens the bubble on the compiled digest | The second story: "no bubble opens for an interdictor with no doctrine" |
  | The reload takes whatever seal the hull's attribute holds | `a_reload_keeps_the_seal_the_bubbles_record_names` |
  | A selection that does not read is taken for none | `a_selection_that_does_not_read_is_refused_and_not_taken_for_none` |
  | The carrier is checked out before the doctrines are resolved | The second story: "her unit is in nobody's custody" |

- **A second reader** (an Opus agent, the diff only, nothing run) found three defects, all
  fixed and each now held by a mutation above: the first cut checked the carrier out before
  it resolved the doctrines, so a refusal left her in a bubble's custody with no bubble; a
  selection whose document did not read was taken for no selection and flew the pack's
  doctrine; and the reload preferred the hull's attribute to the bubble's record unseen.
- **The whole of `tacsim`'s tests on Linux** (conclav.dev), on `5eccf1c01`, which is this
  unit with main merged in at `0bad68f0b`: 4,520 passed, 31 skipped (the `#[ignore]`d ones).
- **On the same commit there**: `mush-contract-tests`, 195 passed and 8 skipped there, no
  budget re-blessed; `cargo clippy --all-targets -D warnings` on `tacsim` and
  `mush-contract-tests`; `cargo doc -D warnings --document-private-items` on `tacsim`;
  `cargo fmt --check`.
- **`mush-contract-tests` on Windows**, the first cut: 203 passed.
- **CI on `5eccf1c01`**: the pull-request run 43258253021 is green but for `test (bot)`,
  which is red on main since `668425fab` (`mine::minds::tests::absent_census`: AP.5's new
  sentence has no row in MW.5's table; PR #563 is the fix) and touches nothing of this unit.
  The dispatch of the dispatch-only jobs on the same commit, run 43258329034: every job
  green, the TacSim harness (no baseline moved), the M6, M7 and M8 stories and
  `tacsim-minds` among them, but for that `test (bot)` and one M8 job that never built (its
  runner could not download the toolchain). Main was merged in again once #563 had landed;
  the PR records the runs on that head.

**Not verified.**

- Nothing was run against a live server or over SSH. The stories drive commands in the test
  rig, the workshop's paste and save among them.
- The picket's two actors (the corvette and the raider) are selected by the same code as the
  interdictor and are covered by the unit tests, but no story selects a revision for either.
- A pack upgrade. The selection is not content, so an upgrade cannot touch it; no test boots
  a second cast file over a world that holds a selection, because the cast is compiled in.
- A selected doctrine that compiles and faults at its first think: the actor is inert for
  that bubble by the step functions' existing rule, which no test of this unit pins.
- The `#[ignore]`d by-hand lanes that no workflow runs.

## MW.11

The think's cost measured, then every space think taken off the world thread, onto the
server's one pool, with a runner kept for each seal (doc 44 §7.1, Q-44-28; first review
finding 5, second review row 17). PR #570. Paths are under `crates/tacsim/src/space/` unless
given.

**The rule for a late think, which is the arena's.** A think is collected on the snapshot a
step leaves and handed to the pool; its intents land at the head of the owner's next step,
through the one applier, before the bubble moves again. A step is not taken until the thinks
it owes have answered. If the pool has not answered when step N+1 is due, step N+1 waits and
the world thread does not: the tick asks whether the batch is done, finds it is not, leaves
that bubble exactly as it stands and goes on to the next. The held step is due at every
later tick and is taken at the first one that finds the answers in, as the step it was when
it first fell due (the same length, the same IC second); the wall time it waited is not
simulated, as frozen time and a stall past the step cap are not. So a late pool makes a
bubble late and never different, which is the arena's rule (`minds/pool.rs`: "a saturated
pool is seen as late wakes, never as a different result"). The arena can wait because each
match has a task of its own; a bubble steps on the world thread, so its wait is a held step.
The rule gave every item of the acceptance in space, so no other rule was considered.

**Built.**

- **The think host** (`thinks.rs`, new; `ThinkHost`). One for each world's store
  (`TacsimStore::space_thinks`). It holds the pool, a runner for each digest, and at most one
  set of thinks in flight for each owner. An owner is a bubble, a hull the backup mind flies
  outside every bubble, or a hull's drones at a cluster. `submit` hands them over, each
  actor's as a pool batch of its own; `settle` is what a due step asks: `Held`, or the
  answers in collection order. Nothing else under `space/` names the pool.
- **Two ways to settle, and who chooses.** The server's world loop calls
  `thinks::run_on_the_wall_clock` once before its first tick
  (`crates/mush-server/src/world_task.rs`, `run`); from then on the host holds a late step
  and never waits. A world no loop runs is a rig whose clock is moved by hand: no wall time
  passes between two of its ticks, so a held step there would only make a story count ticks
  by chance, and its host waits for its workers instead. It is the arena's own split: its
  task tests on a virtual clock wait for the real workers (`arena_task.rs`, `await_thinks`).
- **A bubble's step** (`tick.rs` `step_bubble`, lifted out of `step_active_bubbles` so a
  test can call it). In order: the answers of the last step's thinks land through the one
  applier, in the order they were collected; the crews' standing orders are applied; the
  bubble flies, and a carrier closing on a derelict is moved; the host's own rules are
  settled on that state (the picket's stops, the interdictor's stand-down); and the minds'
  next thinks are collected on it and submitted. Every mind of a bubble thinks on the same
  snapshot, and it is the state the step leaves: nothing moves the bubble after the
  collection but a crew's command. A bubble's opening is step nought: its minds' first
  thinks are collected when it opens (`collect_thinks_at_open`).
- **The picket and the interdictor** (`picket.rs` `collect_picket_thinks`,
  `deliver_picket_think`; `tick.rs` `collect_interdictor_think`, `land_interdictor_think`).
  The order the corvette's and the raider's answers are applied in is fixed when they are
  collected: the raider first once customs have ruled on a carrier, the corvette first
  until then, as before.
- **The backup mind in a bubble** (`backup.rs` `seat_in_bubble`, `collect_in_bubble`,
  `land_in_bubble`). The seat's rules (which burn it lets through, whether she is hove to)
  are read off the bubble when the answer lands, not as the think saw it.
- **The backup mind outside a bubble** (`backup.rs` `step_adrift`, `collect_adrift`,
  `land_adrift`). One tick collects her think, the next acts on it and collects again.
- **The backup seat's three sources are one ask.** Her own seal, the store's house default
  and the compiled default were tried one after another on the world thread, the next only
  when the last faulted. They are now one think on each source in the same batch, and the
  first that answers without a fault is hers.
- **The drones** (`mining.rs` `landed_drone_thinks`, `land_drone_think`; `picket.rs`
  `collect_drone_think`). The mining tick lands what the last tick collected and collects
  again. The launching command no longer thinks at all (`commands.rs`): a drone's first think
  is collected by the next tick.
- **The pool is the server's** (`crates/tacsim/src/minds/pool.rs` `of_world`, `PoolSlot`;
  `crates/tacsim/src/store.rs` `TacsimStore::mind_pool`). The store builds it with its own
  `[tacsim.minds].workers` the first time anything thinks. `MindPool::shared(workers)`, the
  process static sized by its first caller, is deleted, so no caller can name a size. The
  three places a match takes its pool (`crates/tacsim/src/arena_bridge.rs`) ask the store.
- **A runner is kept for each seal** (`thinks.rs` `ThinkHost::job`): by the digest the seal
  names, built at the first think of that digest on this server and shared by every hull
  sealed with it. The card, the empty rule program and the one-cell map every space think is
  held to are built once for the process. `run_sealed_mind_on_pool`, which cloned an arena,
  installed a runner and rebuilt the card and the program for every think, is deleted.
- **A restart** (`tick.rs` `ThinksOwed`, `ActiveInterdictionBubble::thinks_owed`). The batch
  in flight is in RAM; the bubble's record says which step collected the thinks it owes and
  the step length their views were told. A bubble read back with thinks owed and no batch
  in flight collects them again from the state it kept.
- **A crew that takes the helm back** (`tick.rs` `handle_crew_revived_or_connected`;
  `ThinkHost::forget`, `discard`): the think the backup mind had out for her is struck from
  the bubble's batch, or her own batch is dropped (Q-44-37). The other minds' answers of that
  batch still land.
- **The backup mind's think number** is a count on her record
  (`SpaceLinkdeadState::mind_think_seq`). It was the wall second, which a held step would
  have changed.
- **A contact that has left the bubble** between a think and its landing names nothing: an
  intent on it is dropped by the applier, as one on an unknown contact is.
- **A step that stays held is said in the log**: at the fifth tick and every sixtieth after.
- **Nothing is left behind**: a bubble that closes, a hull the seat no longer flies adrift
  and a hull whose cluster session ends each drop their thinks (`keep_bubbles`,
  `keep_hulls`).
- **The contract test**
  (`crates/mush-contract-tests/tests/engine_contracts/space_thinks_off_thread.rs`): under
  `space/` only the host names the pool, a batch handle or the inline runner; nothing runs a
  think where it stands or builds a runner; the host waits in one place, behind its hold
  check; and the server's world loop calls `run_on_the_wall_clock` once.

No config key, verb, player-facing sentence, pack, lane, relay or snapshot format moved, and
`MIND_ABI` is as AP.3 left it. Two records gained an optional field: a bubble's
`thinks_owed` and a link-loss record's `mind_think_seq`.

**A tape replay runs no think.** `BubbleInputTape::replay` and `replay_from_snapshot` apply
the commands on the tape to a snapshot and take no host, no pool and no store; that is as it
was. What a mind decided is on the tape as the commands the applier wrote when its answer
landed. `tick.rs` `mw11_a_replay_of_the_tape_reproduces_the_bubble_and_runs_no_think` holds
it.

**The cost, before and after.**

*The method* (`think_cost.rs`, an `#[ignore]`d lane; the recipe is in `ci.yml` and its
by-hand row in `test-tiers.toml`). One fixed bubble for each actor, stepped once so its
sensors hold their contacts; 200 thinks thrown away, then 5,000 timed with
`std::time::Instant` on the calling thread, each carrying the memory the last one left.
The calling thread stands for the world thread. Intents are not applied, so every sample
thinks on the same snapshot. Release profile. Before the unit the lane timed the whole
think as the step called it (`run_space_rhai_think_with_seal`) and its stages apart; since
the unit it times what is left on the world thread, the collect (the view, the job on the
kept runner, the hand-over) and the landing (asking the host, taking the answers), on a host
that holds as the server's does, and times the think apart as a worker runs it. The time
the lane spends spinning until a batch is done is not world-thread time and is not counted.
Before: the lane on `0d52a87d2`, the code as main had it. After: `556a20a27`.

*Linux* (conclav.dev, EPYC 9J45, 32 vCPU; two runs of each, medians in microseconds, both
runs given; the box's load average was 18 to 28 during the runs before and 6 during the
runs after, so read the medians and not the tails):

| One think of | Before: the world thread | of that, the runner and the job rebuilt | of that, the think | After: the world thread | After: a worker, the think |
|---|---|---|---|---|---|
| the corvette, 1 carrier | 365, 393 | 344, 346 | 6.0, 6.3 | 9.2, 8.9 | 11.9, 11.8 |
| the raider, 1 carrier | 383, 381 | 342, 344 | 3.6, 3.7 | 9.0, 8.2 | 5.8, 6.1 |
| the backup mind, on a carrier | 391, 398 | 345, 344 | 7.6, 7.5 | 9.2, 9.1 | 13.2, 13.6 |
| the corvette, 6 carriers | 407, 401 | 347, 345 | 16.1, 16.0 | 11.0, 10.7 | 21.4, 22.1 |
| the raider, 6 carriers | 384, 377 | 346, 342 | 3.8, 3.6 | 9.3, 4.8 | 6.0, 3.0 |
| the interdictor, 2 carriers | 412, 409 | 341, 343 | 15.1, 15.2 | 9.9, 9.4 | 28.1, 28.0 |
| a drone | 369, 374 | not split | not split | 8.1, 3.9 | not split |

| One step of a picket bubble | Before | After |
|---|---|---|
| its two minds, on the world thread, 1 carrier | 771, 779 | 13.5, 14.1 |
| its two minds, on the world thread, 6 carriers | 795, 793 | 18.8, 18.0 |
| the bubble's own physics, 1 carrier | 0.7, 0.7 | 0.6, 1.0 |
| the bubble's own physics, 6 carriers | 1.6, 1.6 | 2.0, 2.3 |

The 99th percentile of one think on the world thread was 430 to 590 before and is 10 to 18
after. Asking a batch that is still out, which is all a held step costs a tick, is under
0.1. After the unit the world thread's share is the collect (5 to 8, of which the view is
0.2 to 1.0) and the landing (0.4 to 2.8).

*Windows* (the owner's box, release, one run of each, medians):

| | Before: the world thread | After: the world thread | After: a worker, the think |
|---|---|---|---|
| the corvette, 1 carrier | 441 | 4.5 | 8.0 |
| the raider, 1 carrier | 446 | 3.4 | 5.6 |
| the backup mind, on a carrier | 449 | 4.5 | 9.3 |
| the corvette, 6 carriers | 454 | 5.7 | 16.9 |
| the raider, 6 carriers | 431 | 3.8 | 5.7 |
| the interdictor, 2 carriers | 463 | 4.8 | 17.9 |
| a drone | 425 | 1.3 | not split |
| a picket step's two minds, 1 carrier | 885 | 4.1 | |
| a picket step's two minds, 6 carriers | 908 | 10.8 | |

Before, 380 of each think's 440 or so was the runner and the job rebuilt, and the think
itself 7 to 22. The 99th percentile of one think on the world thread was 660 to 810 before
and is 5 to 12 after.

*What the figures say.* A think cost the world thread about 380 microseconds, and a picket
bubble paid twice that on every step, against under a microsecond of physics. Nine tenths
of it was not thinking: it was a cloned arena, an installed runner, a card and a program,
built again for each think. The think itself is 4 to 28 microseconds. With the runner kept
the whole of it would fit on the world thread at about 30 microseconds a think; it is off
the thread because the ruling says so and because a player's doctrine (MW.13) will not be
this small. The worker's figure reads higher after than before because the lane runs that
think a second time on a thread that did not run the first.

**What moved, and why.** Nothing an arena does moved: no seal identity, arena state hash,
pinned ledger line or corpus row is touched, and `MIND_ABI` is as it was. An arena's thinks
run as they did; only who holds its pool changed. What moved is in space, and for three
reasons, each of them the rule and nothing else:

1. **A think lands at the head of the next step.** A bubble's opening is step nought: its
   minds' first thinks are collected when it opens and land at the head of its first step,
   so a cast actor still acts on the first tick, on a snapshot one step older. The backup
   mind in a bubble thought before the step and still lands before it; only her first think
   after she takes the helm is a step later. Outside a bubble, and for a drone, a think is
   collected on one tick and lands on the next, where it used to do both on one. A drone's
   launch no longer thinks.
2. **Every mind of a bubble thinks on the same snapshot.** The corvette no longer sees in
   the same step what the raider just did.
3. **The interdictor waits one think for its hail to be answered**
   (`content/minds/interdictor_patrol.rhai`). Its hail lands a step after it thinks it, and
   the hailed hull's burn a step after that. Its old rule, to judge a hull on the think
   after the hail, now judged a snapshot in which the hull could only just have heard it:
   a hull whose mind braked at the first chance was closed on and fired at for one think
   (the MW.9 story of the Dray caught it: her drift against the interdictor rose from
   45,000 to 57,297 mm/s). The doctrine now holds for that one think (`heard` in its
   memory, trace `INTERDICTOR_AWAIT_ANSWER`). A hull radiating for a fight is still not
   hailed and not waited for.

Each value and each story that changed:

| What | Was | Is | Why, and the evidence |
|---|---|---|---|
| `content/space/picket_cast.json`, the interdictor's `doctrine_sha256` | `a16210e1…188925` | `da72cb9f…d760e13b` | Reason 3: the doctrine's bytes changed (twice: `45a5d733…ee4604` stood between, until the second reader's fix to the wait). `minds/content/cast.rs` `the_packs_cast_names_each_shipped_doctrine_by_its_digest` holds the pin to the file |
| `picket.rs` `kirkwood_picket_and_belt_raider_rhai_doctrines_run_on_rhai_runner_without_faults` | the corvette traced `PICKET_SCREEN_ENGAGE_RAIDER` on the step the raider lit her torch | it traces nothing on that step and screens on the next | Reason 2. The raider's own trace, posture and salvo are on the steps they were |
| `tick.rs` `a_doctrine_changed_to_hold_where_it_closed_makes_the_interdictor_hold` | the interdictor closed on its second think | on its third | Reason 3. The thrust it closes with, and the replay, are asserted as before |
| Story `sfr15`, the carrier's damage after four ticks under the interdictor | `drive 118/250, hull 471/600` | `drive 172/250, hull 524/600` | Reason 3: one volley fewer lands in the four ticks. The step is 54 off the drive and 53 off the hull, the same step MW.9's re-pin made (from `64/250, 418/600`) when the hail took the first think |
| Story `sfr20` | the pilot turns stern on tick 2, beams on 0, 2, 3, vents on 1, 2; four ticks | stern on 3, beams on 0, 3, 4, vents on 2, 3; five ticks | Reason 3: the interdictor's volleys land on ticks 2, 3, 4 where they landed on 1, 2, 3, and the pilot's turn and later shots follow them. The assertions (the stripped mount, the crit, the rear split, the bubble closing) are unchanged |
| Story `sfr13` | the torpedo is in the feeds of the first tick after she radiates | of the second, with her point defence on manual for the read | Reasons 1 and a consequence. A launch now lands before its step's physics, so a torpedo flies in the step it is launched in. **Evidence:** the same prints on `0d52a87d2` and on this unit give the same bubble at steps 2, 3 and 4 (range 17,301, 16,639, 15,745 m; the tube's magazine 7; her two point-defence mounts at 1,950 rounds). On main the torpedo was launched after step 1 and shot down during step 2; here it is launched and shot down during step 2. The only difference is that main's tick 1 showed it sitting at its launcher. With the point defence on AUTO no tick shows it, so the gunner holds it while the feeds are read |
| Story `sfr6`; story `sfr8` | one tick, then two, before the raider's salvo is asserted | two, then three | Reason 1: her torch and her salvo are two thinks, each landing a step after it |
| Story `mw10` | one second before the open bubble's interdictor closes | two | Reason 3 |
| Story `sfr14` | a drone's think count is at least 2 after one tick | exactly 0, 1 and 2 after one, two and three ticks | Reason 1, and no think at launch. The story now pins the latency itself |
| Stories `sfr23`, `sfr25`, `sfr32`, `sfr32b`, and the helper `wait_out_the_grace` | the tick the grace runs out on shows the seat's act | the tick after it | Reason 1: that tick collects her mind's first think, the next lands it |
| Story `cr6` | waits until nothing is in flight, then docks | waits until the raider no longer flies and nothing is in flight | Reason 1: there is now one second with the raider lit and her salvo not yet launched, where the old wait ended and the dock was refused (`SF.R9`). Before, her ordnance in flight carried the wait until the corvette had finished with her (nine steps later, the same fight) |
| Story `mw9_a_dray_whose_crew_has_no_link…` | passes unchanged in its own text | | Through the helper's second and reason 3; she is still never hit |

The other 255 of `tests/domain_world/`'s 266 stories pass as they were written.
No assertion was removed or loosened.

**What the survey had wrong, and what it did not see.**

- "Every think is `MindPool::shared(1).submit().wait()` each time a bubble steps" named four
  thinkers. There were six think sites: the three cast actors, the backup mind in a bubble,
  the backup mind outside one (`step_adrift`, on the space tick), and the drone in two
  places, the mining tick and the launching command (`commands.rs`). The last two are not in
  `tick.rs`, `picket.rs` or `mining.rs`'s step, and all six are converted.
- **The think was not the cost.** Nine tenths of what a think cost the world thread was
  rebuilding its runner and its job (the table above). The script itself was 4 to 17
  microseconds. The second review's row 17 had it right ("what costs is rebuilding the
  runner each think"); the size was not known.
- The backup seat tried up to three sources in sequence, each a full blocking think.
- The backup mind's think number was the wall second, and its view's clock outside a bubble
  is the wall second still (`tier1_view`). The first is fixed here; the second is what she is
  told the time is, and stays.
- **The IC second of a step is an input to the bubble.** `StepWall` carries the world's IC
  second and detection reads it (`is_bubble_ship_detected`). So a bubble held under a running
  clock is late against the world's IC clock: the held step keeps the IC second it fell due
  with, and every later step reads a later one than it would have. That is the same lateness
  the rule accepts for wall time, and it is why the determinism test with the clock running
  holds everything but the IC stamps equal.
- Two picket minds thought one after the other inside a step, and the second saw what the
  first had just done. No design asked for that; it fell out of thinking in a loop.
- The process static made "the configured number of workers" untestable in a process that
  runs more than one world, which every test binary does under `cargo test`.

**Defaults taken.**

| | Default | Cost to reverse |
|---|---|---|
| 1 | **A late think holds its own bubble's step; nothing else moves, and the world thread never waits.** The arena's rule (above) | **Expensive**, as Q-44-28 says: every space story's timing rests on it |
| 2 | **The rig's host waits and the server's holds, and the world loop is what says which.** The default, for a world no loop has claimed, is to wait. The alternative was a config key; a key that makes the server block if it is mistyped was not wanted. A new driver that runs a world on the wall clock must make the same call, and the contract test pins the one that exists | Low: one call |
| 3 | A held step is taken as it first fell due, and the wall time it waited is not simulated. The alternative, one long step for the whole wait, is a different step for a late pool | Medium: it is the determinism |
| 4 | **Every mind of a bubble thinks on the same snapshot.** The corvette no longer sees in the same step what the raider just did, nor the raider the corvette, nor the backup mind either of them. This changes play by one think: the corvette screens against the raider one second after her torch, not in the same second | Low in code, but it is the property that lets the thinks run at once |
| 5 | The pool is one for each store, not one for each process. A server has one store. A test process with many worlds has many pools, each dropped with its world | Low |
| 6 | A runner is kept by digest for the life of the server, and never dropped. A digest's bytes cannot change (the store is addressed by digest), and a server sees a handful of doctrines and their revisions | Low: a bound, if staff ever publish thousands |
| 7 | The first think of a digest builds its runner on the world thread: one compile for each doctrine revision in a server's life (the compiled script is cached by digest already) | Low |
| 8 | A drone does not think in the command that launches it. Its record starts at think nought with no memory and thinks on the next tick | Low |
| 9 | Drones and a hull adrift have nothing that steps, so "held" for them is that they do not think again until the last think has landed | Low |
| 10 | After a restart a bubble that owed thinks collects them again on the state it kept. If a crew commanded between the collection and the restart, the second collection sees that command and the first did not | Low: the alternative is to drop the owed think, which is a different run |
| 11 | A runner's panic leaves its own actor doing nothing for that step, with an `error!` line; it thinks again on the next. Each actor's think is a pool batch of its own, so the other minds of the bubble land as usual. The arena voids a match for the same panic; a bubble has no void | Low |
| 12 | A bubble held for a long time is logged and nothing more. Nothing closes it or clears it | Low. See "Left" |
| 13 | **What a mind launches flies in the step it lands in.** An answer lands before its step's physics, as the arena's does. Before the unit a cast actor's launch was applied after the step, so the crews' feeds showed a torpedo at its launcher for one tick before it moved; now the first tick that shows it shows it a second into its flight, and one that point defence reaches in that second is never shown. The alternative, to land answers after the step's physics, keeps that tick and puts every intent a step behind the state it is applied to. **The owner's to rule if the lost tick matters to play** | Medium: it is where in the step an answer lands, and every space story's timing follows it |
| 14 | The backup seat runs every source it holds on each think (her seal, the house default, the compiled default) and takes the first clean answer. With no sealed mind and no house pointer that is one think, as before; with all three it is three on the pool where it was one on the world thread unless one faulted | Low |
| 15 | A source that has no runner (not held, or it does not compile) is looked up again at each think and warned about each time. No seal site seals such a digest (MW.10) | Low |

**Left, and for which unit.**

- **MW.12** (drones as sorties): the drone's intents are still discarded and its view still
  has no rock and no hold. Its think now lands a tick after it is collected, which is where
  MW.12's intents will land. `collect_drone_think` is where the real view goes.
- **MW.13** (a player's mind on a ship): a player's seal is a new digest on the same host,
  with nothing to add here. It is the first source of the backup seat's ask already. Its
  rule program is still not bound: `thinks.rs` `KIT` binds an empty program for every space
  think.
- **AP.9** (space reads the posture): the posture reaches a backup mind's view at
  collection, in `collect_in_bubble` and `collect_adrift`.
- **MW.20** (minds measured at size): eight bubbles on one server is its row. The lane here
  measures one think and one step; the host's counters (`ThinkStats`: thinks, batches, held
  steps, runners built) are what a soak would read, and nothing shows them to staff yet.
- **MW.12 must give drones the held-step rule.** While a drones' batch is still out on the
  server's host, no further think is collected for them, so how many thinks a drone has
  landed when its sortie ends depends on the pool. Nothing reads a drone's intents yet, so
  no outcome moves; once MW.12 makes its yield depend on them, the yield has to wait for
  the think as a bubble's step does. The same is true of a hull adrift, whose act is one
  passage engaged a tick later.
- **No watchdog on a held bubble.** A think is bounded by its class's operation budget, so a
  runner cannot spin; a pool that never answers would hold its bubbles until a restart, and
  the log says so. Deferred, with no unit: it wants a ruling on what a bubble does when its
  minds are gone (fly on without them, or close).
- **Relation changes are still not on the tape** (MW.9's note). Unchanged.
- **The space journey over SSH and space on the nightly** are doc 35's (SF.R31's list).
  Nothing here was run against a live server.

**The acceptance, and what holds each.** The unit tests are in `tick_thinks_tests.rs`
(declared by `tick.rs`) and `thinks.rs`; the story is in
`crates/tacsim/tests/domain_world/voyage.rs`.

| The row asks | Held by | Before the cut |
|---|---|---|
| The measured cost before and after, with the method | The tables above; the lane `mw11_what_a_space_think_costs_the_world_thread` | Never measured |
| The same encounter, run again from before a decision with the workers' timing varied, gives the same commands, the same mind memory and the same outcome | `mw11_a_picket_is_the_same_whatever_the_workers_timing` (three minds: the corvette, the raider, a backup-flown carrier; customs rule mid-encounter) and `mw11_an_interdiction_is_the_same_whatever_the_workers_timing`. Each takes the encounter to a point where its minds have thinks out, then runs it on from there six ways: thinks run on the stepping thread; a rig's host on one worker and on four; the server's host on one worker and on three, every step's thinks parked while the tick finds the step held nought to three times, the clock standing; and the same with the wall clock running on under the held steps. The first five are held equal whole: every command, the bubble, each mind's memory, think count and contact numbers. The last is held equal but for the IC second on its steps | There was one way to run it |
| A restart between a think and its landing changes nothing | `mw11_a_bubble_read_back_with_thinks_owed_collects_them_again`: the record goes through its stored form and meets a host that has never seen it | Nothing was in flight across a tick |
| A server whose first think is in space has the configured number of workers | The story `mw11_a_server_whose_first_think_is_in_space_has_the_configured_workers` (`workers = 3`; the bubble's opening think is the server's first; the pool is three and is the one a match is handed) and `minds/pool.rs` `m4_a_dropped_pool_answers_what_it_queued_and_a_stores_pool_is_one` (1, 3 and 5) | The process static was sized by its first caller, and space asked for one |
| No think blocks the world thread, the drone's included | `mw11_a_step_whose_thinks_are_out_is_held_and_the_thread_goes_on` (two hundred due ticks with every worker parked: none takes the step, none writes anything, and the step taken afterwards is the one that first fell due); `thinks.rs` `mw11_a_holding_host_does_not_wait_and_a_held_step_keeps_its_length`; `mw11_every_think_of_a_bubble_runs_on_a_pool_worker` (thirty thinks, every one on a `tacsim-mind-N` thread); the contract test, which holds every think under `space/` to the host and the host's one wait behind its hold check; and `crates/mush-server/tests/world_task/arena_bridge.rs` `the_world_loop_tells_the_space_minds_to_hold_late_steps`, on the server's own loop. The drone and the hull adrift go through the same `settle` | Every think was a wait on the world thread |
| A runner is kept for each seal and not rebuilt each think | `thinks.rs` `mw11_a_runner_is_built_once_for_a_digest_and_kept` (a hundred thinks, one runner, the same `Arc` on every job and for a second hull of the same seal); the story (one runner over the encounter); the lane asserts one runner for 5,200 thinks | A runner, a card and a program for every think |
| A crew that takes the helm back discards the think in flight (Q-44-37) | `mw11_a_think_struck_from_a_batch_does_not_land_and_the_rest_do` | The think was never in flight |
| A tape replay runs no think | `mw11_a_replay_of_the_tape_reproduces_the_bubble_and_runs_no_think` | Held by construction, and still is |

**A second reader** (an Opus agent, the diff only, nothing run) found ten things. Fixed:

- **The interdictor's new wait could strand her with her drive cut.** The wait did not
  reset `closing`, and a wait left by one hull's hail could fire later against another hull
  that had been taken without a hail, after which she never closed again. The wait now
  resets `closing`, and a hull taken without a hail, and a station with nobody hostile,
  clear it. The doctrine's digest moved again with the fix, to the value in the table
  above.
- **One runner's panic took every mind of its bubble with it**, because a bubble's thinks
  were one pool batch and the pool loses a batch whole. Each actor's ask is now a batch of
  its own.
- **A bubble read back after a restart could collect on a later state than the first
  collection saw**: a carrier closing on a derelict was moved after the thinks were
  collected. The collection now comes after it.
- A hull that stopped being adrift, or whose cluster session ended, other than by the two
  ways already covered, left its thinks behind. `keep_hulls` prunes both each tick.
- Two stale comments in `voyage.rs` named the deleted `run_sealed_mind_on_pool` and
  `MindPool::shared`.

Answered, not changed:

- **A launch now flies a step before any crew's feed shows it** (the `sfr13` edit). True,
  and it is where the rule lands an answer. Default 13 above puts it to the owner.
- **A drone's think count depends on the pool while its batch is held.** True, and
  harmless until MW.12 reads a drone's intents; it is on the "Left" list for that unit.
- **A store replaced after the world loop has claimed it would wait again.** The store is
  replaced once, at boot, before the loop starts. Not guarded. And a pool whose host
  refuses it every worker thread runs a batch on the thread that submits it, as it does for
  an arena; that was so before.
- **Runners are never evicted, a source with no runner is looked up at each think, and the
  backup seat runs every source it holds.** Defaults 6, 15 and 14.
- **The determinism tests tick once a wall second**, so a held step followed by another
  cadence (the five-second cruise) is not exercised. See "Not verified".

**Verified, and how.**

- **Mutation checks**, on Linux (conclav.dev), each putting one behaviour back or taking
  one rule out, against the unit tests named for the unit. All six were caught:

  | Mutation | Caught by |
  |---|---|
  | A held step is taken as the tick that takes it finds it, not as it first fell due | `mw11_a_holding_host_does_not_wait_and_a_held_step_keeps_its_length` |
  | No runner is kept | `mw11_a_runner_is_built_once_for_a_digest_and_kept`, and five more (a test's parked runner is lost with the cache) |
  | The pool is one worker whatever the configuration says | `m4_a_dropped_pool_answers_what_it_queued_and_a_stores_pool_is_one` |
  | A struck think lands | `mw11_a_think_struck_from_a_batch_does_not_land_and_the_rest_do` |
  | A bubble read back with thinks owed does not collect them again | `mw11_a_bubble_read_back_with_thinks_owed_collects_them_again` |
  | An answer lands on the step that collected it | both determinism tests, the held-step test, the restart test and the struck-think test |

- **The whole of `tacsim`'s tests on Linux** (conclav.dev), on `2a0aa776f`, the unit with
  the second reader's fixes and before main was merged in: 4,558 passed, 33 skipped (the
  `#[ignore]`d ones). `tests/domain_world/`'s 266 stories are among them.
- **On the same commit there**: `mush-server`'s `world_task` `arena_bridge` module, 3
  passed (the world loop's claim among them); `mush-contract-tests`, 199 passed and 8
  skipped there, no budget re-blessed; `cargo doc -D warnings --document-private-items` on
  `tacsim` and `mush-server`; `cargo fmt --check`.
- **Main merged in** at `63dbcb3f6` (AP.4, #568; it touches nothing under `space/` or
  `minds/pool.rs`), as `8f434b94f`. On the merged tree: `cargo clippy --all-targets -D
  warnings` on `tacsim`, `mush-server` and `mush-contract-tests`, on Linux, clean after two
  findings were fixed; `mush-contract-tests` on Windows, 199 passed and 8 ignored.
- **The evidence for `sfr13`**: the same prints on main's code (`0d52a87d2`) and on this
  unit, on conclav.dev, by a patch to the story that was never committed. The figures are
  in the table above.
- **Two of the unit's own tests raced, and CI's four-core runner found it** (run
  43321942691, `test (tacsim-lib)`): the held-step test asserted that a worker had reached
  its parked think without waiting for it, and the pool-worker test read the workers'
  traces while the last step's thinks were still out. Both now wait for the worker. The
  unit's tests and every test under `space::` then passed pinned to one core six times
  running and to two cores three times, on conclav.dev (`taskset`).
- **The harness gate on conclav.dev**, run 43320927299 on `1298949b0` (the code as it is;
  the commits after it touch tests and this note): green, the timing gate live. No
  baseline field moved, so nothing is re-blessed.
- **CI on the pull request's head and the dispatch-only jobs**: the pull request records
  the runs.

**Not verified.**

- **Nothing was run against a live server or over SSH**, and no bubble has been held on a
  real server: the held step is exercised by unit tests on the bubble's own step function
  with parked workers, and the world loop's claim by one test on the server's loop.
- **A held step in cruise cadence, and a held step whose length differs from the step the
  views were told.** The determinism tests use the one-second cadence throughout.
- **The interdictor doctrine's stale wait** (the second reader's first finding) is fixed
  and is not pinned by a test of its own: no test takes a second hull in while a hail is
  out.
- **The picket's cast actors with a selected revision** under the new timing: MW.10's
  story selects for the interdictor only.
- **Eight bubbles at once, and a saturated pool on a real box**: MW.20's.
- **The Windows figures are one run of each**, on a box that was also building. The lane's
  figures were taken at `556a20a27`, before the second reader's change that gives each
  actor its own pool batch: a picket step now hands over twice where it handed over once.
  Not re-measured.
- The `#[ignore]`d by-hand lanes that no workflow runs.

## MW.12

Drones as mind-driven sorties (doc 44 §7.1, §4 F; Q-44-27; doc 35d §9, SF.R30's row). A
collector drone is flown by its mind: what it brings back is what it lifted and carried home,
and its sortie's record leaves the excursion book when it is back aboard. **The mind ABI did
not move**: it is `rhai=1.26.0;api=9;stock=3` as before. Paths are under
`crates/tacsim/src/space/` unless given.

**Built.**

- **A drone flies a bubble of its own** (`sortie.rs`, new; `DroneFlight`). `+mine/drones`
  opens, for each drone, a `LocalSpaceBubble` of three ships: the ship it left (`bay`), the
  rock it was sent to (`rock`) and itself (`drone`), in the frame that moves with the rock.
  The flight is kept on the drone's record in the cluster session
  (`mining.rs` `DroneSortieState::flight`) and is persisted with it; its tape is not, and is
  begun again on the bubble it kept, as a tape that has grown long is.
- **The view is the one view** (`view.rs` `space_mind_view`, through `picket.rs`
  `collect_drone_think`). Nothing was added to what a view exposes. The rock is a contact
  that is not friendly: its range, bearing, elevation and velocity, and `reads_dead` once it
  bears no ore. The ship is the contact that is friendly. The hold is the drone's one mount
  row, its collector: `rounds` is the lifts the hold still has room for, `rounds_max` the
  lifts a whole hold takes, `ready` whether it has cycled and has room, and its one band its
  reach. The drone's own velocity, drive and step are `space(v)`, as for every hull.
- **The intents go through the one applier** (`applier.rs` `apply_intents`, seat
  `Seat::DRONE`). `lock`, `helm`, `speed` and `hold` are the rows every hull has. One row is
  new: a `fire(id, slot)` that names a drone's collector is one command on the tape,
  `BubbleInputCommand::WorkRock`, which lifts a tonne from the rock the drone has locked.
  `Seat` gains `work`; no seat but a drone's has it. No intent kind was added.
- **The bubble's rules** (`bubble.rs`). `work_rock`: a lift needs the drone within 150 m of
  the rock and under 5 m/s against it, the collector cycled (3 s) and room in the hold; what
  leaves the rock's `ore_kg` is what enters the kit's `hold_kg`. `settle_drones`, in the
  step: a drone that has been more than 60 m from the ship it left and is back inside 60 m
  under 8 m/s lands, rides with her and does nothing more. A drone has a drive of 5 m/s² and
  400 kg of reaction mass, which bounds how long it can steer. A ship that is neither a
  drone nor a rock mixes nothing new into `state_hash`, and writes nothing new in its
  document.
- **The host's words are on the tape.** The ship is flown by her cluster session, and the
  deposit is the world's. Before a drone's think lands, the tick tells its bubble where the
  ship is (`PlaceShip`) and what the rock bears (`SetOre`), each as a command when it has
  changed. So a tape replays the whole sortie, the lifts and the landing included, with no
  world and no think.
- **The step** (`sortie.rs` `fly_drones`, called from `mining.rs` `step_clusters_on_tick`).
  A hull's drones step together, one second of bubble time for each space tick that finds a
  wall second gone. A late tick is one step, not a long one. In order: the answers of the
  thinks the last step collected are taken from the host; drone by drone, its bubble is told
  the ship and the ore and its answer lands; every bubble flies its second; the sorties that
  ended leave the session; the next thinks are collected on the state the step left. The
  held-step rule MW.11 left owed is the bubble's own: the step is not taken until its thinks
  have answered, on the server the tick goes on without it, and a session read back with
  thinks owed collects them again (`ActiveClusterMiningSession::drone_thinks_owed`).
- **The yield** (`mining.rs`). A drone that lands files one draw of what its hold holds,
  through the path a laser cycle uses (`tacsim_site_work`, deposit vault to hold vault). A
  drone still out after 240 steps is reeled in and files nothing. The tonnes a drone carries
  are held against the deposit while it carries them (`rock_uncommitted_pool_tonnes` now
  subtracts `drones_carried_kg`), so a laser or another hull cannot draw them; the deposit
  itself is debited once, by the landing's draw. A tonne lifted and not brought home never
  left the deposit. A landing also counts the draws already on their way into the hold as
  room taken (`hold_incoming_tonnes`).
- **The record closes** (`tick.rs` `SpaceBubbleBook::sorties_to_close`,
  `BubbleInputProvider`; `mining.rs` `close_landed_sorties`). A sortie that is over, by a
  landing, a reel-in or the ship flying on (`SpaceBubbleBook::end_cluster_session`, which
  every place that dropped a session now calls), is put on a list the book persists. Each
  tick the drone itself files `tacsim_space_bubble_input` with the switch `drones_land`,
  whose prepare removes the `tacsim.excursion` record only while it still names that sortie.
  The entry leaves the list when the book no longer holds the record. So a restart between
  the landing and the close closes it afterwards, and a close that lands twice removes
  nothing.
- **The doctrine is the cast's** (`content/space/picket_cast.json`, `picket.rs`
  `DroneCastEntry`, `crates/tacsim/src/minds/content/cast.rs` `CastActor::Drone`). The cast
  has a fourth row, `drone`, with the doctrine's name and digest. `+mine/drones` resolves it
  as every seal site does (staff's selection, else the pack's, from the store) before it
  makes anything, and refuses the launch in words if the store does not hold it. `@mind/cast
  drone=…` selects a revision. A drone that is out keeps its seal, and a reload takes the
  seal its record names (`reloaded_seal`), where it used to seal the compiled digest again.
- **The house doctrine** (`crates/tacsim/content/minds/drone_mining.rhai`), rewritten. It
  flies to the rock, lifts until its hold is full or the rock reads done, and flies home;
  after five lifts asked for and not given it goes home with what it has. It burns in
  pulses, a think on and a think off (see "what the survey did not see"). One line says
  when it has enough, `let enough = tool.rounds_max;`, which is the line the stories change.
- **A bay slot is launched again only when its drone is back**
  (`ActiveClusterMiningSession::free_drone_slots`). The slot of a new drone was the count
  of drones out plus one, so after one of two landed the next launch reused the slot, and
  the object, of the drone still out.
- **The absent player's rule** (`sortie.rs` `drone_seat`). The seat of a hull's drones is
  `Seat::DRONE`, without its collector while her crew is away: `arena::absent::permits` is
  asked to mine, as MW.9's note said this unit must.

**The mind ABI did not move, and why it did not have to.** The row asks for "a view of the
rock and the hold". Every fact the house doctrine needs was already exposed: a contact's
range, relation, velocity and `reads_dead`; a mount's `ready`, `rounds` and `rounds_max`; the
hull's own velocity and drive. What a move would have bought is the rock's ore as a number
and a `hold` with its own name. A move re-pins every sealed mind's identity, the arena's
state hashes and ledger lines, the cross-platform corpus and the retained-ABI table, and the
mining station of doc 17, which will define what a player's mining mind reads, has not been
designed. So the rows are reused, the reuse is said in the doctrine's header and in
`sortie.rs`, and the station can name them properly when it moves the API for its own
reasons.

**What moved, and why.** No seal identity, arena state hash, ledger line, corpus row or
harness baseline field moved. One digest is pinned for the first time, and one player-facing
sentence changed:

| What | Was | Is | Why, and the evidence |
|---|---|---|---|
| `drone_mining.rhai`'s digest | `95b4127f…ca3099d5`, named nowhere (every seal took `drone_mining_digest()`, the hash of the compiled bytes) | `0af0a496…6d6910b1`, named by `picket_cast.json`'s new `drone` row | The doctrine's bytes changed: it is rewritten. `minds/content/cast.rs` `the_packs_cast_names_each_shipped_doctrine_by_its_digest` holds the row to the file, and now covers the drone |
| `+mine/drones`' answer | `Launched N collector drone(s) onto Block A (S-TYPE) for 120s (4 t … each into stow.ore): …` | `Launched N collector drone(s) onto Block A (S-TYPE) for … ; each brings back what it lifts, up to 4 t, into stow.ore: …` | There is no timer and no fixed yield to state |
| Story `sfr5` | one six-second step after the launch lands 20 t (4 t laser, 16 t drones) | the step lands the laser's 4 t; the four drones are then flown home, a wall second a tick, and each is told back aboard with 4 t; the 20 t and every later assertion stand | The drones are sorties |

`sfr14` and `sfr22` pass as they were written: a drone still lands think `n - 1` after `n`
ticks, and a launch still stages its `RigRecord`.

**What the survey had wrong, and what it did not see.**

- **"The excursion book holds no drone" was not only tidiness.** `tacsim.excursion` is the
  surface's book too, and the surface's launch counts an operator's open excursions against
  `MAX_EXCURSIONS_PER_OPERATOR` (four). A prospector who had launched four drones, ever, had
  four records that nothing closed.
- **The bay slot was reused while its drone was out** (above). With timers that all ran 120
  IC seconds it could not happen; with sorties of different lengths it would have.
- **A drone's sortie was shorter than a tick.** 120 IC seconds is five wall seconds at the
  live rate of 24:1 and a thirtieth of a second at the stories' 3,600:1, so in every story a
  drone's sortie was over before its mind had landed a think. A mind cannot decide a yield
  on a clock it never thinks inside. The sortie is therefore on the bubble's clock, wall
  time, as every Tier-2 bubble is.
- **A changed burn takes effect twice.** MW.9's note has it: a changed burn is an impulse of
  one second on the command and then the thrust the step integrates. A burn that stands
  gives its strength for the step; one that changed gives it twice. A controller cannot be
  sized on that unless it knows which it will get, so the doctrine cuts its drive on the
  think after every pulse: each pulse follows a cut drive, is always a changed one, and
  gives `n` for a second and `n` for the step. The bubble's rule was not changed.
- **Two drones could lift the same tonne**, in this unit's first draft: the rock's ore was
  told to each bubble after the answers had landed, so two drones whose bubbles both showed
  the last tonne both lifted it. The unit test for it failed, on both platforms at the same
  step. The ore is now told before each drone's answer lands, drone by drone.
- **The reload sealed a drone again with the compiled digest** when its attribute was
  missing (`tick.rs` `boot_read`), over whatever the record held: MW.10 fixed this for the
  cast's hulls and left the drone.
- **A drone's objects are made anew at each visit to a cluster.** `drone_things` is the
  session's, and the session is dropped when the ship leaves, so the next visit makes four
  more DARK objects in the hull. Not changed. See "Left".
- The crew is rarely away while a drone is out: the link-loss grace is 300 s and a drone's
  endurance 240 steps. It takes steps that are not taken (held for a late think, a docking
  pause, a frozen timeline) for the grace to run out first. The rule is wired for those, and
  for the day either figure moves.

**Defaults taken.** Every figure is a named constant.

| | Default | Cost to reverse |
|---|---|---|
| 1 | **A sortie runs on wall time, a second a step, and takes about a minute.** The house drone's whole sortie from the 600 m standoff is 53 steps (44 with one lift). At the live rate that is 21 IC minutes, where it was two | **Medium.** It is the unit's shape: every drone story is counted in steps. The figures inside it are low |
| 2 | **The laser is now far quicker than the drones.** `+mine/laser` is still 4 t in 60 IC seconds (2.5 wall seconds at 24:1); four drones bring 16 t in about a minute. Not rebalanced here: the laser's timer is doc 35's and is as provisional | Low, and **the owner's to tune**: it is which tool a prospector reaches for |
| 3 | A hold of 4 t (`DRONE_HOLD_T`), a lift of 1 t, a collector cycle of 3 s, a reach of 150 m at under 5 m/s | Low |
| 4 | A drone drive of 5 m/s² and 400 kg of reaction mass, refilled at each launch and charged to nobody | Low; charging the ship's tank for it is a ledger debit |
| 5 | A drone lands inside 60 m of the ship under 8 m/s, once it has been further out | Low |
| 6 | **A drone still out after 240 steps is reeled in with nothing**, and the tonnes it held stay with the rock. The alternative, to land what it holds, makes the flight home optional | Low |
| 7 | **A ship that leaves the cluster leaves her drones behind**: their sorties end, they bring nothing, and their records close. Nothing refuses the departure, because the backup mind must be able to take a ship home | Low |
| 7a | **Inside the cluster she holds her station while a drone is out.** `+helm/approach`, and `+helm/match` on another rock, are refused (`commands.rs` `DRONES_OUT`): the cluster's approach moves her most of two kilometres a tick, and a drone could never make its bay | Low |
| 7b | **She launches from 250 m or more off the rock** (`DRONE_LAUNCH_STANDOFF_M`; an approach leaves her at 600 m). Nearer, her drones would lift without leaving the bay's reach and could not land | Low |
| 8 | The drone's row in the cast names a doctrine and a digest and nothing else, in the cast file with the three hulls | Low |
| 9 | **The hold and the rock are read through rows that already exist** (above). A script reads `v.weapons[0].rounds`, not a `hold` | Medium: it is what a drone doctrine is written against until doc 17's station |
| 10 | The rock tells a mind that it bears ore or does not, not how much | Low |
| 11 | A drone's doctrine is selected for the cast's drone, for every ship's drones at once | Low |
| 12 | The operator is told when each drone is back aboard, and when one is reeled in | Low |
| 13 | A landing whose tonnes the hold has no room for stows what fits; the rest never leaves the deposit | Low |
| 14 | A close whose record the book does not hold yet waits 30 ticks for it (`SORTIE_CLOSE_WAIT_TICKS`), and a close that did not land is filed again after 1, 2, 4 … 64 s. A launch that lands later than the wait leaves its record open | Low |

**Left, and for which unit.**

- **MW.13** (a player's mind on a ship): nothing here. A drone's seal is still class I with
  no principal.
- **A player-authored mining mind** is doc 17's first non-combat station (Q-44-27). It is
  where the hold and the rock's ore get names of their own, by an API move.
- **AP.9** (space reads the posture): a drone's view reads `RETURN`. A drone is nobody's
  standing order.
- **The drones' objects** (above). Deferred, with no unit: it wants the bay's drones kept on
  the hull, not on the session.
- **The laser against the drones** (default 2).
- **A drone is not a contact to anyone.** Its bubble holds three ships. A picket, a raider
  or another prospector cannot see or stop it. SF.R30's row did not ask for it.
- **The known defect** the brief named (a passage whose launch runs after its first leg's
  frozen end passes an interdicted waypoint unmet, PR #571) was not met: this unit does not
  touch the passage or the promotion.

**The acceptance, and what holds each.** The unit tests are in `sortie.rs` and `applier.rs`;
the story is in `crates/tacsim/tests/domain_world/voyage.rs`.

| The row asks | Held by | Before the cut |
|---|---|---|
| A drone whose mind is changed to stop short brings back less | The story `mw12_a_drone_whose_doctrine_stops_short_brings_back_less_and_no_sortie_stays_in_the_book`: staff publish revision B with `@mind/new`, `@mind/paste`, `@mind/save` and select it with `@mind/cast drone=`; the drone that is out keeps A and lands 4 t; the next is sealed B and lands 1 t. And `mw12_a_doctrine_changed_to_stop_short_brings_back_less` (4, 3, 2 and 1 lifts, each a shorter sortie) | Four tonnes at 120 IC seconds whatever the doctrine; no revision could be selected |
| The excursion book holds no drone after it lands | The same story: the record is in the book while the drone is out and gone when it is back, and the list of closes owed is empty | Every launch left a record for ever |
| Across a restart too, once and not twice | The same story: a restart with the sortie open, then a restart between the landing and the close with everything the landing tick submitted lost. The hold holds five tonnes, the record is gone, and six more ticks change nothing. `mw12_a_session_read_back_mid_sortie_ends_as_the_one_that_was_not` (read back after steps 1, 2, 9, 30 and two before the end) | — |
| The yield is what the drone did | `mw12_the_house_drone_fills_its_hold_and_lands_with_it`; `mw12_a_drone_that_does_not_come_home_is_reeled_in_with_nothing` (one that never asks; one that fills its hold and stays); `mw12_a_rock_that_runs_out_sends_the_drone_home_with_what_it_has` | A timer |
| Matter is conserved | The story: the deposit and the hold add up to what the deposit held, after each sortie and after both restarts, and nothing is credited while a drone is out. `mw12_two_drones_on_one_rock_never_lift_the_same_tonne` | The draw was conserved; the yield was not the drone's |
| The intents go through the one applier, the think through the one host | `a_collector_works_the_rock_its_drone_has_locked_for_a_seat_that_may_work`; the contract tests of MW.11 (`space_thinks_off_thread`) and MW.9 (`space_mind_memory`), unchanged and passing | The intents were dropped |
| The held-step rule | `mw12_a_step_whose_thinks_are_out_holds_the_drones` | Owed by MW.11 |
| A replay runs no think | `mw12_a_replay_of_the_tape_reproduces_the_sortie_and_runs_no_think`, after every step of a sortie | Held by construction |
| A late tick is not a long step | `mw12_drones_take_one_step_a_wall_second_and_a_late_tick_is_not_a_long_step` | — |

**A second reader** (an Opus agent, the range and the unit's brief only, nothing run) found no
loss or duplication of ore and no record closed twice, and eleven other things. Fixed:

- **A close could be dropped before its launch had staged the record.** A ship that launched
  and flew on in the same second put the close on the list; the next tick found no record,
  took the sortie for closed and dropped the entry; the launch then landed and staged a
  record nothing would close. The entry now waits for the record (default 14), and the story
  flies on before the launch lands.
- **Moving the ship inside the cluster stranded her drones silently.** Refused now (default
  7a). The story asks for both orders while a drone is out.
- **A drone launched while a batch was out thought a step sooner after a restart**, because
  the session read back collected for every drone it then held. A drone's record now says
  whether the batch that is out asked it (`DroneSortieState::think_out`), and a batch is
  collected again from those drones alone.
  `mw12_a_drone_launched_while_thinks_are_out_thinks_no_sooner_for_a_restart`.
- **A ship standing inside her drones' reach of the rock**: they lifted in the bay, never
  left it and could not land. The launch is refused there (default 7b).
- **A session was rewritten every tick while the timeline was frozen or she was docking**,
  by a wall-second stamp that did not need writing: a drone's step is a second however long
  it stood.
- **Two lines were past 120 columns**: the launch's answer, which is now two lines and no
  longer prints each sortie's id, and the line for a hold with no room.
- **A close the book refused was filed again every tick**, each filing one more entry in the
  bounded queue of bubble inputs that crews' helm orders share. It now backs off (default 14).
- Three slips in this note and one misplaced doc comment.

Answered, not changed:

- **A crash between the launch's action landing and the session's document being written**
  leaves a record with no drone and no close owed. True, and it is the window every module
  write beside an action has. Not reconciled.
- **A tape begins again every 64 entries**, so a replay covers the commands since it was
  last begun, not the whole sortie. As a bubble's does.

**Verified, and how.**

- **Mutation checks**, on Linux (conclav.dev), each putting one behaviour back or taking
  one rule out, as a one-file patch over the unit's own, against the unit tests of
  `sortie.rs` and `applier.rs` and, for the six that reach the world, the story with `sfr5`,
  `sfr14` and `sfr22`. All eighteen were caught. "The story" is
  `mw12_a_drone_whose_doctrine_stops_short_brings_back_less_and_no_sortie_stays_in_the_book`.

  | Mutation | Caught by |
  |---|---|
  | The yield ignores the mind: a landing files the whole hold whatever was lifted | The story: "B went home with one lift: one tonne more, drawn once" (8 t against 5) |
  | A think's intents are discarded, as before the unit | Nine unit tests, `mw12_the_house_drone_fills_its_hold_and_lands_with_it` first |
  | The sortie's record is left open (no close is filed) | The story: "the excursion book holds no drone after it lands" |
  | The selection is not read: a launch seals the compiled digest | The story: "the next drone is sealed with the revision staff selected" |
  | The closes owed are not persisted | The story: "the close is still owed after the restart" |
  | A held step is taken without its thinks | `mw12_a_step_whose_thinks_are_out_holds_the_drones`: "a held step moves nothing and writes nothing" |
  | A late tick is a long step | `mw12_drones_take_one_step_a_wall_second_and_a_late_tick_is_not_a_long_step` (7,000 ms against 2,000) |
  | The rock's ore is told after the answers land | `mw12_two_drones_on_one_rock_never_lift_the_same_tonne` (6,000 kg carried of 5,000) |
  | A reeled drone lands its hold | `mw12_a_drone_that_does_not_come_home_is_reeled_in_with_nothing` |
  | The seat of a crew that is away still works | `mw12_a_drone_whose_crew_is_away_lifts_nothing_and_comes_home` |
  | A collector works for any seat | `a_collector_works_the_rock_its_drone_has_locked_for_a_seat_that_may_work`, and the test above |
  | The ore is told to the bubble off the tape | `mw12_a_replay_of_the_tape_reproduces_the_sortie_and_runs_no_think` (step 25, slot 2) |
  | A drone lands without having left its bay | Eleven unit tests, `mw12_a_lift_needs_reach_and_a_cycled_collector_and_a_landing_needs_a_slow_return` among them |
  | A landed drone stays in the session | The story, `sfr5` and `sfr14` |
  | A close is dropped before its record is staged | The story: "a close whose record is not in the book yet waits for it" |
  | A session read back asks every drone | `mw12_a_drone_launched_while_thinks_are_out_thinks_no_sooner_for_a_restart` (step 58 against 57) |
  | The ship may move with drones out | The story: `+helm/approach B` is taken |
  | A launch from inside the drones' reach is allowed | `mw12_a_ship_inside_her_drones_reach_of_the_rock_does_not_launch_them` |

- **On Linux (conclav.dev), on the unit as it is**: every unit test of `tacsim` and all 267
  stories of `tests/domain_world/` (2,568 run, 7 skipped; one failed, the unit's own story,
  on an assertion about which refusal `+helm/match B` gives first, since corrected and run
  again with the space unit tests and the drone stories: 85 passed). On the unit before the
  second reader's fixes, `cost_lane`, `domain_arena` (1,127), `domain_economy` (588) and the
  first 296 of `domain_field` passed; that run was stopped there to free the box.
- **The memory-size clippy** CI's lint shard runs (`.github/clippy-m7`), on conclav.dev:
  clean.
- **On Windows**: `cargo fmt --check`, `cargo clippy --all-targets -D warnings` and
  `cargo doc -D warnings --document-private-items` on `tacsim`; the 80 unit tests under
  `space::` and `minds::content`; `mush-contract-tests`, 199 passed and 8 ignored, no budget
  re-blessed. The two unit tests that failed in the first draft failed at the same step, with
  the same figures, on Windows and on Linux.
- **`cargo test -p mush-bot`** on Windows: no failure in the part of its output that was
  kept. The pull request's `test (bot)` job is the check.
- **CI on the pull request's head and the dispatch-only jobs**: the pull request records the
  runs.

**Not verified.**

- **Nothing was run against a live server or over SSH.** The story drives the production
  commands on the test world; no drone has flown on a box.
- **The crew-away rule's wiring.** `drone_seat` is tested, and so is a drone flown on the seat
  it answers. The line in the mining tick that reads the hull's link state and asks it is
  reached by no test: see what the survey did not see.
- **The refusal of `+mine/drones` when the store does not hold the drone's doctrine**, and the
  refusal when she stands too near the rock. The first calls MW.10's resolution, which is
  tested for the three hulls; the second calls `too_close_to_launch`, which is tested, from a
  command no story reaches at that range.
- **A close that waits out its thirty ticks and is dropped**, and a close filed more than
  once. The story's close lands at its first filing.
- **The tonnes a drone carries against another hull's laser.** `rock_uncommitted_pool_tonnes`
  subtracts them and two drones of one hull are tested; two hulls on one rock are not.
- **The laser against the drones** (default 2) is not balanced, and nobody has played either.
- The `#[ignore]`d measurement lane (`think_cost.rs`) compiles and was not run; its drone row
  now measures a view of three ships where it measured an empty one.

## MW.13

A player's own mind sealed on a ship (doc 44 §7.1, §4 F; Q-44-26; first review finding 4,
second review rows 17, 18 and 25). It is also doc 23's cut S.4d, the berth anchor a ship's
claim needed (doc 23 §19, D-23-25; `23-cut-notes.md`). **The mind ABI did not move**: it is
`rhai=1.26.0;api=9;stock=3`. Paths are under `crates/tacsim/src/` unless given.

**Built.**

- **`+ship/mind`** (`port/ship_mind.rs`; the switch on `+ship`). `+ship/mind filed:<n>
  [org=<org>] go`, at the helm of a hull that lies at a port and carries a core, seals a
  filed mind its holder may fly on her. Bare, it reads what is aboard. `+ship/mind off go`
  unseals at her berth, before she sails. The refusals are the rig launch's own, from the
  same function (`surface::dispatch::dispatch`): a licence not held, one changing hands, a
  hull that is another's, a licence's seats all flying, and **an unfiled draft on a governed
  core** (finding 4). A hull with no core is refused by that, and a lapsed or revoked
  licence seals nothing.
- **The seal is the Roll's claim, by the rig's path.** Nothing is sealed in the verb. Its one
  submission files a claim cursor on the hull's `tacsim.ship_epoch` record
  (`ShipEpochRecord::claim`); the port tick's epoch arm opens the claim on the next fire as
  the operator, with the IC host, the `StartPermit` and the four owners a rig's launch uses,
  and the claim's own capture is the one seal (D-23-14: `Execution::capture`, through
  `seal_with_policy`). The launch arm is held off until the claim has opened.
- **Class, principal and claim across a restart** (finding 4). They are the claim book's,
  which the Roll and the four owners persist. `ship_mind::sealed` reads them from it. No
  bubble record changed and `compact_mind_seal` is untouched: a cast actor's seal is still the
  compact one, and a player's never goes through it.
- **The claim closes once, when she docks.** The bundle that closes the ship's epoch at a
  port moves the cursor to `Closing`, by the epoch's own kind, and the arm submits the Close
  as the operator. One record's one cursor names one open token, and the book closes a token
  once. A ship's cursor is never `Open`: the book says whether the claim is open.
- **The sealed mind thinks on the one host** (`space/thinks.rs` `SealedMind`, `Source`,
  `job_on`, `jobs_on`), with its seal's class row, its seal's card and **its seal's bound
  rule program** (row 17: the first space seal that runs one). The source is the text the
  claim retains. The backup seat's first source is that mind (`space/backup.rs` `sources`);
  it no longer reads the hull's `tacsim:mind_seal` attribute, which only a cast hull or a
  drone ever carried and which a salvaged hull kept.
- **The seat: helm and sensors, and no fire** (`space/applier.rs`). `Seat` has `lock` apart
  from `fire`. A sealed mind sits in the absent seat with one difference: it may lock. Every
  shot it asks is dropped, its burns are held to the rule the house default's are (only a
  burn that heaves her to), and it mines and trades nothing (the seat has no such act). A
  `LOCK NEAREST` row of its rule program acts in a bubble (`program_lock`); `Fire` and
  `HoldFire` rows bind inert, because a ship's seal carries no fire channel.
- **A seal is checked again where it is used** (`ship_mind::fits`). The seat is answered a
  seal only while the hull still carries a core of the class and governor the seal was
  taken over, and its dispatcher still has authority over the hull by the Roll's rule
  (`hull_authority`: a title, a lease, an office). A seal that no longer fits is not thought
  with, the holder is told why, and the arm closes its claim (`void`).
- **A claim given up, and a claim left behind.** The arm gives an Open five submissions, a
  fire apart. Then it gives the claim up (`refuse`), tells the holder, and once the cursor
  reads `Refused` withdraws the Open from the IC host (`IcHost::withdraw_surface_open`, new in
  `minds/ic_sortie/host.rs`). The host keeps one Open pending an actor and drops it only on the
  book's readback, so an Open that was never admitted would otherwise refuse that pilot's next
  seal on any hull until a restart. A withdrawn Open that turns up later finds no capability
  and is refused. One that lands in the gap between the give-up and the arm's next fire opens
  a claim no cursor names: the arm closes any such claim as its dispatcher (`owes`), `+ship/mind`
  reads it as closing, and a second seal is refused while it stands.
- **One guard where every Open is minted** (`minds/ic_sortie/surface.rs`, `surface_start`): a
  unit lying directly on a domain is given a permit only at a hangar port's berth. Sim's berth
  admission cannot tell a ship from a rig, so without the guard a rig stood on a site's ground
  would be admitted as berth-held, where Sim refused it before S.4d.
- **The epoch arm's caps hold** (`port/epoch.rs`). The mind arm is offered no hull that has an
  end of its epoch due, whether that end is ready or held back, so a visit for a mind never
  submits an end past the arm's retry caps.
- **A cored Dray** (`definitions/core_replacement.rs` `ShipCores`, `fitted_catalog`): six
  roots, `tacsim.unit.dray_core_{i,ii,iii}` and an ungoverned twin of each, built by the
  family builder the rig uses and registered with governed hardware. `surface_chassis` maps
  them to the Dray, so every port verb reads her as one. **No recipe fits one.**
- **The berth anchor** in Sim's exclusion: doc 23 §19 and `23-cut-notes.md`, S.4d.
- Help: `docs/help/ships.md`, "A mind sealed aboard". The bot's sentence table has a seventh
  family, `ship_mind` (24 rows, with its census).

**What the survey and the brief did not see.**

- **No ship carried a core**, and nothing in play fits one. The Dray is Class 0 by design
  ("her drive ROM flies a filed plan and decides nothing"). Hence the family above.
- **No ship could open a claim** (D-23-12), and the claim as S.4b built it forbade a held hull
  a journey at three places: the Open's admission, the boot audit and the plan check. The
  first rated run of the story read `sim: IC exclusion refused reason=not in a configured
  base`. Hence S.4d.
- **A core cannot be changed under a sealed mind at all.** The unit's owner fences a hull
  under a claim (`mush_tactics::ic_custody::refuse_mutation`): the Co-op's re-dress, a
  component's fitting or stripping, a core's fitting, a scrapping and staff's rebind are each
  refused, and the claim's Close asks for the Open's unit definition. So the row's "a core
  stripped after sealing leaves no seal of its class" is kept the stronger way: the strip is
  refused until the mind is unsealed. Ruled by the programme lead on 2026-11-13 (the done-when
  below is amended to say so).
- **The backup seat thinks outside a bubble too** (MW.8's `step_adrift`): finding 18's
  "nothing thinks in free flight" was true of the cast and is not true of the seat. A sealed
  mind is its first source there as well, with the seat's two acts (make for port, hold).
- **A sealed mind's rule program has one live action on a ship**: `Lock`. And a rule's
  native predicates other than a range have no fact in a bubble (odds, bands, aspects, scans),
  so such a row never acts there. A row with a think term acts only on a think whose script took
  the sample (`stock::rules()`), as on the ground.
- **A filed mind's class is its certificate's**, not the core's: `seal_with_policy` asks
  that the sealed class equal the certificate's minimum, so a mind rated above the hull's
  core is refused at the Open, as on the ground. The story's mind is rated Class I and sealed
  at Class I over a Class II core.

**Every route by which a hull's core or title changes, and what each does to a seal.**

| Route | Where | With a mind sealed aboard |
|---|---|---|
| The Co-op's re-dress, `+coop/refit prospector|standard` | `port/refit.rs` | **Refused**, by `MIND_HOLDS_HER` ("unseal it first"); the unit's owner would refuse the write. Story A |
| A bay's work on a hull: `+bay/order`, `/fit`, `/strip`, `/scrap`, `/return`, `/stow` | `port/bay_work/command.rs` | **Refused** by the same sentence, at the one place the verbs resolve their hull. Not reached by a story: a held ship cannot be berthed in a workshop either (Sim's freeze refuses the interior) |
| The certified core fitting and the ungoverning fitting | `hardware/`, `mush-economy` `core_refit` | **Refused** by the unit's owner (`component_replacement.rs`). No ship can stand in the shop in any case. Not tested here |
| Staff's rebind, `@unit/remove` then `@unit/create` | `mush-tactics` | **Refused** ("held by an IC sortie"). Story A |
| Damage that lowers a core's class in a fight | `arena::core::read_core_under` | The seal no longer fits (`Unfit::Core`): **voided**. Not tested: no space fight degrades a core today |
| A change of title the unit's owner does not see: `@asset/transfer`; a buyout's settlement (`port/buyout.rs`); a recovery house's sale (`port/house_sale.rs`) and sweep (`port/house_sweep.rs`); a salvage claim's award (`port/claims.rs`); a foreclosure on a lien or collateral | the economy | **Checked again**: the dispatcher's authority over the hull is asked at every think and every port fire. Lost, the seal is not thought with and its claim is closed (`void`). A buyout that titles the hull to the pilot who sealed it keeps it: her authority moves from the lease to the title. Story B does the transfer by staff; the sale, the sweep and the award are not flown |
| A charter or a lease that ends; a recall (`port/recall.rs`) | the economy, the port tick | As a change of title: the lease arm of `hull_authority` stops answering, and the claim is closed. Not flown |
| The hull docks at a port | `port/epoch.rs` | The claim **closes** with the epoch. The main story |

**With no claim open nothing is kept to seal again.** There is no stored selection: a seal
lasts one passage (D-UM-45), and a new `+ship/mind` is resolved against the hull's core and
its holder at that `go`. Story A pins it: after the unseal the yard re-dresses her as a
Dray-P, and the same licence is refused for want of a core. Story B: after the title has
passed, the pilot's `+ship/mind` is refused because the hull is another's.

**The acceptance, and what holds each.** The rated stories are in
`crates/tacsim/tests/domain_field/mw13_ship_mind.rs`, on the combined M8 world (the
production `LaunchComposition`); each takes one genuine Registry rating and runs on the CI
dispatch (`m8-gate-stories`).

| The row asks | Held by | Before the unit |
|---|---|---|
| On a governed hull a filed mind the player holds is sealed, and an unfiled draft is refused | The main story, step 1 (the claim opens at her berth; the seal reads her as principal, Class I over a governed Class II core, helm and sensors); the fast story `mw13_the_verb_refuses_a_draft_and_a_coreless_hull_and_the_house_default_locks_nothing` (the draft and the coreless hull) | No verb; no ship core; the Open refused |
| With her link down, inside a bubble, that mind makes a helm or sensor choice the house default does not | The main story, step 2: at the Kirkwood picket her mind locks the corvette that hails her and heaves her to, by its own trace words. The fast story flies the same passage with nothing sealed: the house default heaves to and locks nothing | The seat read only the house default, and dropped every lock |
| After a restart the seal reads the same class, principal and claim | The main story: a restart at her berth with the claim open, one inside the picket's bubble, one under way after the picket, and one between the dock and the Close; `aboard` is equal each time, and the mind flies on | No seal to read |
| The claim closes once when she docks | The main story, step 4: the epoch's close moves the cursor by `returned_to_base`; the arm's first Close is dropped and the server restarts; the record owes it again; the book holds it closed, and six fires and another restart later holds the same close token. The arm's first Open is dropped as well | No claim |
| A core stripped after sealing leaves no seal of its class (**amended**: refused while sealed) | Story A: the yard and staff's rebind are refused, her core and seal are as they were; unsealed, the re-dress lands and nothing seals again | The seal attribute stayed on the hull |
| A hull sold or swept carries no seal to its new owner | Story B: her title passes; the seat reads no mind at once, the pilot is told, the claim closes, no claim stands on the hull, and the pilot can seal nothing more on her | The seal attribute stayed on the hull |
| (the second reader) A claim given up leaves nothing behind | Story C: every Open lost, the claim is given up and the pilot told; the Open that turns up late is refused; she seals again; an Open that lands in the one gap left is closed by the arm, and she seals again after it | — |
| Its rule program bound; a mind whose rule program differs behaves differently | `space/tick_thinks_tests.rs` `mw13_two_sealed_minds_of_one_script_differ_by_their_rule_programs`: two hulls in one interdiction bubble, one script sealed twice through the one seal entry, one with a `LOCK NEAREST` row; that hull holds the interdictor locked and the other holds no lock | `KIT` bound an empty program for every space think |
| No fire channel: an intent on one is dropped | The same test (the script asks a shot of a real mount at every think, with a lock standing; nothing fires) and `space/applier.rs` `mw13_a_sealed_minds_seat_locks_and_never_shoots` | — |

**Defaults taken.**

| | Default | Cost to reverse |
|---|---|---|
| 1 | **The berth anchor** (doc 23 D-23-25): Sim's movement exclusion admits a ship, and the Open's record gains a field | **Expensive.** It is the movement-exclusion protocol |
| 2 | **A seal lasts one passage**, port to port (D-UM-45, as a rig's lasts one excursion). She seals again to fly it again. The alternative, a standing choice that seals at every departure, needs a selection kept on the hull and cleared at every route above | Medium: a second verb and a stored choice |
| 3 | **The seal is taken at her berth by its own verb**, not by a word on `+ship/undock`, so it holds for a queued or an ordered departure and for a lift off a pad alike | Low |
| 4 | **The claim opens before the epoch and its sortie id is not the epoch's.** The ship epoch's id is the journey's `departed().at` (S.2's V1), unknown at `go`; the claim's is the IC instant of `go`. The epoch's black box lines are unchanged and do not name the claim's seal | Medium if an inspection must read the claim's seal on the ship's own line: `ship_sealed` then takes the claim's facts |
| 5 | **A sealed mind in the absent seat may lock.** A lock is the sensors' act and the seal carries no fire channel. The house default still may not | Low: one field of `Seat` |
| 6 | **A sealed mind flies only while her crew has no link.** A crew at its stations flies her itself, as a seated pilot does on the ground | Medium: handing a present crew's helm to a mind is a new seat |
| 7 | **A draft on an ungoverned ship core is refused** as not built. A draft opens no claim, so nothing here would carry its seal | Low to add; it needs a seal kept outside the claim book |
| 8 | **A seal that no longer fits closes `powered_down_in_base`**, at her berth's host or the port she left | Low |
| 9 | **The Open is given five submissions**, a fire apart, before the claim is given up and the holder told | Low |
| 10 | **The licence's seats are counted at `go` over the book's active claims** and not over claims still opening on other hulls (a rig's `go` counts both) | Low; two `go`s in the same two fires could both pass for a one-seat licence, and the second Open is then refused by its own hull's exclusion or stands |
| 11 | **A mind rated above her core is refused at the Open**, after five tries, by the generic sentence; the verb does not read the certificate first | Low |
| 12 | The six cored Dray roots are in every launch's catalogue, and only the Dray has them (not the Dray-P) | Low |
| 13 | **A lock her mind set stays when her crew's link comes back.** The crew clears it or keeps it | Low |
| 14 | **A claim left behind closes `powered_down_in_base`** at her berth, or at the port she left if she has sailed since | Low |
| 15 | **The host keeps one Open pending an actor** (S.4b's rule, not this unit's). A pilot who seals on two hulls inside one fire has the second wait a fire, and each wait counts as one of its five tries | Low |

**Not built, and for whom.**

- **A route that fits a core to a ship.** The stories have staff bind the root
  (`@unit/remove`, `@unit/create`), as doc 17h's stories bound a rig's. **A gap for the
  owner**, beside MW.1's "no hulls for Houses": until something fits a core to a ship, no
  player can seal a mind on one.
- **Fire channels** (the ruling), and with them return fire in space (MW.9's follow-up).
- **What a client shows of a mind on a ship** (doc 44 §7.1, "not units").
- **The ship's black box naming the claim's seal** (default 4).
- **AP.9** (space reads the posture) and **MW.21** (the completion story) are theirs. MW.21
  can stand on `ship_mind::aboard` and the main story's helpers.
- **The known late-launch defect** (PR #571) was not met: the story's launch runs on the
  fire after the burn lands, inside the first leg.

**Found, and not fixed.**

- **A restart on the corridor before an interdicted waypoint raises no picket.** With the
  server restarted between the link loss and the Kirkwood picket, Sim logs `an interdicted
  corridor leg presents into NO domain; no activation was raised` and the bubble never opens
  on her. It is not this unit's: a probe with nothing sealed aboard fails the same way (run on conclav.dev, 2026-11-13; in the story rig, not investigated further). The story restarts under way after the picket instead.
  **Fixed since** (2026-11-13; doc 35's cut notes, "A crew that is away meets the picket"): the carrier hook
  opened the waypoint's domain only for a hull with a connected player aboard, and a restart leaves a pilot
  whose link was down not connected. It was production's, not the rig's. The restart before the picket is
  flown with the house default in the voyage group; this unit's main story still restarts after it, with a
  sealed mind.
- **A rig's refused Open stays pending in the IC host** (by reading, not run). The rig's arm
  (`surface/mind_claim.rs`, `refuse`) neither acknowledges nor withdraws the Open of a claim it
  gives up, and `IcHost::acknowledge` drops only a claim the book holds. The host keeps one
  Open pending an actor, so that operator's next mind launch on any rig would not be enqueued
  until a restart. The ship's arm had the same fault until story C found it;
  `withdraw_surface_open` is there for the rig's arm to call. **Run and fixed since**
  (2026-11-13): [23-cut-notes.md](23-cut-notes.md), "S.4b-2 follow-up".

- **The MW.5 field story fails about as often as it passes under load, on main**
  (`c4_a_filed_mind_flies_an_absent_pilots_escort_and_is_taken_home`). Its escort never leaves
  its cell: "the mind takes the escort out at its carrier's ramp: last at Some((146, 114))",
  handed and still in the field. Four runs at once on conclav.dev at main `e4e2f6e8e` failed
  three; four at this branch's head passed four. It failed once in this branch's dispatch run
  and once in another branch's (run 43461753678), and passed when run again. Not investigated
  in this cut. Run down and fixed afterwards: the story's own raid could wreck the escort
  (MW.5's "The raid is one round").

**Known limits.**

- **A claim whose dispatcher is gone, or whose hull has no unit, does not close.** Every
  transition is submitted as the dispatcher, and a Close asks for the hull's unit. There is no
  `destroyed` path for a ship (doc 23 keeps it reserved), and none was built here.
- **A claim left behind flies until the arm's next fire.** `sealed` reads the claim book, not
  the cursor, so for one port fire the seat would think with it. It arises at her berth.
- **Costs.** Each port fire and each offer of the epoch action walks every ship hull once. The
  think host keeps one compiled mind a hull and does not evict it.

**Reviewed.** A second reader (given the diff, not the conclusions) raised ten points.

| | Finding | Disposition |
|---|---|---|
| 1 | A give-up racing an Open still on its way leaves an open claim no cursor names, which never closes | **Fixed**: the withdrawal and the arm's close of a claim left behind, above; story C |
| 2 | A dispatcher who is gone, or a hull with no unit, leaves a claim that never closes | Not fixed: a known limit, above |
| 3 | The arm's tick keys repeated across fires, so an op refused once might never be asked again; a void under way closes `powered_down_in_base` | **Fixed**: the key carries the fire count. The kind is default 8 |
| 4 | Sim's berth admission admits any unit on a domain, a rig included | **Fixed**: the guard in `surface_start`; `s4d_a_rig_on_the_ground_is_given_no_open_permit_and_a_rig_in_its_bay_is_asked` |
| 5 | A hull with a held-back end of its epoch could be offered for its mind, and the visit then submitted the end past the arm's caps | **Fixed**: the mind arm skips every hull with an end due. No test pins it |
| 6 | `+ship/mind` named the wrong mind over a claim left behind | **Fixed**: it reads as closing |
| 7 | `holds` answered for a rig's claim, so a rig's refit was refused in a ship's words | **Fixed**: it answers only for a berth-anchored hold |
| 8 | A walk of every hull each fire; a compiled mind a hull kept for ever | Not fixed: the costs, above |
| 9 | An overflow in the lock's range sum | Not reachable: the sum is `i128` over `i64` differences |
| 10 | A lock the mind set outlasts it | Default 13 |

The same round found one more by running: the host's pending Open (story C's first half).

**Mutation checks.** Each mutation was applied alone or with one that touches another story,
the named test run, and the edit undone.

| | Mutation | Fails |
|---|---|---|
| U1 | Sim admits a berth where berths are not configured | mush-sim `s4d_a_port_berth_is_a_hangar_for_a_ship_where_berths_are_admitted` |
| U2 | A bay hold may root a journey as a berth hold may | The same |
| U3 | A berth hold may be placed on another domain from her berth | The same |
| U4 | An Open is minted for a unit on a domain that is no berth | `s4d_a_rig_on_the_ground_is_given_no_open_permit_and_a_rig_in_its_bay_is_asked` |
| U5 | A sealed mind thinks with the empty rule program | `mw13_two_sealed_minds_of_one_script_differ_by_their_rule_programs` (its think-gated row) |
| U6 | The sealed mind's seat may fire | The same (a shot is fired) |
| U7 | The sealed mind's seat may not lock | The same |
| U8 | The applier lets any seat lock | `the_absent_seat_drops_every_lock_and_every_shot_and_burns_only_to_heave_to`, `mw13_a_sealed_minds_seat_locks_and_never_shoots` |
| U9 | A sealed mind thinks at Class III whatever its seal | `mw13_two_sealed_minds_of_one_script_differ_by_their_rule_programs` |
| S1 | The verb does not name the governed-draft refusal | The fast story. The draft is still refused, in the other sentence: the verb has no path that submits a draft |
| S2 | The epoch's close does not move the cursor | The main story: the claim closes `powered_down_in_base`, not `returned_to_base` |
| S3 | `holds` answers no | Story A: the yard's own failure line, not "unseal it first" |
| S4 | `fits` does not ask the holder's authority | Story B: the seal is still read after her title has passed |
| S5 | A claim given up is not withdrawn from the host | Story C: the late Open lands |
| S6 | The arm does not close a claim no cursor names | Story C: the claim does not close |
| S7 | The seat does not take the sealed mind as its first source | The main story: `BACKUP_COAST` and no lock |
| S9 | The production composition admits no berth | The main story: the Open does not land |

U4 and U5 passed on their first run. The guard's test did not reach the call it guards, and
no test had a rule row with a think term; both tests were strengthened and both mutations
fail now.

**Persisted records.** `ShipEpochRecord` gains `claim`, optional and absent on the wire when
none: a record written before reads as it did (`mw13_the_epochs_close_leaves_a_cursor_it_cannot_read_for_the_arm`).
Sim's `OpenImage` gains `berth` (S.4d). Nothing else. No definition, descriptor or manifest
moved; no pin was re-pinned.

## MW.14a

The compiled NPC commander (doc 44 §7.1, §4 D, §4c rows 14 and 15; Q-44-20, Q-44-44; doc 27
FO-15, Q-D4, Q-F1). Built on Q-44-44's default: a host-side issuer, compiled and deterministic,
with no LLM and no change to the mind ABI.

**Built.**

- **The rule, in one file** (`crates/tacsim/src/detachment/commander.rs`). `View` is all a
  commander decides on: what its own frame holds (`Sight`: the live contacts its party's book
  holds Hostile), the state of its roster (`Strength`: hulls, and those lost in this field) and
  the detachment's work point. `decide` is the whole rule, first match wins: **withdraw** once
  the roster has lost a hull (`WITHDRAW_AT_LOSSES`, 1), **hold** while a Hostile is in sight,
  **advance** to the work point otherwise. `Posture` is what it may ask for. The types are plain
  data in and plain data out, with no handle on the world, the clock or the dice, which is the
  shape MW.14b's doctrine and a House's policy doctrine (Q-44-32) take.
- **The issuer** (`crates/tacsim/src/detachment/sortie/commander.rs`, called from the NPC walk
  in `sortie/npc.rs`). When a roster's field command names an NPC its leader and that NPC's unit
  stands in a running field, `step` builds the view, takes the posture and gives it as a roster
  order to the first hull that does not yet have it. It goes through a player leader's delivery
  and nothing beside it: submitted as the NPC, fenced by the roster's revision and term
  (`field_orders::Act::Issue`); Org's mission rights read for the NPC over each unit; a
  `mush_propagation` envelope to the recipient, which interference holds; the recipient's own
  executor and the kernel's own checks (`field/orders.rs`, `arena/detachment.rs`, both
  unchanged). The order book is the commander's only memory, so a restart derives the same next
  order. The one thing kept in RAM is scheduling: an order just submitted to a slot is not
  submitted again for `RESEND_AFTER_S` (60 wall seconds), and an attempt that submitted nothing
  is not repeated for `RETRY_AFTER_S` (5), so one refused at its final take or answered NEGATIVE
  is retried at that pace and the commander's own hull flies its order in the fires between.
  Wall time, because at the live rate of 24:1 an IC minute is under three fires.
- **What the commander sees** (`crates/tacsim/src/field/orders.rs`): `sight` reads the Hostile
  contacts off the last frame the NPC's own seat was sent; `lost` says a unit is lost in its
  field when its operator's own frame shows a wreck or the field's kernel has named it
  destroyed.
- **A lost hull stops leading at once** (`sortie/field.rs`, `reconcile`): the roster's command
  facts read `lost` beside the world's unit record, which is checked out and stale while a field
  runs. Before this a commander destroyed in a field stayed the roster's leader until the
  field's close, and the kernel refused every order it gave (`NoCommander`). The fact is the
  roster's, so it holds for a player leader too: a player whose hull is destroyed in a field
  stops leading, and the next eligible player or the NPC commander leads.
- **The successor** (`crates/mush-tactics/src/detachment/roster.rs`, `reconcile`, and
  `roster/checkpoint.rs`, `restore`): while the nominated commander's own unit is lost or
  disabled, the first other NPC in slot order that is healthy and authorized leads under a new
  term, and the commander leads again once its unit is not. A commander who is only absent
  (still aboard in ramp transit, out of its seat) or only lacks the command right is not passed
  over; a roster that nominated no commander never elects an NPC; an active player outranks
  both.
- **Yielding to a returning player** needed no code: the roster's own rule (doc 27 Q-D4, the
  return hysteresis) makes the player the leader, `step` finds the NPC is not, and what the NPC
  had in flight under its term is discarded by the poll as any deposed leader's is.
- `docs/help/detachments.md` says what an NPC leader does. Doc 27's Q-D4 carries the successor
  as an amendment.
- **`roster_rig`** grew what the stories needed: `fly` takes a roster of two hired NPCs
  (below); `grant_command` issues the mission-role grants `+sortie/command-grants` prints, each
  under the organiser's own command loan; `orders_of` reads the roster's order book.

**Tests.** In-source: the rule's three arms and their order, and the same view giving the same
posture (`detachment::commander`); each posture as the roster order a player types
(`detachment::sortie::commander`); the successor, its restore, and a player outranking it
(`mush_tactics::detachment::roster`, two tests). Stories in `domain_field`
(`crates/tacsim/tests/domain_field/field_mw14a.rs`), each passing alone on Windows:

- `mw14a_an_npc_led_detachment_advances_holds_and_withdraws` (108 s) and `…_at_512` (89 s). A
  captain hires Sana Okonkwo and Ilya Mendes, names Okonkwo the commander and launches both
  from aboard the carrier; no player is in the roster's units. An escort and an intruder stand
  on the ground and the escort's fire order promotes the site. With nobody typing an order:
  Okonkwo orders both hulls to the work point and both drive for it; staff jam the roster's
  comms and the intruder fires on the roster's carrier, so her HOLD is admitted and sent and
  does not arrive, and both hulls keep their ADVANCE; the jam is cleared and both hulls stand;
  the intruder destroys Okonkwo's rig, and Mendes leads under a new term, orders the withdrawal
  from his own unit and drives for the ramp.
- `mw14a_a_returning_player_takes_the_lead` (26 s). A captain leads from rig 1 with Okonkwo,
  the plan's commander, in rig 2. While the captain leads she gives no order. The captain's
  session ends: she leads and orders the advance, to her own hull only. The captain returns:
  after the hysteresis the lead is theirs, their `ORDER DT:… 2 HOLD` is delivered and obeyed,
  and she gives no order under their term.
- `mw14a_the_same_run_gives_the_same_orders_twice` (55 s). The script to the HOLD on two fresh
  worlds delivers the same orders, with the same sequences, from the same slot.

**Divergences and defaults.** None needs a refactor to undo; each is one function or one filter.

- **The work point is a default.** A plan names no objective, so the commander advances to the
  site cell `WORK_POINT_CELLS` (6) out from the flight's ramp, straight away from the pad. A real
  objective is a field on the plan, which is a plan-format change and was not made.
- **An NPC-led roster holds only when it is fired on.** Only a roster's current leader may mark
  a party Hostile (`field/parties.rs`, `current_leader`), and an NPC marks nobody, so its book
  holds a party Hostile once that party's fire arrives on the roster or on a friend. The story's
  intruder fires on the roster's carrier for that.
- **"The next NPC in the roster's order"** is the first other NPC by slot number, not the next
  after the commander's slot. Its contract scope is not checked: Mendes, hired as a subordinate
  operator, succeeds Okonkwo. That amends doc 27 Q-F1's "explicitly assigned" for the one case
  of a commander who cannot command, as Q-44-44 ruled.
- **Succession is on the commander's unit, lost or disabled**, and nothing else. A commander
  who is absent for another reason is not succeeded and the roster has no NPC leader, as before;
  otherwise the successor would lead for the seconds the commander spends on the ramp at every
  launch. The lead comes back when the commander's unit is whole again.
- **One order a slot, never `ALL`.** An `ALL` is refused whole when any slot is unavailable, and
  a principal acts once a fire, so a posture reaches `n` hulls over `n` world seconds.
- **Only inside a running field.** On open ground an NPC leader gives nothing, and the surface
  executor goes on running whatever was delivered.
- **The link loss in the returning-player story is the session's facts set by hand**
  (`set_connected`, `set_live_conns`), as `mush-server`'s leadership story sets them. What the
  field does with an absent pilot's hull is MW.7's and is not exercised.
- **The jam is staff's** (`@prop/jam <place>=4` at the crew's place). No player-held jammer
  reaches the IC transport.
- **The stories run at 1:1**, the field rigs' rate, as every field story does. The one IC window
  in them is the roster's three-IC-second return hysteresis, waited out in full; at 24:1 it is
  an eighth of a second.
- **A Dray-P berths two, and the acceptance needs three people aboard.** The organiser and two
  hired NPCs fit only because a person in a rig's cab holds no berth: the organiser boards,
  takes rig 1's cab, the crew board, the plan is drafted and activated, slot 2's NPC is seated,
  the organiser steps out (`+tv/1 leave`) and slot 1's NPC is seated, all at Ceres before the
  hull sails. Every step is a player's command, and `roster_rig::fly` pins the sequence. **For
  the owner:** no help page says so, and if a cab should count against the berths then no
  shipped hull can fly a two-NPC detachment with its organiser aboard.
- **A lessee's mission-role grant needs its parent.** `+sortie/command-grants` prints a draft
  with `"parent": null`, which Org refuses from a lessee; the rig adds the lessee's own command
  loan. The help page already says a lender supplies its parent, and the printed line does not.

**Not built.**

- MW.14b, the Rhai commander.
- **The organiser cannot read what their NPC commander ordered.** `+sortie/orders` shows the
  reader's own outbox and an NPC's orders are in nobody's but its own, so the stories read the
  roster's order book as typed state.
- Any posture but the three: the commander gives no ENGAGE, SCREEN, SUPPORT or MINE, holds no
  fire, and marks no party.
- Damping. A Hostile that drifts in and out of the commander's sight flips the posture between
  HOLD and ADVANCE as often as once a world second.
- A commander's orders on open ground, and a plan objective.

**Not pinned by a story.** A restart with an NPC leading. The commander taking the lead back
from its successor (the roster's unit test has it). The ADVANCE returning once the Hostile is out
of sight. The withdrawing rig boarding its carrier (the story ends with it driving for the ramp).
A successor refused for want of the command right (unit test only). A posture answered
NEGATIVE and sent again a minute later, and an order refused at its final take and retried: the
resend guard has no test of its own. Whether a destroyed hull keeps its field mark until the
field closes was not checked beyond the acceptance story, where the successor's WITHDRAW stood
for the half minute the story watched; if the mark were cleared earlier the roster would read no
loss and the posture would go back to HOLD or ADVANCE.

**Reviewed.** A read-only review of the engine diff by a second agent found six points, four
fixed in the PR's last commit: the resend guard was in IC seconds, which is no pacing at 24:1; a
posture answered NEGATIVE counted as standing and was never sent again; the work point's clamp
panicked on a site that reads no side; and an attempt that never reached the queue was recorded
as sent. The fifth is the missing damping above. The sixth was two doc comments.

## MW.14b

Not built. **Deferred by the owner on 2026-11-13 (Q-44-49)**: the compiled commander is kept, and
MW.14c below extends it.

## MW.14c

The compiled commander's gaps, and a focus-fire order (doc 44 §7.1, §5.13; Q-44-49, the owner's
ruling of 2026-11-13: "lets defer the rhai, and fix the gaps with the hard-coded solution, and add
a focus fire order"). No Rhai, no LLM, and nothing in `crates/mush-mind` moved: `MIND_ABI` is
still `rhai=1.26.0;api=9;stock=3`. The kernel did not move either: `crates/tacsim/src/arena/` is
untouched, and a `CONCENTRATE` reaches a hull as the `MindObjective::Engage { target }` an
`ENGAGE` naming a contact already was.

**Built.**

- **The dwell** (`crates/tacsim/src/detachment/commander.rs`, `DWELL_S`, 30). A posture the
  commander ordered less than thirty wall seconds ago is kept, whatever its sight says now. A
  loss is exempt: the WITHDRAW is never delayed, and a roster ordered home is never turned
  round. The rule's input gained what the roster's history records its NPC leader last ordered
  and for how long (`Held`, `Named`). The host reads it from the orders the history keeps of
  its NPC leaders (`sortie/commander.rs`, `remembered`; `FieldOrders::given`, below), each
  stamped when it was admitted: a posture's spell began at the earliest order saying the same
  since the last order that said another. That record is written in the transaction that
  admits an order and nowhere else, and nothing new is kept in RAM, so a restart reads the
  same and decides the same. A successor reads the posture of the commander it succeeds, so
  it inherits the dwell. (The order book alone cannot say when a spell began: it holds one
  order a slot, and an order of an earlier spell left on a hull the later orders never
  reached made the posture look old. The second reader found it.)
  **Wall time, read off IC stamps.** The history stamps an order in IC milliseconds; the host
  divides the IC time since by the world's rate now. At 24:1 an IC-second dwell would be a
  twenty-fourth as long, and a principal acts once a world second, so that would be no damping
  (MW.14a's review found the same of the resend pacing). A change of rate inside a dwell
  stretches or shortens that one dwell.
- **The organiser reads their NPC commander's orders** (`sortie/field_orders.rs`, `Given`,
  `GIVEN_KEPT`; `sortie/field_commands.rs`, `npc_orders`). The roster's history keeps the last
  sixteen orders an NPC leader gave: the slot, the sequence, who gave it, what it said and
  when. The
  transport keeps a message's reference and not its words, and the book keeps one order a slot,
  so before this a superseded order of an NPC's was nowhere a player could read.
  `+sortie/orders` prints them after the reader's own, oldest first, one line an order inside
  120 columns (`NPC_ORDER`: `Sana Okonkwo to slot 2, order 7: CONCENTRATE BK; WILCO received.`),
  each with its answer once the reply has reached the leader who gave the order, the rule a
  player's own outbox follows.
  **Who may read them: the organiser and every player flying a slot on that roster**, which is
  who could read `+sortie/orders` for the operation at all (`sortie/commands.rs`, `readable`).
  The NPC is hired under the organiser's contract and leads their units. Anyone else is refused
  the whole report, as before.
- **A plan names its work point** (`crates/mush-tactics/src/detachment/planning.rs`,
  `Plan::work_point`; `crates/tacsim/src/detachment/planning/commands.rs`, `+sortie/workpoint`).
  `+sortie/workpoint <draft>:<revision>=<x>,<y>` names the field cell, `=none` clears it, and
  `"work_point": [x, y]` in the plan's JSON does the same; `+sortie/plan` shows it in words. It
  is an edit of the draft: the revision moves and declarations are made again. A cell is valid
  below 512 on each axis (the widest site; a plan is made before its site is known). The
  commander advances there when the cell is on the site the roster flew to, and to MW.14a's
  default (six cells out from the ramp) when the plan names none or the cell is off the site.
- **`CONCENTRATE`, the focus-fire order** (`FieldObjective::Concentrate`;
  `crates/tacsim/src/field/orders.rs`, `concentrate`). `ORDER DT:… ALL CONCENTRATE BK`, or a
  numbered slot. It is issued through `field_orders::Act::Issue` like every roster order: the
  leader's authority and term, each unit's command right, the order book, the IC transport and
  its jamming, the party's carrier relay. A player leader types it and the commander gives it
  through the same act.
- **The commander concentrates** (`commander.rs`, `Decision::concentrate`). While it holds it
  names one contact for the roster and keeps to it until the contact is destroyed or out of
  its sight, then names the next; a contact merely out of sight keeps its name for the dwell,
  and a destroyed one is replaced at once. It names only a contact its own frame holds Hostile
  (`field/orders.rs`, `hostile_contact`). It still marks nobody and sets no player's POSTURE,
  so an NPC-led roster concentrates only on a party that fired on it or on a friend. **It
  concentrates the hulls that carry something to fire** and leaves a mining rig on its HOLD
  (`sortie/commander.rs`, `can_fire`, the unit's own record): an engagement would send a hull
  with nothing to fire at the Hostile it was told to hold for, and a roster of prospectors is
  led exactly as MW.14a led it. The posture goes out first, one slot a fire as before; a
  `CONCENTRATE` that every hull it can reach still owes goes to all of them in one order, and
  one that only some owe goes slot by slot.

**What an `ENGAGE` naming a contact does today, and what `CONCENTRATE` adds.** Established in
the tree before building (`arena/detachment.rs`, `apply_detachment_command`; `arena/intents.rs`,
`rekey_objective`; `field/orders.rs`, `step`): `<slot> ENGAGE BK` carries the contact's row key
in the leader's own scope. The host offers it to the kernel unconditionally. The kernel drops
the hull's standing order, then re-keys the contact into the recipient's own classified rows.
If the recipient holds it, the hull flies `Engage { target: Some(row) }` and answers WILCO. If
it does not, the hull is left on `Engage { target: None }`, an engagement of any Hostile on
doctrine, and answers NEGATIVE: its earlier order is gone. Either way the engagement stands
until another order replaces it; nothing ends it when the contact dies. `ALL ENGAGE BK` is
refused whole when any slot is unavailable. `CONCENTRATE` adds four things:

1. **It reaches the hulls that can take it.** `ALL CONCENTRATE` is admitted for every hull the
   leader holds a command right over that is available and on its field flight on the site,
   and refused only when there is none. That is a new order target in the book
   (`mush_tactics::detachment::OrderTarget::Able`), which the issue path admits for a
   `CONCENTRATE` and nothing else; `ALL` is as strict as it was for every other order, and for
   an `ENGAGE`.
2. **A hull that does not hold the contact keeps the order it had.** Before the kernel is
   offered anything, the host looks for the named contact on the recipient's own frame: a row
   it has identified, under the designator the leader read, not read destroyed. An identified
   contact's designator is its own callsign, the same on every scope that has identified it,
   so this is the hull finding the contact in its own observation. Not there: NEGATIVE, the
   kernel is offered nothing, and the earlier order goes on standing untouched.
3. **A hull that holds it engages that contact** through the existing kernel path, and
   answers WILCO once its own frame shows the engagement standing on a held row.
4. **When the contact is destroyed or lost the hull goes back to its earlier order, by
   itself.** The poll that delivers a `CONCENTRATE` records what stood on the slot
   (`Concentration::before`) and reserves a sequence for the return above the order's own. Each
   visit reads the recipient's own frame; once the row the engagement stands on reads destroyed
   or is held no longer, the host offers the kernel the earlier order again under the reserved
   sequence. It waits on no relay and is answered to nobody, like a lapsed order's stand (doc 45
   unit AP.8). Through a second `CONCENTRATE` the hull goes back to what the first replaced. An
   earlier order that had ended (completed, expired, refused) is nothing to go back to, and the
   hull is left with no order; one that had lapsed with its leader is gone back to as the HOLD
   its stand was. "The order it had" is the order the book had delivered to the hull: if that
   order had not yet reached the kernel when the `CONCENTRATE` arrived (a damaged relay's
   delay, a visit not yet made) and the hull declines the `CONCENTRATE`, the host offers it
   under the return's sequence, through the relay it had still to pass.

It hands nobody a contact and grants no permission: the doctrine still fires only on parties
its own party holds Hostile. A leader may name only a contact they have identified; a
provisional number is refused by name (`CONCENTRATE_UNIDENTIFIED`).

**The absent-player rules.** A `CONCENTRATE` presses (`field/orders.rs`, `presses`), so one a
player gave lapses when that player is gone exactly as an `ENGAGE` does (AP.8), and the hull
takes the same stand. **The order under a `CONCENTRATE` lapses too** (`field_orders.rs`,
`under_lapsing`, `Concentration::stand_down`): an `ENGAGE` or an `ADVANCE` a player gave, with a
`CONCENTRATE` delivered over it, is an order the lapse scan could not see, and a hull that
declined the `CONCENTRATE` or came back from it would have gone on pressing for a leader who
had gone. In the write that finds that leader gone the record's earlier order becomes a HOLD
(under the lapsed order's own HOLD FIRE) with a sequence of its own; a hull on the contact
stays on it, and any other hull is offered the HOLD at once. It is a plain HOLD: it does not
take the second half of AP.8's stand (answering a hit). A player's unit is never flown by a roster order, a `CONCENTRATE`
included: the NPC walk executes orders for hired crew only, an absent player's slot is not
available and the tolerant target leaves it out, and `arena::absent` was not touched.

**The order word: `CONCENTRATE`. Flagged for the owner.** The owner asked for "a focus fire
order". `FOCUS` was not used because `+focus` is the command that takes a console, and
`ORDER … ALL FOCUS BK` beside `+focus #12/pilot` reads as the same thing. It is a word of its
own and not a form of `ENGAGE` because what it does to a hull that cannot find the contact, and
what it does when the contact dies, are different, and `ALL ENGAGE BK` already means the strict
thing. Renaming the word is one match arm in `field_commands.rs`, one in `directive_text` and
the help text; the stored variant is `concentrate`, which a rename would leave alone.

*Renamed to `FOCUS` by R-47-1 (doc 47 unit CO.1); `CONCENTRATE` is still accepted as an alias and the stored variant is unchanged.*

**The plan's stored format. The one piece here that is expensive to reverse.** `Plan` gained
`work_point: Option<(u16, u16)>`, written as `"work_point": [x, y]`. The key is optional on read
and is not written when no point is named, so every plan stored before this unit loads, reads as
naming none, re-encodes byte for byte, and keeps its material hash; a stored draft's `plan_hash`
and every consent on it still match. A plan that names a point hashes differently, as any
material edit does. The other two option keys of a plan are required on read; this one is not,
which is what keeps older records loading. A reader older than this unit refuses a plan that
names a point (`deny_unknown_fields`); there is no such reader to keep, the project being
pre-launch. Two other stored records grew the same way, optional on read and absent when empty:
the roster's message history gained `concentrating` and `given`, and `FieldObjective` gained
the `concentrate` variant.

**Defaults taken**, each provisional, with what it costs to change:

| Default | Value | To change |
|---|---|---|
| The dwell | `DWELL_S` 30 wall seconds, for a posture and for a contact out of sight | one constant |
| What the dwell exempts | a withdrawal after a loss; a new name once the named contact is destroyed | one arm each in `decide` and `target` |
| Which contact | the nearest Hostile in the commander's own sight not read destroyed, the lower row key between two at one range | `target`, one function |
| Orders kept for reading | `GIVEN_KEPT` 16, NPC leaders' only | one constant; a player leader's orders are not kept |
| Who reads them | the organiser and the roster's players | `readable`, one function |
| A work point's bound | below 512 on each axis | one constant, held equal to the widest site by a test |
| A leader's contact | must be identified | one check in `concentrate_on` |
| The tolerant target | `CONCENTRATE` only | one check in the issue path |
| Who is concentrated | by the commander, hulls that carry something to fire; by a player leader, whoever they order | `can_fire`, one function |
| An expiry | a `CONCENTRATE` takes no `EXPIRES` | one check at the parse and one at the issue |
| The order word | `CONCENTRATE` | two match arms and the help |
| The plan's format | an optional `work_point` key | **expensive**: stored plans, presets and accepted rosters carry it once written |

"The Hostile that last fired on the roster" was the brief's example of a target rule. The
commander's own frame does not say which contact fired, and the rule decides on that frame
alone, so it could not be written without handing the commander the kernel's record. Nearest
was taken instead.

**Divergences.**

- **A hull that lost the contact is not ordered onto it again** until the commander names
  another, or the hull had answered NEGATIVE (which is re-sent at the resend guard's pace, as a
  refused posture is). The book cannot tell the commander that a hull has gone back to its
  earlier order, and the commander does not read another hull's frame.
- **A commander in a hull that carries nothing to fire reads no contact destroyed.** It goes on
  holding for a wreck until its own frame drops the plot (the sensor does not see wrecks, so
  that is a few seconds; story 1 waits it out).
- **A `CONCENTRATE` takes no `EXPIRES`** (`CONCENTRATE_NO_EXPIRY`). An expiry ends the
  engagement in the kernel with nothing to take the hull back to its order.
- **A successor names the commander's contact again**, once: a contact's row key is its
  leader's own scope's, so the successor cannot read which contact the roster is on.
- **An earlier `ENGAGE` naming a contact is gone back to from its own leader's unit**, in
  whose scope the contact was named; if that unit has been lost the kernel refuses the return
  and the hull stays on the engagement until it is ordered otherwise.
- **Only inside a running field**, as MW.14a: on open ground an NPC leader gives nothing, and a
  `CONCENTRATE` there is refused at issue (it names a contact of a field's cockpit).
- **`ALL CONCENTRATE` includes the leader's own slot** when the leader holds the command right
  over their own unit, as `ALL` does. To a player it is the advice every order is.

**Tests.** In-source: the rule's dwell, its exemption and the named contact
(`detachment::commander`, two tests beside MW.14a's two); the history's posture and when its
spell began, a posture as a roster order, a named work point used only on its site
(`detachment::sortie::commander`); a history written before the unit loading, what a
delivered `CONCENTRATE` records, the lapsed order under one, the stand-down, the kept orders
(`detachment::sortie::field_orders`, five tests); an NPC leader's line inside 120 columns
(`detachment::sortie::field_commands`); a work point's cell (`detachment::planning::commands`);
`CONCENTRATE` pressing (`field::orders`, AP.8's test extended); the tolerant target and the
plan's format, old and new (`mush_tactics::detachment`, three tests); the sentences as the game
prints them (`mush-bot`, `mine::lines`). Stories in `domain_field`
(`crates/tacsim/tests/domain_field/field_mw14c.rs`), every field with three parties in it;
times are conclav.dev's, each run in a batch of four:

- `mw14c_a_posture_stands_for_the_dwell_and_a_loss_withdraws_at_once` (93 s). Two hired crew
  under Okonkwo's command advance and reach the work point. The escort fires on the roster's
  carrier: a Hostile is in her sight and, the advance being older than the dwell, she orders
  HOLD at once. The intruder's player destroys the escort and her frame drops the contact:
  second by second to the dwell's end she orders nothing, and then she orders the ADVANCE. The
  intruder's player destroys the roster's other hull a few seconds into that dwell: the
  WITHDRAW is ordered inside it. The organiser then reads her advance, her hold and her
  withdrawal in `+sortie/orders`, each line inside 120 columns, with WILCO where the answer
  reached her; the third party's player is refused the report.
- `mw14c_a_plans_work_point_is_advanced_to` (34 s) and `…_at_512` (17 s). The organiser names
  the work point with `+sortie/workpoint`, the plan shows it, the draft's revision moves, and
  both hulls' ADVANCE is to that cell: the miner's unread pocket on Ahuna Flank, where a rig
  drives for it, and `(262, 268)` on Urvara Reach, a cell no 256 site holds. **The plan written
  before the unit** is MW.14a's acceptance, unchanged: `roster_rig::fly` drafts its plan with no
  `work_point` key, and those two stories still advance to the default six cells off the ramp.
- `mw14c_a_leaders_concentrate_puts_a_hull_that_holds_the_contact_on_it` (33 s). A captain
  leads with Rook Adeyemi in an armed rig on HOLD, marks the intruder's party Hostile and
  orders `ALL CONCENTRATE` on it with the crew's comms jammed: the order is in flight and does
  not arrive, and the hull holds and fires nothing. The jam lifts: the hull stands on an
  engagement of that contact and fires on it. The intruder is destroyed: the hull's own frame
  shows its HOLD again, the delivered order is still the `CONCENTRATE` (nothing was sent to
  bring it back), it fired on nothing else, and on HOLD it fires nothing.
- `mw14c_a_hull_that_does_not_hold_the_contact_keeps_its_order` (27 s). The same roster with
  the hired rig 67 cells from the intruder and 54 from its leader, past its own sight and past
  the range a party shares contacts over. It answers NEGATIVE; its order row is the same HOLD
  row byte for byte; the field's ledger holds no engagement commanded for it; it fires nothing.
- `mw14c_the_order_under_a_concentrate_lapses_with_its_leader` (22 s). The same far hull on
  its leader's ADVANCE declines a `CONCENTRATE` and drives on. The leader quits: the history
  turns the order the hull goes back to into a HOLD under a sequence of its own, and the hull
  holds. The kernel was offered no engagement for it and it fired nothing.
- `mw14c_an_npc_led_roster_concentrates_on_the_party_that_fired` (40 s). Two hired crew in
  armed rigs, no player in a unit. The intruder fires on the roster's carrier; the escort,
  nearer, never fires on the roster. The commander holds and names the intruder for both hulls
  in one order (one sequence, the tolerant target); both fire on it, every round either fires
  is at it, the nearest contact in her sight is not Hostile and is not the one she named, and
  the organiser reads the order with both WILCOs.
- `mw14c_an_absent_players_unit_is_sent_no_concentrate_and_does_not_fire_first` (30 s). A
  captain leaves their armed rig on HOLD and quits; the plan's commander leads, is fired on,
  and concentrates her own hull on the intruder. Every order she gave went to her own slot, and
  the captain's rig, its mind's, in the field, with the Hostile in sight, fires nothing.
- `mw14c_a_restart_with_the_npc_leading_gives_the_same_next_order` (22 s). The world is
  stepped a second at a time to the second her first order is admitted (the ADVANCE to her own
  hull), and the server restarts there. On the next boot the next order in the book is the
  same ADVANCE to the other hull, next in sequence; her own is the order given before the
  restart, with the same stamp.
- `mw14c_the_same_run_gives_the_same_orders_twice` (46 s). The script to the commander's
  `CONCENTRATE` on two fresh worlds delivers the same orders, with the same sequences, from
  the same slot.

MW.14a's four stories, MW.15's two, AP.4a's two and AP.8's five pass unchanged beside them.

**What the stories stand in for**, beyond MW.14a's and AP.8's list: a hired NPC sits in an
armed rig by a refit (no shipped command seats one, AP.8's gap); a hull is made fragile by a
staff hand so a kill takes seconds; and where a dwell is measured the story steps the world's
clock itself, the arena running in real time beside it.

**Mutations.** Each was applied over the unit in a second clone on conclav.dev and the test
named for it run; every one failed that test.

| Mutation | Test it fails |
|---|---|
| no dwell (a posture may change at once) | story 1: "5 s after the HOLD, inside the dwell, the posture is not changed" |
| `+sortie/orders` shows a stranger the report | story 1: "a stranger is refused the roster's orders" |
| the plan's work point ignored | story 2, at 256 and at 512 |
| a `CONCENTRATE` offered to a hull that does not hold the contact | story 4: "its HOLD goes on standing, untouched" |
| the earlier order not gone back to | story 3: "once the contact is destroyed the hull stands on its earlier order" |
| the commander reads and names contacts its book does not hold Hostile | story 5: the roster never advances, holding for a contact that is not Hostile |
| the commander orders an absent player's unit, and the issue admits it | story 6: "every order the commander gave went to her own slot" |
| an order in flight remembered only in RAM | story 7: "the same next order as the book owed before the restart" |
| the order under a `CONCENTRATE` does not lapse | "the order under the CONCENTRATE lapses with its leader and the hull holds" |
| unarmed hulls concentrated | story 1: a `CONCENTRATE` reaches two mining rigs inside the dwell |
| a `CONCENTRATE` does not press | `field::orders`, AP.8's pressing test |
| the plan's new key required on read | `mush_tactics`, the older-plan test |
| `ALL` made tolerant for every order | `mush_tactics`, the atomic-`ALL` test and the tolerant-target test |

**An absent player's unit firing first** could not be produced by any one edit in this unit:
the NPC walk never executes a roster order for a player's unit, the kernel takes no objective
for a unit flown for an absent player, and the mind is shown as hostile only what has fired on
it. Story 6 pins the outcome and that no order reaches the unit; the mutation shown for it is
the nearest one this unit owns.

**Not built.** MW.14b. A `CONCENTRATE` on open ground. Sub-leaders (Q-44-21). A commander's
SCREEN, SUPPORT or MINE. A player-held jammer.

**Not pinned by a story.** A Hostile that leaves the commander's sight by driving out of it
(the story's Hostile is destroyed, and the commander's frame drops the wreck). The name kept
for the dwell while a live contact is out of sight (the rule's unit test has it). A declined
`CONCENTRATE` over an order that had not reached the kernel (the second reader's first point:
the executor's path has no test of its own). The order under a `CONCENTRATE` lapsing while
the hull is on the contact, or after it came back from it (the story flies the declined
case). The tolerant target refused for an order that is
not a `CONCENTRATE` (no typed line can ask for it; the check is the issue path's own). A
successor inheriting the dwell (unit test only). A `CONCENTRATE` chained over another in a
field. A work point named on a preset.

**Reviewed.** A second reader was given the engine diff and the requirements, not the
conclusions, and reported five points, all checked in the tree and all fixed before the PR:

1. a declined `CONCENTRATE` dropped an earlier order that the book had delivered and the
   kernel had not yet been offered (a relay's delay): the hull kept advancing while the book
   said HOLD. The executor now offers that order under the return's sequence;
2. the lapse scan could not see the order under a `CONCENTRATE`, so a hull that declined one,
   or came back from one, went on pressing for a leader who had gone. `under_lapsing` and the
   stand-down are the fix, and a story flies the declined case;
3. the dwell was read off the book's standing orders, and an order of an earlier spell left on
   an unreachable hull made the posture look old. It is now read from the kept orders, which
   are stamped and in order;
4. a successor started with no dwell. It now reads the posture of the commander it succeeds;
5. a `CONCENTRATE` given an expiry ended in the kernel with nothing to take the hull back.
   It takes no `EXPIRES` now.

It checked and found sound: the sequence fencing of the return against earlier and later
orders and against a lapse's reserved numbers; every state `validate` can be handed; the
tolerant target's limits; who may read an NPC leader's orders and when an answer shows; the
plan's bytes and hash for an older plan; that no contact knowledge passes to a recipient.
Building the stories then found one more thing the rule needed: a commander concentrating a
roster of mining rigs would have sent hulls with nothing to fire at the Hostile it was holding
for, so it concentrates armed hulls only.

## MW.15

The story doc 36 owed for SCREEN and SUPPORT flown as delivered orders (doc 44 §7.1; doc 36 cut
GC.0's "Not built"; Q-36-42). No engine code changed: the cut is a test rig and a story.

**Built.**

- **`roster_rig`** (`crates/tacsim/tests/support/field/roster_rig.rs`), the rig that flies a
  detachment's roster into a field in tacsim's own world harness. `fly` is `mush-server`'s
  `pair_field_fixture` on `driving_rig`'s composed world, every step the production command:
  the hire at the Slack Line (`+sortie/crew`, `+sortie/hire`), a leased prospector a slot with
  its operational loans (`+tv/lease go`, `+tv/<n>/permissions go`), the crew brought aboard
  (`+sortie/board`), the voyage to the site, the sub-loan to each NPC
  (`@unitgrant/issue … operate_crew parent=<loan>`), the leader's own boarding
  (`+tv/<n> board`, `+focus`), the plan (`+sortie/draft`, `+sortie/accept`, `@station/handoff`,
  `+sortie/ready`, `+sortie/activate`), each NPC's seat (`+sortie/seat`), the launch
  (`+sortie/launch`) and the refresh until the field command names a leader.
- Its parts are what MW.14a's story needs next: `Plan` takes any billets (a `Leader`, a `Hired`
  crew member by name) and a `commander` by name; `commit` lands whatever a line queued on the
  composed path whoever owns the action (tacsim's, Org's unit grants, Sim's helm hand-off, which
  `driving_rig::drain` drops); `apply_due` lands a due scheduled action on the composed scheduled
  path (Sim's `control_handoff_apply`, the lander doc 36 said such a story needs); `beat` is one
  world second of both; `fence` and `reply` read the order prefill and a sent order's status off
  `+sortie/orders`, as a player's client does.
- **The story** (`crates/tacsim/tests/domain_field/field_mw15.rs`), at 256 on Ahuna Flank and at
  512 on Urvara Reach. A captain leads a mining roster from rig 1 with Ilya Mendes hired into rig
  2. A second captain's armed hull escorts and a third party's armed hull stands off it; the
  escort's `+field/fire` promotes the site and three parties are admitted. Then, by the leader's
  own `ORDER DT:… ` lines:
  1. `2 SCREEN 90`: the order is delivered, stands on Mendes's own frame, the rig turns onto 90
     degrees and the WILCO comes back. The leader drives four cells off and the screen stays on
     its cell and its bearing;
  2. `2 SUPPORT 1`: the rig leaves the cell it screened from, closes to its station on the
     leader's rig, and answers WILCO;
  3. `2 SCREEN 270`: the rig turns onto the new bearing, so the bearing it holds is the order's;
  4. `2 SUPPORT 2` (its own slot): NEGATIVE, and the screen goes on standing.

**Tests.** `field_mw15::mw15_a_rosters_screen_and_support_are_delivered_and_obeyed` and
`…_at_512` in `domain_field`. Local, Windows: 85 s and 81 s, each passing alone. GC.0's two
in-source gates are untouched.

**Divergences.**

- **One NPC flies both orders, in turn.** A Dray-P's bay holds two rigs, so a roster with a
  player leader has one hired NPC (`fly` refuses a third billet). The row's "the screening unit"
  and "the supporting unit" are the same rig under two successive orders, with the screen's hold
  checked while the leader moves away and the supporter's close checked after it.
- **"Keeps to its charge" is the doctrine's station, with the motor's tolerance.** The supporter
  ends within `SUPPORT_STATION_CELLS` (5) of its own plot of the charge plus the kernel motor's
  arrival tolerance; the story allows two cells for that.
- **The escort and the intruder are fixture hulls with players seated** (CF.2's `armed_on` and
  `seat`), not the content's raider on its doctrine, so nobody fires after the opening order and
  the story does not hang on the field's dice. Three parties stand in the field; only the roster
  is under test.
- **The rigs are stood on their cells by a staff hand** after the launch, as every field story
  places its units.

**Not built.** A SUPPORT whose charge is another NPC's rig (it needs a third bay), and SCREEN or
SUPPORT under fire with a Hostile in the screen's arc: the story is about delivery and obedience,
not the stand the doctrine takes. A restart with an order standing.

**Not pinned by a story.** The jammed and the damaged carrier relay holding these two orders back
(FR.5's stories pin that for the orders they fly). A SUPPORT refused for a charge in another party
or outside the field, which GC.0's in-source gate covers and this story covers only for the
recipient's own slot.

## MW.16

Surveyed as [doc 45](45-the-absent-player.md): every way a player can be absent while something of theirs is exposed, what the game does and what an attacker can do today, checked in the tree.
Twenty-five owner questions, Q-45-1 to Q-45-25, open at their defaults; no units of work until they are ruled.
PR #533.

## MW.17

The last decision in the staff view of an NPC (doc 44 §7.1, Q-44-30; first review, finding 8;
second review, §4c row 19).

**Built.** In `crates/mush-npc`:

- **A turn records what woke it.** `NpcHostState::begin_turn` (`runtime.rs`) drains the mailbox,
  so it is the one place that knows; it now writes a `LastDecision` on the NPC's runtime: the
  correlation, when it began, and for the first `LAST_DECISION_WOKE_MAX` (8) stimuli of the batch
  their kind (`Stimulus::kind_name`) and whom each was about. `fire_dialogue` (`module.rs`) adds
  how many public facts and recalled memories the snapshot showed the model
  (`note_decision_context`); guarded and secret facts never cross the async boundary, so the
  count is of public ones.
- **A completion records what the turn came to.** `NpcCompletion::on_complete` writes a
  `DecisionResult`: the model answered with these actions (their tool names), the model did
  nothing, the profile's fallback line with the staff-facing reason (capped at 160 characters),
  or the payload did not decode. A completion for another turn changes nothing
  (`finish_decision`).
- **`@agent/show` prints it**, in two lines after the runtime line (`LastDecision::lines`), and
  `docs/help/npc.md` describes them. The verb is Builder and up, as before.

The record holds kinds, ids and counts. It holds no prompt, no reply and no stimulus text, in
keeping with the audit's rule (`docs/help/llm.md`).

**Tests.** `runtime::tests::mw17_the_last_decision_records_what_woke_a_turn_and_how_it_ended`
(the record, the cap, another turn's completion, the two lines carrying no text) and
`mw17_staff_read_what_woke_a_turn_and_whether_the_model_or_the_fallback_spoke`
(`crates/mush-npc/tests/agents.rs`): through `@agent/show`, a turn in flight, the same turn
answered by the model, the next turn starved of budget and answered by the fallback, and a
player refused the verb. No test makes a provider call. Local: `cargo test -p mush-npc` 150 +
72 + 34 passed; clippy clean with warnings denied.

**Divergences.**

- **The NPC decision call still carries no NPC.** The row asks for `decision_llm.rs` to pass its
  NPC so the per-NPC budget and `@llm/kill <npc>` reach a decision. It cannot be done at that
  call site: the worker lease it is handed (`mush_core::decision::DecisionWorkerLease`) is
  public-safe by design, "no world or private witness reference can cross the async boundary",
  and names a controller and a decision, not an object. Carrying an audit attribution on the
  lease is a change to that contract in `mush-core` and belongs to its own cut.
- **"The knowledge tiers the NPC was allowed"** is reported as a count of public facts and of
  recalled memories. The snapshot carries no other tier, so there is nothing else to report.

**Not built.** The record does not survive a restart (it is runtime state, like the mailbox).
It keeps one turn, not a history. It does not cover a summarise turn or a scene-turn decision
(`npc_decision`), only dialogue turns.

**Not pinned by a story.** A completion whose payload does not decode (the `Unreadable`
result) through `@agent/show`.

## MW.18

The ambient timer scheduled (doc 44 §7.1; Q-44-31, Q-44-47; second review, §4c row 19).

**Built.** In `crates/mush-npc`, with three keys in `[npc]` (`crates/mush-config`,
`crates/mush-composition`, `mushd.toml.example`):

- **The scheduler** (`module.rs`, `NpcTick::schedule_ambient`). Every 30 real seconds the
  agent tick finds the rooms that hold a present player beside an enabled major NPC. For
  each one that is due and quiet it begins one ordinary dialogue turn for one of the
  room's majors, woken by `Stimulus::AmbientTimer` alone. Nothing is read for a game
  with nobody connected.
- **Presence, one definition** (`present_for_ambient`): a player character with a live
  connection, not `DARK`, who typed within `[npc].ambient_active_secs` (600) real
  seconds. A link-dead character is still `World::is_connected` but has no live
  connection; a bare connection (MW.24's SSH `mind push`) is neither connected nor live;
  both fail `World::has_live_conns`, so one test covers the two. The NPC must also be
  able to perceive that player.
- **The room window** (`runtime.rs`, `AmbientState`): a room is due when it has been
  attended for the settle time (300 real seconds) and its last unprompted turn is at
  least `[npc].ambient_window_secs` (1,800) old. The window opens when the turn BEGINS,
  whether or not the model then speaks and whether or not a budget refuses the call, so
  it bounds calls and not only lines. Two majors in one room take the room's turns in
  rotation. Times are real seconds (`World::now`), never the IC clock.
- **The NPC's own window** (`NpcRuntime::last_unprompted_at`): an NPC also takes at most
  one unprompted turn in that window, wherever it walks. This is Q-44-31's "each", kept
  beside Q-44-47's room cap, so a major that follows a player from room to room does not
  speak in each.
- **Quiet**: no NPC of the room, of any tier, is mid-turn, has a stimulus waiting, has a
  memory fold pending, or took a turn within the settle time. An unprompted line never
  talks over a conversation.
- **Under the existing budgets.** The turn is fired through `fire_dialogue`, so it uses
  the major dialogue route, the per-NPC, per-feature and global budgets, the audit row
  and MW.17's `LastDecision` unchanged. Nothing was added to the budget (Q-44-48).
- **A degraded unprompted turn says nothing.** `LastDecision` gains `unprompted`; when the
  model is not reached, a turn somebody prompted still says the profile's fallback line,
  and an unprompted one says nothing. Staff are still told the NPC is degraded.
- **What staff see.** `@agent/show` gains one line, `Unprompted: ...`: never (and why:
  the timer is off, not a major, parked), waiting for a present player, due in so many
  minutes, or when the room last heard a line and how long until the next. Its last-turn
  line already reads `woken by the ambient timer`. `@llm/log npc_dialogue_major` shows
  the call with `actor=-` and `npc=` the NPC (MW.17b).
- **The switch that enables the majors: `[npc].wake_majors`** (default `false`). The
  shipped cast (`crates/tacsim/content/cast/*.json`) authors its eighteen majors
  `"enabled": false`. With the key set, a cast pack CREATES its major NPCs enabled,
  whatever the pack says, and a later upgrade of that pack keeps each major enabled or
  parked as staff last left it (`content.rs`, `applied_enabled`). So it wakes the majors
  of a fresh world at boot, and on a world that already exists it wakes nobody.
  **For MW.3**: set it under `[npc]` in the box's `mushd.toml` (the template is
  `deploy/instance/mushd.toml.tmpl`, which this unit did not touch). On a wiped and
  reloaded box that is all. On a world whose cast is already there, also run
  `@agent/enable <npc>` for each major; the key then keeps them woken across the next
  pack upgrade.

**What the survey had wrong, or left out.**

- "Presence counts link-dead, DARK and idle characters" is true of `players_present`,
  which every stimulus-driven wake uses, and that is left as it is: a stimulus is itself
  something a live player just did, so nothing is spent on an idle room there, and a
  `DARK` wizard who speaks to an NPC is still answered. The narrower definition is the
  ambient timer's alone.
- The eighteen majors are stubs. Each has a one-line background and a persona of a few
  words (four have an empty persona), no goals, no knowledge, no lore and no fallback
  line. Enabled, they will speak from almost nothing. That is content, not this unit,
  and it is the first thing a dev-box playtest will show.
- They stand in six rooms (five in Ceres's Authority, four on Aitken's spine, three each
  at Stickney and Voltaire, two at Ligeia, one in the Ceres Registry). With every one of
  those rooms attended that is at most six unprompted calls in 30 minutes for the whole
  game, which is the figure to hold against the day's budget (Q-44-48).
- A pack re-apply writes the whole profile again and keeps only `public_faction`. So
  without `wake_majors`, a major woken by hand goes back to `enabled: false` at the next
  pack upgrade, and any other NPC that staff parked with `@llm/kill` is woken again by
  one. That is the applier as it was, for every tier, and it is left alone here except
  for majors under the key, which keep what staff left. The general case (a pack upgrade
  quietly undoing a park) has no unit; it wants one.
- Q-44-31 says "at most once every 30 real minutes each"; Q-44-47, the later ruling, says
  a room hears at most one line in a window whichever NPC speaks. Both are built: the
  room cap is the stricter where NPCs share a room, the NPC's own is the stricter for
  one that walks.

**Defaults taken** (none ruled; all cheap to change):

- **The settle time, 300 real seconds**, and the quiet rule. Without them the line would
  come the second a player walks in or in the middle of a conversation. Not a file key.
- **The window opens at the turn's beginning**, so a refused or silent turn uses it up.
- **A degraded unprompted turn is silent.**
- **Rotation** by object id among the room's enabled majors.
- **A restart forgets the windows** (runtime state, like the mailbox): a room may hear a
  second line inside 30 minutes across a restart, once its settle time has passed again.
- **A clock that steps backwards fails closed**: scans stop until the clock passes the
  last scan's second again, and a window or settle time measured across the step reads
  as not yet passed. Nothing extra is spent.
- **`wake_majors` keeps what staff left on an upgrade.** The first build woke every major
  again at each pack upgrade, which undid `@llm/kill`; the outside read of this unit
  caught it. The cost of the rule built instead is that the key alone does not wake an
  existing world.
- **Only majors**, as ruled; Q-44-47's alternative (the timer for minor NPCs) is not
  built and nothing here prepares it beyond the tier test in `ambient_speakers`.

**Tests.** No test waits on a real clock or calls a real provider.

- `crates/mush-npc/tests/agents.rs`, on a clock stepped by hand with a mock provider:
  `mw18_a_room_hears_one_unprompted_line_in_its_window` (two majors, one line, the audit
  row, both staff lines, the whole window silent for both, the other NPC next);
  `mw18_only_a_present_player_draws_an_unprompted_line` (link-dead, `DARK`, idle and
  bare for two hours: no turn begun, no room tracked, the NPC never woken; then served
  once the `DARK` one is seen); `mw18_a_parked_or_lesser_npc_never_speaks_unprompted`;
  `mw18_an_unprompted_turn_waits_for_quiet_and_says_nothing_when_it_degrades`;
  `mw18_a_zero_window_turns_the_ambient_timer_off`;
  `mw18_presence_is_read_from_the_engines_own_link_state` (real `ConnUp`, `BareUp`,
  `Cmd` and a lost `ConnDown` through `Engine::step`);
  `mw18_an_npc_that_walks_to_another_room_keeps_its_own_window`.
- `runtime::tests::mw18_a_rooms_window_is_due_once_and_rotates_its_speakers`;
  `content::tests::mw18_wake_majors_creates_a_major_enabled_and_keeps_what_staff_left`.
- `mush-config` `mw18_npc_ambient_keys_default_and_parse`.
- `crates/tacsim/tests/domain_economy/stations.rs`
  `mw18_wake_majors_applies_the_shipped_principals_enabled`: the production roster with
  the key set boots the real cast with all eighteen majors enabled;
  `ic1_the_principals_never_wake` is the same boot without it.
- Mutation check: with the window, the `DARK` test, the live-link test, the tier test and
  the silent-fallback rule each broken, four of the five stories fail.

**Not built.**

- Nothing enables the majors on any box: that is MW.3, with the owner.
- `deploy/instance/mushd.toml.tmpl` has no `wake_majors` line (the deploy chain's file).
- The windows are not persisted.
- The stories drive the NPC module's tick by hand. No story runs the ambient timer
  through the production world task, and none flies it against the live cast.

**Not pinned by a story.** A room over the 256-room table cap being served later (a room
that has heard a line keeps its row for the whole window, so 256 such rooms keep a new
one out for up to 30 minutes); an NPC in an exclusive scene being passed over; a speaker
inside a cooldown longer than the settle time being passed over; the per-NPC ceiling
refusing an unprompted turn (the global budget's refusal is the one flown); a real pack
upgrade over a parked major (`applied_enabled` is pinned as a function, and the fresh
boot is flown against the real cast).

**Read from outside.** An independent read of both units (MW.17b and MW.18) checked the
claims above against the code and found four things, all fixed in this unit: the
`wake_majors` rule above, stray spaces in the staff line for a silent unprompted turn,
the missing per-NPC window, and a help sentence that overstated what `@llm/kill` stops
when a call is already under way.

## MW.19

The bot's minds lane on the nightly (doc 44 §7.1; Q-44-33; §4c row 20). Built on main
`63dbcb3f6`. Q-44-33 is ruled at its default.

**Four things the owner should read first.** The lane is the first player to walk the
market's whole path on a world brought up by the runbook, and it met these on the way.

1. **The counter had no door.** The bring-up digs the venue room (`@dig Named launch
   counter`) and a dug room has no exit. `@mind/publish`, `+minds`, `+minds/buy` and its two
   steps are refused anywhere else ("Listing and buying minds happen at a market counter."),
   and every story and the bring-up's own seed line reach the room by `@teleport`. So on a
   world brought up for dev1 or dev2, no player could list a mind or buy one. **Fixed here,
   in the bring-up:** the Active boot's last step (and a second run on a box that is already
   live) opens `Mind Counter <MC>` (`mc`, `minds`) from the Registry & Co-op Desk at Ceres
   Freeport into the counter, and `Registry <R>` (`out`) back. **The place is a default the owner should rule on**: the
   desk is where a new captain already stands to enrol, and the room's text has the
   Registry's half of it. The counter itself is still an undescribed room.
2. **Since MW.24, an offline player's `mind push`, `pull` and `check` were refused.** MW.24
   made the SSH exec a bare connection, which is never a live one. The workshop's author
   session asked for a live connection (`WorkshopCommands::principal`), so every typed mind
   request from a player with no shell open read "No owned mind or authoring job is
   available.", a create included. MW.24's own story could not see it: it drives a toy
   service, not the workshop. With a shell also connected the exec worked. **Fixed here**
   (`minds/workshop/`): a connection bound to the actor stands in for the live one, and the
   session such a request opens is ended when it is answered, since no hook ends it. The
   lane holds it from outside: the rig runs `mind push`, `mind pull` and `mind check` (a job)
   for a login with no session open before either bot connects. The author bot's own five
   requests do not hold it: its line session is open when it makes them. **One sibling is
   not served:** a check or a save of a forked mind (one the character did not write) from
   an offline player. Its result is delivered through the line session's author session, so
   it was queued, held the account's one job slot and then answered "unavailable"; it is now
   refused at once, in words ("a forked mind is checked and saved from a logged-in
   session"). Serving it is not built.
3. **A rating talks to its author all the way through.** MW.5 read this in the code and did
   not see it on a connection. It is real: the author's session receives the engine's
   `Queued.` and `Done.` for every step the Registry takes on its behalf, 4,294 lines over
   the lane's 24-minute rating. **Not fixed** (the engine's submit path and the world task's commit
   line, for a continuation the player did not type). The lane counts them and holds the
   truth as an expected failure: the job says `KNOWN DEFECT (expected to fail)` while they
   come, and fails the day they stop, until the marker is taken out. The launch under a
   licence and the recover print the same pairs for the claim's steps.
4. **An IC launch tells the operator nothing about the mind.** `+tv/1 launch
   mind=filed:<licence> go` answers as a plain launch does. No line names the mind sealed
   aboard, `+tv` does not, and the licence's row in `+minds/holdings` reads the same with the
   rig out as with it home. Only a refusal, or a lapsed licence falling to the stock
   doctrine, is spoken. With no field on the site the mind never thinks either. So the
   lane's "flies it on an IC sortie" is: the launch under the licence is not refused and
   does not fall to the doctrine, the rig stands at its ramp, comes home, and the hull sails
   home. **A sentence for the operator is the gap; no unit owns it.**

**Built.**

- **The lane** (`crates/mush-bot/src/mine/market.rs`, a sub-machine of the mining lane as
  the escort's is): two roles, two bots, two logins.
  - `--minds author --charter <name>`: the runner writes the bot's own mind over the SSH
    mind service before the lane types a line (`mind push` create and replace, `mind check`,
    `mind push` save; `transport::mind_exec`, the bot's own JSON from `help mindcraft`). The
    lane enrols with the Co-op (the only way a new character has a credit account; the stake
    pays the filing fee), reserves the charter, asks for the rating, waits for `Registry
    work finished.`, files, reads its filed number from `+minds/holdings`, walks through the
    counter's door, lists the mind sealed and reads its own row on `+minds`.
  - `--minds buyer --charter <name> [--minds-site <site>]`: the escort's kit-up and the
    bunkers (the armed rig is the one Occator leases with a core; a prospector carries
    none), then the counter, read every 20 s until the charter is on it; the purchase in
    its three confirmed steps and the licence number from `+minds/holdings`; the simulator
    (`SIM TEAM red`, `SIM MIND filed:<licence>`, `SIM START`, `BOARD`, `MIND`, `EXIT`, the
    match's end, `SIM REPORT`); then aboard, the hop, `+tv/1 launch mind=filed:<licence>
    go`, `+tv/1` at the ramp, the recover, and the hop home.
  - A submitted step is done only on a `Done.` that follows its own sentence in the window
    (the engine names no step on its `Done.`, so that is as near as a reader can hold it),
    and every such step is read back by the next one: the holdings, the counter's row. A
    refused or busy answer is typed again; a refusal that is the game's ruling (the counter rule, a
    licence the centre does not know, any of the launch's nine refusals, a licence fallen
    to the doctrine) ends the lane quoting it.
- **The lobby's vocabulary** (`arena/lobby.rs`): `LobbyLine::SimMind(licence)`, `SIM MIND
  filed:<licence>`. The arena pilot's machine never queues it; the market lane builds its
  simulator lines from `LobbyLine`. `MIND`, `EXIT` and `SIM REPORT` are the market lane's own
  three and stay out of the arena pilot's reach (`tests/arena_e2e.rs`' `forbidden` is
  unchanged).
- **The sentence table read at last** (`mine/minds.rs`): the lane reads the `market`,
  `dispatch` and `claim` families, and a new `walk` family holds fifteen sentences the six
  censused modules do not: the engine's `Queued.`, `Done.` and its refusal, the Registry's
  five notices about a rating, the simulator's four about a chosen mind and `SIM REPORT`'s
  two, and the doctrine fallback. Most are written inline where the game prints them, so
  each needle is held to the game's own source text (`include_str!`) or to its constant.
  The same way, tests hold the mind service's JSON to `mush_proto`'s own types (the bot
  writes it by hand; it may not depend on the game), and the lane's rooms and exit words to
  the bring-up's constants and the shipped out-of-character area.
- **The rig** (`scripts/bot-minds-lane.sh`): a scratch instance rendered by the deploy's
  `render.sh` from an `instance.env` with dev2's switches (`MINDS_LAUNCH=on`,
  `REGISTRY_PULSE_MS=50`, `RAID_DIRECTOR=true`, `IC_RATIO=24`, the LLM off), its own Redis,
  the split pair, and the launch brought up by `launch-bringup.py --no-systemd --no-seed
  --no-llm-check`. The two logins are registered at the front door with `create` and each
  adds its own SSH key with `@sshkey/add`: the player's hand, which the bot is forbidden.
  After the bring-up no line is typed by staff. It needs no root.
- **The nightly job** `bot-minds-lane` (`.github/workflows/ci.yml`): its own job, runner,
  instance, ports and Redis container; nightly and on a dispatch; the bots' transcripts,
  reports and the instance's logs uploaded. `.github/nightly-paths.txt` maps it;
  `shipped_configs.rs` holds the script's `instance.env` to the template's keys.
- The game: the workshop fix and its test
  (`mw19_an_offline_authors_request_over_a_bare_connection_is_served`), and the counter's
  door in `deploy/instance/launch-bringup.py` with the README's step.

**The mind and the pace, and why.** The rating is the lane's length, so both were chosen
for it and measured on conclav.dev (debug build, `REGISTRY_PULSE_MS=50`):

| Mind | Rating, asked to finished |
|---|---|
| A copy of Standard Doctrine (`@mind/new … FROM Standard`) | about 36.5 min (182 unit-minutes) |
| The lane's own (`mine/market_mind.rhai`: stand, and answer fire by the stock rules) | about 24.3 min (130 unit-minutes) |

The lane's mind is the second: a script of the bot's own, the quickest that the Registry
rates, and one that never asks its hull for what the hull cannot do. The pulse is the dev
boxes' 50 ms, because the rig is meant to be what a dev box is; MW.1 measured 10 ms at 15%
less, which is not worth leaving the dev boxes' value for.

**Rates.** The instance runs the live game's 24:1 (`IC_RATIO=24`) from boot to the end:
the Co-op's readiness, the site's pad, both hops, the launch and the recover are all the
game's own clock at that rate. Nothing in the lane runs at 3,600:1. The Registry's trials
run at the Registry's own pace (the pulse above), and the simulator's match on its own arena
clock; neither is the IC clock.

**Time, and where it goes** (conclav.dev, the script's own log and the bots' milestones):

| Part | Seconds | Paced by |
|---|---|---|
| The launch's three boots | 61 | the bring-up |
| The world, its front door, two logins and their keys | 25 | the script |
| The author's mind over SSH (five requests), then the Co-op's enrolment | 84 | the Co-op's readiness after a fresh boot, at 24:1 |
| The charter's reservation and the rating | 1,448 | the Registry's pulse; the rating alone is 1,446 |
| The filing, the holdings, the counter's door, the listing and its row | 6 | the engine's commits |
| The buyer's purchase, from the row to the licence in its holdings | 23 | the engine's commits |
| The simulator: out of character, the claim, the pod, the match, the report, back | 285 | the match's own arena clock (4:33 until the machine won) |
| Aboard and the hop to Kerwan Floor | 162 | 24:1 |
| The site map, the launch, the rig at its ramp, the recover | 38 | 24:1 (a launch and a recover are 300 IC s each) |
| The plan, the hop home, the dock | 159 | 24:1 |
| **The whole script** | **2,295** (38 min) | |

The buyer's kit-up (94 s) and bunkers (190 s, most of it waiting for Kerwan's pad to open)
run beside the rating and add nothing. Two thirds of the lane is the rating.

**Runs.** On conclav.dev (Linux, 32 cores, shared with other sessions; debug binaries of this
branch; each run on its own scratch instance and Redis):

- **Run 1: PASSED in 2,295 s.** The figures above. The author typed 12 lines and the buyer
  152; the buyer read the counter 61 times before the row was on it.
- **Run 2: PASSED in 2,285 s** (the rating 1,438 s), the same tree.
- **Run 3: PASSED in 2,262 s** (the rating 1,440 s), after the second reader's fixes: the
  registration that waits for each answer takes 2 s where the paced one took 22.
- **The bring-up a second time over run 1's store:** the Active boot read "the counter
  (#811) has its door from the Registry & Co-op Desk (#105)" and opened nothing.
- By hand, before the lane existed, on two more scratch instances: the whole path walked
  line by line (the sentences in the lane's unit tests are those transcripts'), and the two
  ratings of the table above.

**The nightly job, by dispatch, on GitHub's hosted runners** (`gh workflow run ci.yml`; the
job builds the three debug binaries in 5 to 8 minutes, then runs the script):

| Run | Head | The script | The rating | The job |
|---|---|---|---|---|
| 43329751649 | `8d05f8e31` (before the second reader's fixes) | PASSED, 2,243 s | 1,404 s | 43 min |
| 43334739075 | `cfd93f5c1` | PASSED, 2,246 s | 1,410 s | 44 min |
| 43334743232 | `cfd93f5c1` | PASSED, 2,422 s | 1,572 s | 49 min |
| 43334747198 | `cfd93f5c1` | PASSED, 2,280 s | 1,447 s | 45 min |

The three on `cfd93f5c1` were started together and ran side by side, each on its own
runner; they are three green dispatches of one head, not three nights. A hosted runner
rates no slower than conclav.dev: the rating is the Registry's pace, not the machine's.
Every run counted 4,292 or 4,294 unasked lines. In run 43329751649 another job's cell was
red once, `test (rest-server)`, on mush-server's
`player_market_commands::rate_queues_and_cancels_a_genuine_registry_preparation` (141 s,
no assertion text in the log); the same cell passed in the pull request's run and in all
three dispatches of the final head. It is a story of the Registry's own service, which this
unit does not touch; it was not run down.

The unit tests: `cargo test -p
mush-bot --lib` (529 passed, the census among them), the workshop's
`mw19_an_offline_authors_request_over_a_bare_connection_is_served` (with the old gate put
back it fails on "No owned mind or authoring job is available."), `shipped_configs`, and
`cargo test -p mush-contract-tests`; clippy with warnings denied and rustdoc on `mush-bot`,
`mush-config` and `tacsim`. All on Linux; the bot's library and tacsim's were type-checked
on Windows.

**What the lane leaves out.** Nothing of the unit's list is left out for want of a launch
route: write, file, rate, list, buy, fly in the simulator and launch on an IC sortie are
all walked by a player's own lines. What it does not do:

- **The mind does not think on the IC sortie** (item 4 above): no field opens. A field
  needs a second party on the site; MW.5's filed-mind story and MW.21 hold a mind in a
  field.
- **No restart.** Q-44-33's reason names "five modules and a restart"; the lane restarts
  nothing. A restart in the middle of a rating, or with a claim open, on the split pair is
  not walked by any bot.
- **The author's own licence is not flown**, a renewal, a resale and a fork are not walked.

**Found, beyond the four above.**

- **The Co-op is not enrolling for about 80 s after a fresh boot**, and a site's pad is
  closed for about 3 min. Both are the game's own readiness and the lane waits them out
  as the mining pilot does; they are part of the lane's length.
- **`mind pull` and `mind push` refuse `list`** ("Operation does not match the SSH command.")
  though the request type has it: no exec command carries a library listing.
- **A logout while the hull is under way hands it to the space backup mind at once**
  (`[SPACE] Bridge link restored; the backup mind stands down` on return). Met by hand, not
  by the bot, which stays connected; noted because a player who types `QUIT` in a two-minute
  hop reads it.

**Defaults taken, for the owner.**

- The counter's door and its place (item 1). Moving it is the three constants at the head
  of `launch-bringup.py` and the bot's walk to it (`market.rs`, `Route`); a bot test fails
  if the two disagree.
- The lane's mind is not a copy of Standard Doctrine. A House would not buy it; a player
  does.
- The site is Kerwan Floor (two real minutes out); `SITE=` or `--minds-site` names another.
- The debug binaries, not a release build: the release build is half an hour of thin LTO on
  a hosted runner and the lane is paced by the game's clocks. The rating took 23 to 26
  minutes on the hosted runners and 24 on conclav.dev.
- The known defect of item 3 passes the job. If the owner would rather the nightly be red
  until it is fixed, it is one line of the script.

**Not built.**

- **The three real nights.** The unit's row asks for three nights green. Three dispatches
  of one head stand in for them (above); the nights are observed after the merge, on the
  job `bot-minds-lane` ("MW.19 the bot's minds lane (the mind market over SSH)"). The
  nightly-report job lists it by that id.
- A fix for item 3, and a sentence for item 4.
- The author told why a rating failed (MW.5 left it for this unit): the lane's author
  fails with "has no finished Registry rating" after the Registry's `finished`, which is all
  the game says. The trial's diagnostic is still test-only.
- More modules in the census. The `walk` family is held to source text, not censused.
- A field in the IC half, and a restart (above).

**Not pinned by a story.** The door on a box that is already live: the second-run path
hangs it on the running world through the same function, and it was run only by the
bring-up's own Active boot (twice over one store, the second finding the door there). The
workshop's session release when an exec is abandoned with its job still running (the
session then lasts until the character's next logout; bounded by `MAX_SESSIONS`). The
refusal of an offline fork's check (no fixture makes a forked mind in the workshop's unit
rig).

**Read from outside.** A second reader was given the diff and the house rules, not this
note, and asked to break it. It found no way for a request to reach another character's or
another account's session, job, draft or reply, and nothing wrong in the closed vocabulary,
the row readers or the job's wiring. What it did find, and what was done:

- *A forked mind's offline check was queued and then always failed, holding a job slot and
  leaving a session.* Refused at the door now (item 2).
- *The lane never drove the fixed path for a job:* the author's requests come from a
  logged-in bot. The rig's offline probe now runs a `mind check` as well, and the note says
  which requests prove what.
- *The unit test claimed more than it showed* (its "foreign" read went over a live
  connection; after the bare connection closed it asked nothing). It now reads as a second
  offline character on its own bare connection, asks as that character on the first one's
  connection, and asks again after the close.
- *The door could stop the Active boot before the NPCs were enabled and the lent hulls
  returned, and `--llm-check` had begun to write to the world.* The door is the boot's last
  step; `--llm-check` opens nothing; only a box that is already live gets the door on its
  running unit. The "already there" test no longer depends on the Exits line not wrapping.
- *`Done.` was taken from anywhere in a step's window.* It now has to follow the step's own
  sentence, with a test; the docs say what that does and does not prove.
- *Room names, the door's word and the mind service's JSON were tied to nothing.* Three
  tests tie them (above); `.github/nightly-paths.txt` gained the wire types, the engine and
  the world task.
- *The time bounds contradicted each other* (a two-hour wait under a hundred-minute run).
  The rating is waited ninety minutes, the author runs a hundred at most, the buyer a
  hundred and forty, inside the job's hundred and eighty.
- *The script* resolved its paths after changing directory, could exit silently on an
  unreadable host key, did not stop the bots on an early exit, and paced the registration
  with fixed sleeps. All four fixed; the registration now waits for each answer.
- *The runner lost its records of the mind service's replies when a step failed.* Each is
  written as it arrives.
- *The docs claimed dispatches that did not exist yet and an IC flight the lane does not
  show.* The row and this note now say what was run and what the IC half is.

Left as found: `confirmed` cannot attribute a `Done.` to a step beyond its order (the engine
gives nothing to attribute by); the buyer reads the counter for up to a hundred minutes with
a leased rig and bunkers aboard, which cost it nothing in the two runs.

## MW.20

Minds measured at size, as a measurement and not a gate (doc 44 §7.1, Q-44-34, Q-44-48).
PR #577. Harness and test code only: no game code changed, and no figure is asserted by
any job.

**The short of it.** At 100 units nothing misses on the measuring box, the Rhai seats
included, alone or with eight bubbles beside it. A step of a 100-mind field or match takes
1 to 2 ms against a budget of 20 to 40; the think pool is never late; no bubble's step is
ever held. The misses are past the target (a 300-unit match with 100 or more Rhai seats,
a 200-unit one with every hull seated) and on a slower box (the October miss was on the
live box, which is half as fast a core).
Three things were found that the row did not ask about: an open bubble costs the world
thread more to save than to step; the shipped lobby cannot field more than twelve units,
so the 100-unit match is measured on the kernel and not through `SIM START`; and hulls
with no order do not close on each other, which decides what a field measurement must
look like.

### What was built

- **`crates/tacsim/tests/domain_world/minds_at_size.rs`**, four `#[ignore]`d lanes on the
  by-hand tier (`test-tiers.toml`, with the `MW20_*` knobs; the recipe is a comment in
  `ci.yml`). Each prints `MW20` lines and asserts only that its rig is sound.
  - `mw20_a_field_of_minds_at_size`: GC.12's field (Ahuna Flank's own ground, priced as
    the host prices it) with 100 hulls in three or six parties, the production fill or
    Rhai seats, opened as `arena_task::spawn_field` opens one and stepped on the task's
    policy: a wake's thinks go to a pool after the step, are awaited and landed at the
    next wake's head, and are awaited before a due snapshot's encode. The step is timed
    alone, as the task's `took` is.
  - `mw20_eight_bubbles_think_on_the_pool`: eight customs-picket bubbles opened through
    player commands, the think host holding late steps as the server's does, each module
    tick of a second fired alone so the space tick's cost is read apart from the rest.
  - `mw20_a_field_and_eight_bubbles_on_one_server`: both, on one world and the store's
    one pool, the field stepped in real time on a thread of its own.
  - `mw20_an_idle_fire_by_tick`: the stocked world at 24:1 with nothing in flight.
- **`crates/tacsim/tests/harness.rs`** (the fill harness). The minds lane takes a sizing
  variant, `rhai_authored`: its slots a side at Class I on the scripted engine, Standard
  Doctrine, the rest the ROM, which is the fill as it was before the fill decision.
  `TACSIM_MINDS_RHAI_SEATS` is its seats a side. It runs only when
  `TACSIM_MINDS_VARIANTS` names it; it is not one of the weekly four, no baseline carries
  a block for it and the delta never reads it. The mock cost run prints one more line,
  `harness_cost_prompt`: the requests it was handed, sized by `mush-llm`'s own budget
  estimate.
- **`crates/tacsim/tests/domain_world/voyage.rs`**: `flown_into_pickets`, eight pilots who
  charter, load, board and clear Ceres for eight venues in one IC second, on the shipped
  lane seeded with a picket waypoint on each of those routes.

**Read-only hooks in the game: none.** The lanes use what is already public: the arena's
`step`, `deliver_answers` and `mind_runner`; the engine's scheduler, which a test may
drain and refill (that is how one tick is fired alone, through the engine's own
`fire_due`); the think host's `stats`; `thinks::run_on_the_wall_clock`.

### How each figure was taken

- **Host.** conclav.dev (`hg.conclav.dev`, AMD EPYC 9J45, 32 vCPU, 62 GB), as `ghrunner`
  in a throwaway clone, release profile (`lto = "thin"`), the code as committed at
  `fb39c2b5`. The box is shared: other sessions were building and testing on it, and its
  one-minute load average was 5 to 10 through the runs (each run's is in its log). No
  `Gate (VPS)` run was in progress or queued. Read medians; a single worst wake may be
  somebody else's link step.
- **Runs.** Three of each configuration unless a row says otherwise. A table cell is the
  median of the runs' figures with the worst run's figure in brackets. The field and the
  match are deterministic (the same state hash every run), so only the timing varies.
- **Not through a server.** No lane boots `mushd` or the split pair. The field and the
  match are the kernel stepped by the lane on the task's policy; the world is a
  `TestWorld` on the production composition (tacsim's own roster, as the stories boot
  it), ticked by the lane. There is no gateway, no Redis, no persist writer and no
  player's session, so nothing here is the cost of sending a frame or writing a batch.
- **The live boxes against this one**, as far as the repo says. dev1 and dev2 are on
  conclav.io: 8 cores and 31 GB, shared by both games and the production Redis
  (`docs/ops.md`). Two readings of the same work exist on both boxes. Standard Doctrine's
  rate, which the minds lane measures at its head, was 9.4 to 10.0 budget units a
  microsecond on conclav.io on 2026-10-09 (worked back from the computed lenses in
  `match_/seal.rs`'s decision log) and is 20.1 here: half the speed. The 300-unit CI
  cohort's wake p99 was 8.8 to 10.0 ms there (16c T-109) and 6.4 to 7.9 here (doc 43
  TS.21): 1.3 times. So read every time below as 1.3 to 2 times longer on the live box,
  on a quarter of the cores, beside a second game. conclav.io had 4 vCPU when both of its
  readings were taken and has 8 now; nothing has been measured there since. No run of
  this unit touched conclav.io.

### 1. A simulator match of 100, the native doctrine

`tacsim_harness_arena_minds` at `TACSIM_UNITS=100`, the authored reach, 30 arena seconds
after the warm-up, the tempo pinned to `Engaged`, a seeded quarter of the wakes one
substep late (the lane's own rule: a late wake steps two substeps and runs the first
one's thinks inline, inside the step). 97 wakes a run. The budget is the lane's lenses:
20 ms a wake at the 99th percentile over the on-time wakes, 32 ms a 500 ms window.

| Field | Wake, median | On-time wake p99 | Late wake p99 | Worst wake | Window p99 | Think CPU a window, p99 | Batch wait p50 / p99 | Held |
|---|---|---|---|---|---|---|---|---|
| the production fill (50 a side at Class I, native) | 1.63 ms | 2.04 (2.18) | 3.21 (3.41) | 3.97 | 3.82 (3.91) | 0.31 (0.35) ms | 72 / 175 µs | 3 of 3 |
| every hull the ROM | 1.50 ms | 1.89 (2.05) | 3.30 (3.32) | 3.41 | 3.67 (3.87) | 0.31 (0.41) ms | 75 / 127 µs | 3 of 3 |
| the production fill, no wake late (1 run) | 1.56 ms | 2.02 | | 2.32 | 3.91 | 0.37 ms | 99 / 163 µs | yes |

It holds with a factor of nine to spare on the wake and eight on the window. A native
think is 4 µs at the median and 12 at the 99th percentile; building a hull's view is 20.
The task's thread is the step; the pool's two threads spend a third of a millisecond in
every half second.

**The world thread's share of a match was not measured at 100.** A match runs on a task
of its own; what reaches the world thread is the egress (frames for seated pilots, ledger
chunks, a snapshot every five seconds). The harness's fan-out lane measures that through
the production task and the real egress, at 24 pilots and no fight (below). A 100-unit
match through the production task would need the staff path and more muster places than
that lane's twenty-four, and this unit did not build one.

The fan-out lane, as it stands (`tacsim_harness_fan_out`, three runs): one match through
the production task and the real egress, 24 seated pilots driving, no contact made. One
apply of a pulse's frames costs the world thread 3.36 ms at the median (3.39), 5.20 at
the 99th percentile (5.35) and 66 to 69 ms once a run, at the match's opening. That is 95
pulses in 30 arena seconds, so about 11 ms of the world thread in each second for 24
pilots, and a pilot's panel is 12.7 KB a pulse. It is the players' share at the target's
24 seats, and one apply is more than a step of the 100-unit match above; the 76 machines
beside them would add ledger lines and no frames.

### 2. An IC field of 100, the native doctrine

Lane 1, 120 arena seconds a run (460 wakes, every one at `Engaged` tempo), no wake late,
the pool two threads. Three parties are miners, their escort and a raider; six are GC.12's
own. The field is fought: 80 of 100 hulls stand at the end with three parties, 68 with
six. The budget is the one GC.12 set the field's cap against,
`[tacsim.harness].engaged_p99_ms`, 40 ms a pulse; the watchdog's `tempo.stall_ms` is
2,000.

| Field | Step, median | Step p99 | Worst wake | 500 ms window p99 / worst | Batch wait p50 / p99 | Think CPU a window | Held |
|---|---|---|---|---|---|---|---|
| three parties | 1.08 ms | 1.67 (1.70) | 3.52 | 3.29 / 5.12 | 66 / 149 (173) µs | 0.14 ms | 3 of 3 |
| six parties | 1.25 ms | 1.81 (2.21) | 2.51 | 3.57 / 4.76 | 63 / 144 (155) µs | 0.13 ms | 3 of 3 |
| three parties, a quarter of wakes late | 1.27 ms | 3.27 (3.47) | 4.33 | 4.61 / 5.28 | 84 / 200 (216) µs | 0.16 ms | 3 of 3 |
| three parties, paced in real time, 60 s | 2.12 ms | 3.01 (3.13) | 3.88 | 5.99 / 7.22 | 59 / 240 (311) µs | 0.30 ms | 3 of 3 |

Paced in real time a step costs twice what it costs back to back: the task sleeps 250 ms
between wakes and comes back to cold caches. That row is the one to hold against a live
box. The loop woke 0.16 ms behind its due time at the 99th percentile, 2.4 at worst, and
no pulse lagged.

What the field hands the world thread, by size: its ledger is 21 KB a second with three
parties and 29 with six, and its snapshot is 2.2 to 2.5 MB every five seconds
(`[tacsim.arena].snapshot_ms`), encoded on the task's thread in 9 to 10 ms (16 at worst).
What the world thread spends taking them was not measured.

### 3. The same on Rhai seats

**It does not miss at 100.** The row expected it to.

*The match*, `rhai_authored`, three runs a row (six for the last), the same field and
lenses as §1:

| Rhai seats | On-time wake p99 | Late wake p99 | Worst wake | Window p99 | Think CPU a window, p99, against the lens | Rhai ops in a window, peak, of 200,000 | Batch wait p50 / p99 | Held |
|---|---|---|---|---|---|---|---|---|
| 12 | 1.79 (1.90) | 3.50 (3.57) | 5.53 | 3.57 (3.66) | 1.12 (1.12) of 12.5 ms | 10,116 | 81 / 409 µs | 3 of 3 |
| 24 | 1.91 (1.98) | 3.74 (3.99) | 4.81 | 3.81 (3.82) | 1.71 (1.98) | 16,836 | 131 / 510 µs | 3 of 3 |
| 48 | 1.98 (2.06) | 4.27 (4.46) | 5.46 | 4.27 (4.46) | 2.78 (2.83) | 30,290 | 376 / 759 µs | 3 of 3 |
| 72 | 1.96 (1.96) | 4.40 (4.66) | 6.05 | 4.40 (4.51) | 3.66 (3.73) | 47,417 | 490 / 985 µs | 3 of 3 |
| 100, every hull | 2.22 (2.32) | 4.94 (5.19) | 6.55 | 4.94 (5.30) | 5.24 (5.49) | 62,788 | 770 / 1,290 µs | 6 of 6 |

*The field*, lane 1, three parties unless said, three runs a row:

| Rhai seats | Step, median | Step p99 | Worst wake | Batch wait p50 / p99 | Think CPU a window | Rhai ops a second, of 400,000 | Held |
|---|---|---|---|---|---|---|---|
| 12 | 1.13 ms | 1.80 (1.87) | 2.05 | 191 / 575 (601) µs | 0.60 ms | 10,056 | 3 of 3 |
| 24 | 1.13 ms | 1.73 (1.77) | 1.99 | 274 / 709 (755) µs | 0.88 ms | 18,408 | 3 of 3 |
| 48 | 1.13 ms | 1.76 (1.88) | 2.33 | 436 / 974 (986) µs | 1.52 ms | 34,686 | 3 of 3 |
| 72 | 1.16 ms | 1.81 (1.81) | 2.26 | 607 / 1,400 (1,420) µs | 2.18 ms | 53,079 | 3 of 3 |
| 100 | 1.12 ms | 1.74 (1.76) | 4.11 | 787 / 1,735 (1,758) µs | 2.85 ms | 72,009 | 3 of 3 |
| 100, six parties | 1.29 ms | 1.82 (1.87) | 2.03 | 744 / 1,728 (1,731) µs | 2.77 ms | 68,751 | 3 of 3 |
| 100, a quarter of wakes late | 1.29 ms | 4.79 (4.88) | 5.22 | 823 / 1,824 (1,936) µs | 2.83 ms | 72,009 | 3 of 3 |
| 100, paced in real time, 60 s | 2.17 ms | 3.60 (3.62) | 5.44 | 51 / 1,313 (1,328) µs | 3.87 ms | 76,336 | 3 of 3 |

*Where the time goes.* A Rhai think of Standard Doctrine is 45 to 85 µs at the median
(about 800 budget units), 80 to 175 at the 99th percentile and under half a millisecond
at worst but for one of 3.5 ms; the native think it replaces is 3 to 4. With every seat on Rhai that is 2.8 ms
of the pool's threads in each half second where the native fill spends 0.14: twenty times
the thinking, and still under one per cent of one core. None of it is on the task's
thread unless a wake is late: an on-time step is the same with 100 Rhai seats as with
none. A late wake runs the first interval's thinks inline, which is the only place the
seats reach the step: a late wake's 99th percentile goes from 3.7 ms native to 5.1 with
100 seats.
The batch a wake hands the pool is back in 0.8 ms at the median and 1.8 at the 99th
percentile, against the 250 ms to the next wake.

*At what seat count it stops holding today.* Not at any count a 100-unit field or match
can hold. Past the target, on the 300-unit field the fill decision was taken on:

| Units | Rhai seats | On-time wake p99 | Late wake p99 | Window p99 | Think CPU a window, p99, against its lens | Rhai ops in a window, peak, of the pool's | Held |
|---|---|---|---|---|---|---|---|
| 200 | 200, every hull | 5.02 (5.09) | 11.81 (11.87) | 11.81 (11.87) | 10.62 (10.63) of 9.35 ms | 125,645 of 150,000 | 0 of 3: the thinking, by 14 per cent |
| 300 | none: the production fill | 8.33 (8.86) | 15.21 (15.34) | 16.62 (17.35) | 1.08 (2.09) of 6.10 ms | 0 | 3 of 3 |
| 300 | 72, what admission admits | 8.74 (9.08) | 16.87 (17.71) | 17.03 (17.71) | 5.77 (5.94) of 6.23 ms | 51,641 of 100,000 | 3 of 3, the thinking at 93 per cent of its lens |
| 300 | 100 | 9.53 (9.93) | 17.85 (19.24) | 17.98 (19.24) | 7.16 (8.26) of 6.23 ms | 67,731 | 0 of 3: the thinking, by 15 per cent (33 at worst) |
| 300 | 150 | 9.10 (9.97) | 18.19 (20.70) | 18.19 (19.96) | 8.78 (8.83) of 6.23 ms | 95,705 | 0 of 3: the thinking, by 41 per cent |
| 300 | 200 | 12.53 (12.98) | 21.66 (27.47) | 22.23 (26.44) | 11.39 (12.23) of 6.23 ms | 129,603 | 0 of 3: the thinking by 83 per cent, the pool's operations a window by 30 |
| 300 | 300, every hull | | | | | | does not open |

The box's load average was 12 through these runs, higher than for the tables above.

So on this box the seats stop holding between 72 and 100 at 300 units, and with every
hull seated at 200 units. **What trips is never the step.** The wake and the window hold
their lenses to 200 seats at 300 units (12.5 ms of 20, 22 of 32). What trips is the lens
on the pool's own threads: the thinking in a window against what the pool's whole
allotment would cost at the host's measured rate, with a quarter over. Standard Doctrine
runs at 12 budget units a microsecond in a fought field and at 20 in the lane's
calibration (one hull, one thread, a warm view), so the lens is passed with the pool two
thirds spent. That lens was written to bound variance, not load, and it is the first
thing a wider use of Rhai seats would have to look at again.

The 72 that hold at 300 units are exactly what admission admits (`match_::pool`: 36 a
side at Class I). The rows past it field seats the lobby would refuse; the lane fields
them on purpose.

With all 300 hulls seated the arena refuses the check-out before a step is taken:
`SnapshotCapacity`, "snapshot needs at least 505630786 bytes; maximum is 503316480". The
snapshot's worst-case bound for 300 scripted minds is past its 480 MB ceiling.

*Why it missed in October and holds here.* The decision's runs were a 300-unit field with
72 Rhai seats. On ALIEN18 they missed the per-wake lens alone (17.3 to 20.8 ms against
12), and that lens then counted the late wakes, which step two substeps by design; it
has since been asserted over the on-time wakes (16c T-108) and moved to 20 ms (T-109). On
conclav.io they missed every lens, the thinking included (15.5 to 16.7 ms a window against
12.6 to 13.3). The same field here holds every lens. So the miss was the box and the
lens's old rule, and the count of seats was never the limit at this size. By the factor
of two above, 100 seats at 100 units would be expected to hold on the live box as well
(the thinking about 11 ms a window against a lens of 25 there); **not verified**, since no
run may be made on that box.

### 4. Eight bubbles in space

Lane 2. Eight hulls, each flown by a player's commands from Ceres onto its own route's
picket waypoint, open eight customs-picket bubbles on one tick, each on its own domain,
each with its two cast minds (the corvette and the raider) thinking through MW.11's host
on the store's pool. The crews stay linked and give no order, so every bubble stays open
for as long as the lane runs. The host holds late steps, as the server's does.

| Pool | Ticks | The space tick on the world thread, p50 / p99 / worst | Thinks a tick | Held steps | Pool queue, peak |
|---|---|---|---|---|---|
| 1 worker, ticks back to back | 3 x 120 | 3.99 (4.58) / 4.61 (5.45) / 6.26 ms | 16 | 0 of 2,880 | 0 |
| **2 workers (shipped)**, back to back | 3 x 120 | 4.01 (4.02) / 4.56 (4.61) / 4.75 ms | 16 | 0 of 2,880 | 0 |
| 4 workers, back to back | 3 x 120 | 4.19 (4.42) / 6.10 (6.43) / 9.36 ms | 16 | 0 of 2,880 | 0 |
| 1 worker, a second apart | 2 x 120 | 6.87 (6.93) / 8.80 (9.12) / 10.92 ms | 16 | 0 of 1,920 | 0 |
| **2 workers (shipped)**, a second apart | 2 x 120 | 8.77 (8.82) / 13.08 (13.81) / 14.23 ms | 16 | 0 of 1,920 | 0 |
| 4 workers, a second apart | 2 x 120 | 6.75 (6.83) / 10.19 (11.19) / 18.05 ms | 16 | 0 of 1,920 | 0 |

The rows a second apart are the server's pacing. They read higher than the back-to-back
rows for the reason §2's real-time row does (a thread that sleeps comes back cold), and
they differ from each other by the box and not by the pool: the two-worker pair ran while
somebody else's build did.

**What the key buys in space: nothing at this size.** Sixteen thinks a tick, each 3 to 28
µs of a worker (MW.11's table), are done long before the next tick with one worker, with
the ticks a second apart or back to back. No step was held in any run and the pool's
queue was never seen above nought.

**The space tick costs the world thread 0.5 ms a second for each open bubble**, and it
is linear: 0.58 ms with one bubble open, 1.07 with two, 2.06 with four, 4.0 with eight
(one run of 120 ticks each, the same world otherwise). Over 900 ticks it does not grow
(p50 4.05 ms, p99 5.32). MW.11 measured what the minds cost that thread at 14 to 19 µs a
picket step and the bubble's own physics at 1 to 2, so the minds are about four per cent
of it. Most of it is `persist_space_state`, which on every tick clones the whole bubble
book, compares it with the copy it kept, and for every bubble that stepped clones it
again and encodes its whole record for the store. One bubble's record is 63.4 KB of JSON;
timed on the same thread outside the tick, one encoding of it is 0.30 ms and one clone
0.03. Two clones and an encoding are 0.36 of the 0.5 ms, before the comparison, and it is
also eight 63 KB documents written to
the store every second, half a megabyte a second while eight bubbles are open. It holds
(4 to 9 ms in a 1,000 ms tick), so nothing here misses; it is the place the time goes, and
the first thing to look at if more bubbles are wanted. Not fixed here.

Eight live domains also cost `sim.domain_pulse` 1.1 ms a second where it costs 0.003
idle, and the port tick is as it is idle (5.0 to 5.2 ms a fire, every five seconds).

**With the backup mind flying every hull** (`MW20_BACKUP=1`, two workers, a second apart,
five runs). Every crew's link is dropped and the grace waited out inside the bubbles, so
each bubble has a third mind. It heaves its hull to, customs rule, and all eight bubbles
close on the 33rd tick. Over those 32 ticks a bubble's step hands over 2.4 thinks, the
space tick is 7.36 ms at the median (7.50) and 8.91 at the 99th percentile (11.41), and
no step is held. A bubble whose carrier the backup mind flies costs the world thread what
one with an idle crew does.

**Eight is `mush_sim::MAX_ACTIVE_DOMAINS`.** A ninth interception queues. The lane ran
at the cap with nothing else live. Whatever else holds a live activation on a server
comes out of the same eight; which of a match or a field does was not checked.

### 5. A field and eight bubbles on one server

Lane 3: lane 2's world ticked a wall second apart on one thread while lane 1's
three-party field is stepped in real time on another, both on the store's one pool, 60
seconds a run.

| | The field's step, p50 / p99 / worst | Woke behind, p99 / worst | Batch wait p50 / p99 / worst | The space tick, p50 / p99 | The rest of a second's ticks, p50 / p99 | Held steps | Held |
|---|---|---|---|---|---|---|---|
| the field alone, real time (from §2) | 2.12 / 3.01 (3.13) / 3.88 ms | 0.16 / 2.4 ms | 59 / 240 / 371 µs | | | | 3 of 3 |
| both, native, 2 workers | 2.10 / 2.81 (3.78) / 3.90 ms | 0.15 / 0.65 ms | 34 / 272 (844) / 1,204 µs | 7.33 (7.65) / 9.12 (10.08) ms | 4.04 / 9.92 (11.49) ms | 0 of 1,440 | 3 of 3 |
| both, 100 Rhai seats, 2 workers | 2.07 / 3.06 (3.16) / 3.65 ms | 0.17 / 0.42 ms | 40 / 1,328 (1,362) / 1,497 µs | 7.24 (7.58) / 10.74 (16.34) ms | 4.13 / 9.58 (20.88) ms | 0 of 1,440 | 3 of 3 |
| both, 100 Rhai seats, 1 worker (2 runs) | 2.10 / 3.11 / 8.34 ms | 0.17 / 0.20 ms | 49 / 1,756 / 2,741 µs | 7.33 (7.42) / 9.39 (9.82) ms | 3.94 / 9.63 (9.75) ms | 0 of 960 | 2 of 2 |
| both, 100 Rhai seats, 4 workers (2 runs) | 2.12 / 3.12 / 3.36 ms | 0.37 / 0.75 ms | 39 / 997 / 1,533 µs | 7.45 (7.54) / 14.49 (17.37) ms | 4.22 / 13.57 (13.93) ms | 0 of 960 | 2 of 2 |

**Together they are what they are apart.** The field's step does not move. The bubbles'
thinks share the pool with the field's batches and are never late. The world thread, with
eight bubbles open and nothing else, spends about 11 ms of each second at the median
(7.3 on the space tick, 4 on everything else); at the 99th percentile the space tick is
9 to 17 ms and the rest of a second's ticks 10 to 21. The
space tick is 7.3 ms here and 4.0 back to back for the reason the field's step doubles:
a thread that sleeps a second comes back cold.

**What the `workers` key buys, with both running**: the field's batch of 100 Rhai thinks
is back in 1.8 ms at the 99th percentile with one worker, 1.3 with two and 1.0 with four
(alone, back to back: 2.4, 1.7 and 1.1). Every one of those is under one per cent of the
250 ms it has. One worker is enough at 100 units and eight bubbles on this box; two
leaves room for a second match, and four buys nothing that was measured.

### 6. What a full match's squads would spend at the LLM (Q-44-48)

No provider was called. The only thing in a match that calls the LLM is squad
deliberation, on the `tacsim_deliberate` route (`z-ai/glm-5.3-flash`, 512 output tokens,
in the deploy template): each machine squad asks for an order when it has none, when its
order expires or when its contacts change, and at most once in 30 arena seconds
(`[tacsim.arena].deliberate_min_ms`). The harness's cost run plays a match with every
squad's request made for real against `mush_llm::MockProvider`, and now sizes the
requests the mock was handed. Fifteen arena minutes is `[tacsim.match].max_duration_ms`.

| Match | Squads | Arena minutes played | Requests | Requests a squad-minute | Input tokens a request, p50 / max | Input tokens, all | With the 512 ceiling added to each |
|---|---|---|---|---|---|---|---|
| 6 a side, all machines (the most the shipped centre fields) | 4 | 3.0 (it ended by elimination) | 15 | 1.26 | 735 / 823 | 11,217 | 18,897 |
| 12 a side (the players' target), all machines | 6 | 15 | 75 | 0.83 | 722 / 868 | 55,621 | 94,021 |
| 50 a side | 26 | 15 | 226 | 0.58 | 738 / 871 | 170,501 | 286,213 |

**The assumptions.**

- *Tokens are characters over four*, over the system prompt, the squad's report and the
  order tool's description and schema: `mush-llm`'s own estimate, the one its budget's
  pre-check charges a request before it is sent. No tokenizer was run.
- *The output is not known.* The mock answers a scripted order of 48 tokens. The route's
  ceiling is 512, and the route asks for low reasoning, whose tokens count as output.
  The last column is the worst case; the truth is between the last two.
- *The mock's order runs out in ten seconds*, so a squad asks again as soon as the floor
  lets it: these counts are near the most a match can ask. A provider that gives longer
  orders asks less. A reply that does not parse is asked for once more, which the counts
  leave out.
- *Every unit is a machine in a squad.* Only machines are dealt into squads, and a squad
  under a pilot's live order does not ask.
- *The lane does not go through the world's door*, which lets eight of a match's requests
  be in flight at once, nor through `[llm.concurrency]`'s three background slots and
  queue of twelve. A match of 26 squads asks 26 times in its first pulse; on a server
  some of those are shed and asked again later. The count over a match is about the same;
  the opening burst is not.
- *One sample of each*, one seed. The fight is deterministic under the mock.

**Against the day's budget** (5,000,000 tokens and $5, 60 requests a minute, one bucket
for every route; the ledger settles what a call used and starts again at a restart; a
squad's commander is the call's actor, so the $1 a day an actor is allowed by default is
a commander's, and is never met at these counts):

| Match | Tokens settled, at 48 out to 512 out | Matches in a day's 5,000,000 | Requests a minute, mean |
|---|---|---|---|
| 6 a side | 11,937 to 18,897 | 264 to 418 | 5 |
| 12 a side, 15 minutes | 59,221 to 94,021 | 53 to 84 | 5 |
| 50 a side, 15 minutes | 181,349 to 286,213 | 17 to 27 | 15 |

**In money: not measured for the route's model.** The repo configures no price for
`z-ai/glm-5.3-flash`; OpenRouter reports each call's cost in its reply, and that is what
the ledger adds up. Two figures bound it, neither a quote. At the mock's placeholder of
$0.00008 a call the three matches are $0.001, $0.006 and $0.018. At the only rates the
template does configure, the Anthropic provider's $5 and $25 a million tokens, which
deliberation would pay only if its route were pointed there, they are $0.07 to $0.25,
$0.37 to $1.24 and $1.12 to $3.75.

**What it says for Q-44-48.** At the configured route the $5 is not what runs out; the
token budget is, and it is not close: a day of back-to-back twelve-a-side matches is 96
matches and the budget covers 53 to 84 of them, with the Registry, the NPCs and the
content routes on the same bucket. A server that runs a few matches an evening spends a
few per cent of the day. The provider's 60 requests a minute is nowhere near (5 to 15 a
minute a match). The default stands on these figures; the one thing the owner may want to
change is not the size but the shape: there is no `per_feature` line for
`tacsim_deliberate`, so nothing stops matches spending the tokens NPC dialogue needs.

### The floor: an idle fire of each module tick, in release

Lane 4: the stocked world at 24:1, nobody connected, nothing in flight, 600 seconds,
each due tick fired alone; three runs. 26 ticks are registered on this roster.

| | ms a fire, mean | ms a fire, p99 / worst | Fires in 600 s | ms a wall second |
|---|---|---|---|---|
| `tacsim.port` | 5.05 | 5.14 / 5.15 | 120 | 1.01 |
| `propagation.delivery` | 0.70 | 0.74 / 0.91 | 600 | 0.70 |
| `tacsim.surface` | 0.11 | 0.13 / 0.16 | 600 | 0.11 |
| `tacsim.field` | 0.016 | 0.03 / 0.07 | 600 | 0.02 |
| `tacsim.space` | 0.010 | 0.02 / 0.03 | 600 | 0.01 |
| the other 21 | under 0.04 each | | | 0.02 together |
| **all** | | | | **1.89 (1.94)** |

The rows are the first run's; the total is the three runs'. A whole second's fire, timed
as the engine fires it: 0.87 ms at the median, 6.8 at the 99th percentile, 11.3 at
worst. One second in five carries the port tick.

PR #576 measured the same in a test build: 44.6 ms a fire, 31.9 of it the port tick. In
release the port tick is 5.0 ms and a second's ticks are 1.9: six times and more than
twenty times less. A story's idle second is slow because of the build it runs in; the
server's is not. The port tick is still more than half of the idle world thread (#576
split it by arm: the clerks).

**This is tacsim's composition**, as its stories boot it. The server's own roster adds
the market's, the Roll's and the Registry's ticks, which this crate's tests cannot
compose and which were not measured.

### What misses

At the target, on this box: nothing. Past it, or on another box:

| What | Where | By how much |
|---|---|---|
| Rhai seats, 100 of them in a 300-unit match | conclav.dev | the thinking's lens by 15 per cent (33 at worst); the step holds |
| Rhai seats, 200 in a 300-unit match | conclav.dev | the thinking by 83 per cent, the pool's operations a window by 30 |
| Rhai seats, every hull of a 200-unit match | conclav.dev | the thinking by 14 per cent |
| Rhai seats, every hull of a 300-unit match | any box | the arena does not open: the snapshot's bound is 505.6 MB against 480 |
| The fill at 300 units with 72 Rhai seats, on the live box | conclav.io, 2026-10-09 (`match_/seal.rs`'s log), not re-run | the window by 12 per cent, the thinking by 23 to 26; holds here |
| A 100-unit match through `SIM START` | the shipped centre | cannot be started: six pods a side |
| A three-party field whose parties stand 31 cells apart | lane 1's first layout | never fights (below) |

### Found, not fixed

1. **Saving an open bubble costs the world thread more than stepping it** (§4): at least
   0.36 of each bubble's 0.5 ms a second is `persist_space_state` copying and encoding the
   whole record, and each open bubble is a 63 KB document written to the store every
   second. Nothing misses at eight. A dirty flag, or writing a bubble's record on change
   of what a restart needs and not on every step, is the cheap win. `space/tick.rs`,
   which MW.12 holds.
2. **The shipped simulator fields twelve.** `PODS_PER_SIDE` is 6 and
   `[tacsim.match].max_slots_per_side` is clamped to it, so a match is at most six a
   side, AI fill included. The combat-scale target of 100 units with 24 players has no
   path through the lobby; this unit measured the kernel at 100.
3. **Hulls with no order do not close.** Three parties 31 cells apart stood at `Alert`
   for 180 arena seconds without a shot; 18 cells apart they fight at once. The reading,
   not traced through the doctrine: a hull with no order holds its ground and fights what
   comes into its reach. In a match the squads' orders are what move them, and those come
   from the LLM; in an IC field nothing orders a machine but its player's posture. A
   field of minds fights only where its parties already stand in reach of each other,
   which is how the lane stands them.
4. **A field's snapshot is 2.2 to 2.5 MB every five seconds** (§2), and a match's is the
   same order. It is known (16b cut AF.2 measured the encode at 300 units); it is listed
   because nothing has measured what the world thread and the store spend on it.
5. **A bubble read back after a restart is held for one tick.** Its thinks are collected
   again and its step is asked for in the same tick, before a worker can have answered.
   Seen while building the lane (an earlier form of it that copied a bubble's record, not
   committed) and not reproduced through a restart; harmless, and by the rule MW.11
   wrote.
6. **`mushd.toml.example` still says** the pool is "process-wide … sized once per process
   by the first match". MW.11 made it the store's.

### For the owner

- **`[tacsim.minds].workers`: leave it at two.** Nothing measured needs more, on a box
  with cores to spare. On an 8-core box shared by two games it is two threads that are
  nearly always asleep.
- **The LLM's day budget (Q-44-48): the default stands.** See §6. Consider a
  `per_feature` budget for `tacsim_deliberate` if matches and NPCs are to share a day.
- **Rhai seats.** The fill is native because 72 seats missed on the live box in October.
  At the size the game fields (twelve) and at the target (100) the seats hold on this box
  with room, and the arithmetic says they would on the live box. If the owner wants the
  fill, or licensed minds in numbers, on Rhai, the evidence against it is one day's runs
  on a box that has since doubled its cores. That is a decision this unit does not make.
- **Space.** Eight bubbles cost the world thread 4 to 7 ms a second, nearly all of it
  saving. If space is to carry many more encounters than eight at once, item 1 above is
  first, and `MAX_ACTIVE_DOMAINS` is the cap that will be met before the CPU is.

### Not measured, and why

- **Anything on a live box or through a server**: the rule of this unit. Every figure is
  conclav.dev's, a kernel or a `TestWorld`.
- **The world thread's share of a 100-unit match or field**: only its inputs (ledger and
  snapshot sizes) and the fan-out lane's 24 pilots.
- **A match's squads against a real provider**: tests never call one. The token count is
  an estimate; the output and the price are bounds.
- **A field with seated players, or with players absent** (MW.7's gate, AP.6's posture):
  every hull is a mind with no order. The gate is a branch in the fire path and the
  narrowed view is smaller, so it is not expected to cost more; not measured.
- **Space with players' own minds** (MW.13, not built), **drones** (MW.12, in flight) and
  **the interdictor's bubble**: the picket's two cast minds, and the backup mind, are what
  was measured.
- **Class II and III seats.** The Rhai seats are Class I on Standard Doctrine, about 800
  budget units a think. A licensed mind may spend 4,500. The harness's ceiling variant
  measures four such hulls; a field of them was not run. Admission bounds their sum by
  the same pool.
- **The server roster's idle ticks** (the market, the Roll, the Registry).
- **More than three runs of anything**, and a quiet box.

### Defaults taken

| | Default | Cost to reverse |
|---|---|---|
| 1 | The field and the match are measured on the kernel, stepped by the lane on the task's policy, not through the task. The task prints no step time, and a hook in it was not wanted | Low: a lane |
| 2 | Eight bubbles are eight picket waypoints seeded into the shipped lane, flown to by commands. A test may not write the bubble book (doc 35d's contract), so a copied record was not an option | Low |
| 3 | The crews stay linked and idle, so a bubble stays open; the backup mind's bubbles are a second row | Low |
| 4 | "Held" for a field is GC.12's budget and the pool's lateness; for a match it is the harness's lenses as they stand | Low |
| 5 | The Rhai sizing variant is outside the weekly four, the baseline and the delta | Low |
| 6 | A token is four characters, as `mush-llm`'s budget has it | Low |

## MW.21

The completion story, with two accounts (doc 44 §7.1; first review, finding 9; second
review, row 18). **Tests only**: no code under `src/` moved, no persisted record, verb,
sentence, pin or ABI. One story,
`crates/tacsim/tests/domain_field/mw21_completion.rs`,
`mw21_a_bought_mind_flies_its_buyers_ship_and_her_first_command_wins`, on the combined M8
world (the production `LaunchComposition`).

**What the story walks.**

1. Two players, each a character on a gateway account of their own, each chartered by the
   Co-op (the charter opens a player's wallet, 25,000cr). Neither is staff: a staff verb
   typed by either answers `Permission denied.`
2. The author writes a script with the workshop's verbs (`@mind/new`, `@mind/line`,
   `@mind/save`), has the Registry rate it through its real trials (`@mind/rate`), files it
   (`@mind/file`, twice: the charter's shell, then the filing) and lists it sealed at the
   counter (`@mind/publish <filed> form=sealed`). The fee, 100cr, leaves the author's wallet.
3. The buyer leaves her ship (`+ship/leave`), reads the counter's list (`+minds`, `+minds
   <code>`), asks a quote (`+minds/buy`), pays it into escrow (`+minds/buy/pay`) and takes
   the licence (`+minds/buy/complete`). 400cr leave her wallet, 380cr reach the author's,
   and `+minds/holdings` reads her licence. She boards again (`+ship/board`).
4. She seals it (`+ship/mind filed:<her licence> go`): the Roll opens one claim at her
   berth. She files for Stickney (`+ship/undock stickney`) and her link is lost; past the
   grace a mind has her helm.
5. The server restarts with her on the corridor, the mind at her helm. At the Kirkwood
   picket the mind she bought flies her for ten thinks, with a second restart after the
   sixth. She reconnects with the eleventh on the host and types
   `+tac/lock off`. Six steps later she quits. The mind has her at once, heaves her to,
   stands her for customs, and she is cleared.
6. She makes Stickney with nobody aboard connected. The claim closes. Six fires, a restart
   and six fires later the Roll's book is the same. The buyer, back aboard, reads no mind
   sealed.

**The script** ("Warden") is written so that each thing the row asks is visible on the
bubble's tape. Under the hail it locks the ship that hails her. Its first think asks a burn
that would run the picket, with all her drive gives; from its second it brakes, shedding her
drift over 24 equal burns it works out from her drift and keeps in its memory (the house
default brakes with everything at once). Every think it asks a shot of each of her lasers.
Hove to, it cuts her drive, which is the seat's word to submit her to customs. A cold hull
it leaves cold, so the Registry's trials rate it Class I in 145 s.

**The eight clauses, and the assertion that holds each.**

| | The row asks | Held by |
|---|---|---|
| 1 | Two accounts, neither staff, by registered commands | `excursions::two_accounts` (two entries of the rig's account oracle, not equal); both connected; `@econ/mint` refused both; every submission a line stages is asserted to be its typist's and is landed on the composed path (`types`). The filing row's `requester` is the author; the author's wallet reads the fee less, then the price less the Registry's 5% more; the buyer's reads the price less; the one `roll_mind_personal_paid_licence` row whose `holder` is the buyer is not the owner's licence, and her `+minds/holdings` line names it |
| 2 | She fits it and takes her out | `ship_mind::aboard`: dispatcher and principal the buyer, the mind `warden`, the licence her own, Class I over a governed Class II core, helm and sensors only. The Roll's `roll_mind_ic_claim` family holds one row for her hull, open, whose `open.operation` is the seal's claim, `open.dispatcher` the buyer and `open.licence` hers |
| 3 | A choice the house default would not make | Two differences, both read off the bubble's tape. The story first flies **the same passage with nothing sealed** (`house_default_at_the_picket`: the same world, hull, core, berth, filed passage and link loss): the house default writes one burn of 820 mm/s², has her hove to in one step, locks nothing in any reading, traces only `BACKUP_` words and is cleared in three steps. The bought mind's first think writes `SetLockedTarget` on the corvette and no burn; its second writes one burn of 70 mm/s², which is what the script's arithmetic gives her 11.2 m/s of drift, and an eighth or less of the house default's; and at the step where the house default had her hove to she is still under the hail |
| 4 | The absent-player limits | From the readings of the whole bubble, in order (`Watch::log`, 60-odd readings, each holding what the tape gained that names her ship): every entry is the lock on the corvette, a `CommandBurnMmps2`, or (while her crew has her) her own lock-off; no shot, no decoy, no rock worked. The run the first think asks is not written and her drift is unchanged after it (twice: each time a mind takes her). Between two readings with a mind at her helm her drift never grows and her reaction mass never rises. She never reads `RanThePicket`. Her posture stays STANDARD. Mining and trading: no row of the market moved (clause 5) |
| 5 | Only the ship's own stores | `Books`, read when the mind takes her, at the reconnect and at her berth in Stickney: **every account in the world** has the same posted balance at all three (the author's and the buyer's are asserted again by name); the only typed rows that moved are under `tacsim/` and `inventory/`, so nothing of the market, the Roll or the economy; at most one inventory pool moved; her hold's provisions are as they were and her reaction mass is less. The buyer's licence row is byte for byte as it was |
| 6 | A restart while it has her helm | Twice. **On the corridor before the picket** (main's fix of that leg, doc 35's "A crew that is away meets the picket", merged while this was built; no story flew it with a sealed mind): `aboard` is equal, the mind still has her helm with her buyer not connected, the Roll's claim rows are equal, and the picket's bubble opens on her at the end of the leg. **Inside the bubble, six thinks in**: `aboard` is equal to the seal read at her berth (class, principal, claim, digests) before and after; the Roll's claim rows are equal; the bubble reads back with the same step, lock, thrust, drift, status, think count, trace word and reaction mass, the mind at her helm, and no think in flight. The first step after it lands think seven: `WARDEN_HEAVE_TO` with the same thrust and nothing new on the tape. A mind that had lost its memory would have traced `WARDEN_RUN` |
| 7 | She reconnects with its next action pending | Before: ten thinks landed, the brake on, the lock standing, `ThinkHost::in_flight` true for the bubble's batch at the step its record says it owes, and that step handed the host as many thinks as every step before it. At the reconnect, before any step: the helm is hers, the thrust is nought, the tape gained nothing, she hears the hand-back line. Her first command: `Target lock cleared.`, the tape gains exactly `SetLockedTarget { None }`, and `tacsim.space_bubble_tape` holds one row. Then six steps, each asserted: nothing on the tape for her, no lock, no thrust, her drift to the millimetre a second what it was, and fewer thinks handed to the host than a step with her mind took. **That the struck think would have acted** is shown in place: she quits, the mind has her on the instant, and the same script's first think in the same position writes the lock on the corvette again and its second a burn. **The blink** (see the mutation checks): back and gone inside one step, and the think that was on the host when she came back does not land in the next seat |
| 8 | The claim closes exactly once when she docks | Docked at Stickney with her buyer not connected. Until the Close lands the book's row is the open row. After: one row for her hull, its `open` part unchanged, `closed.operation` the token `ic_claim_closed` answers, `closed.kind` `returned_to_base` at Stickney's host. After six fires, a restart and six more: the rows are equal, the same close token, no active claim, nothing aboard, the cursor resting on the closed claim. `+ship/mind` reads `MIND_NONE` |

**What staff do** (the story's header says the same).

| | Fiat | Why |
|---|---|---|
| 1 | Staff bind her hull to `tacsim.unit.dray_core_ii` (`ic_sortie_surface::fit`) | No ship carries a core and no route in play fits one (MW.13, "Not built"). **The one the gap forces** |
| 2 | Staff carry the author to the counter, and the buyer there and back to the berth gallery (`@teleport`) | The M8 rig's counter is a room staff made, with no door. A rig gap, not the game's. The buyer leaves and boards her ship by her own verbs (a staff teleport of a character aboard a ship is refused) |
| 3 | Her tank and locker are stowed by staff's hand (`excursions::docked_dray`) | Every ship story's rig. A pilot buys them at the chandlery; `cost_lane.rs` flies that |

Nothing is minted. MW.5's and MW.13's stories top their pilot's wallet up for the filing
fee (`c4_filed_mind_field::file_own_mind`); this story's author files from the wallet the
charter opened, so that top-up is not needed there either. The story has its own copy of
that helper for a named author (`writes_rates_and_files`), because the helper's file
is held by another session (the fix of MW.5's flaky field story).

The story itself drops and raises connections (a lost link, a reconnect, a clean QUIT),
restarts the server over its kept store three times (`M8World::reboot_reconnecting`), and moves the
clock: past the link-loss grace in one jump, to the instant each leg of her passage ends,
and a second at a time inside the bubble. No wall-clock window is waited out.

**Run time and tier.** 166 to 168 s on conclav.dev (Linux, unoptimized test build), of
which the rating is 145 s, the house default's flight 7 s and everything after the filing
about 7 s. Tier: nightly, in the `m8-gate-stories` job with the other one-rating stories
(`test-tiers.toml`; `ci.yml`). It needs no pulse knob and no checkpoint: the M8 rig pumps
the Registry as fast as it lands.

**Repeats**, all on conclav.dev, each a batch run at once:

| Text | Base | Batch | Passed | Wall |
|---|---|---|---|---|
| before the blink | `4efa11521` | four | 4 of 4 | 167 to 168 s |
| before the blink | `4efa11521` | four, beside the six mutation runs | 4 of 4 | 170 to 173 s |
| with the blink | `4efa11521` | four, beside one mutation run | 4 of 4 | 167 to 168 s |
| with the blink | `4efa11521` | twelve | 12 of 12 | 167 to 171 s |
| final: the corridor restart, main `1c96e57c1` merged | `1c96e57c1` | four | 4 of 4 | 166 to 168 s |

No run failed. The box's load average was 2 to 15 through them, so "under load" here is up
to twelve copies of the story at once and not a saturated box.

**The CI dispatch.** At the first text (run 43510830838, `430a505b8`, before main moved):
the story's job, "M8 MW.21 a bought mind flies its buyer's ship", passed on the hosted
runner, 250 s for the story (the rating 217 s) and 11 min 47 s with its build. Every
pull-request job of that run passed but one cell, `test (rest-server)`, red on mush-server's
`player_market_commands::rate_queues_and_cancels_a_genuine_registry_preparation` (150 s, no
assertion text in the log). That is the test MW.19's note records red once in its own
dispatch: a story of the Registry's service, in a crate this unit does not touch. The run
was cancelled with its long weekly jobs still going when main moved under it. The dispatch
of the final head is named in the pull request.

**Mutation checks** (conclav.dev, in the throwaway clone; each applied alone to the merged
code, the story run, the edit dropped). M2 to M6 were run on the text before the blink, over
`4efa11521`; nothing they touch, in the code or the story, has moved since. M1 and M7 were
run on the final text over `1c96e57c1`.

| | Mutation | The story |
|---|---|---|
| M1 | The hand-back does not strike the think in flight (`tick.rs`, `ThinkHost::forget`) | **fails** at the blink: the think collected for the seat she took back lands in the next seat (`WARDEN_RUN`, the lock, think one) where nothing should |
| M2 | A sealed mind's seat may fire (`backup.rs`, `land_in_bubble`) | **fails** at its first think: the tape gains `FireOffensiveLasers` at the corvette |
| M3 | A sealed mind's burns are free | **fails** at its first think: a burn of 820 mm/s² away from the corvette, her drift 12.84 m/s |
| M4 | The seat does not take the sealed mind as a source (`backup.rs`, `sources`) | **fails**: `BACKUP_HEAVE_TO`, and no lock |
| M5 | The hand-back does not cut the mind's burn | **fails** at the reconnect: thrust -70 with the helm hers |
| M6 | A crew that quits is not handed over at once | **fails**: `Grace` where the mind should have her |
| M7 | A hull whose crew is away does not meet the picket (`sim_routes.rs`: main's fix of the restart on an interdicted leg, undone) | **fails** after the restart on the corridor: "the Kirkwood picket's bubble opens on her" |

M1 **passed** the story's first draft, which had the reconnect, the six quiet steps and
the quit, and no blink. The pending think is kept from landing twice in the code: the
hand-back strikes it from the bubble's batch, and the landing drops a backup mind's answer
when no link-loss record with the helm stands on the hull (`tick.rs`, `land_bubble_think`),
or when the record's think number is not the answer's. With the strike removed the second
keeps her. The strike alone is what holds only when a new seat with the same think number
stands on her by the next step, which is a link that comes back and drops again inside one
step. The story now flies that: after the quit and one step, with the new seat's first
think on the host, she connects and quits again; the next step lands nothing, and the step
after lands a think collected after she left.

**Found, not fixed.** None is a defect in a merged unit that the story had to work round.

- **A burn of three kilograms costs her hold a tonne.** In the bubble her reaction mass
  goes from 9,000 kg to 8,997 kg over both of the mind's tenures; her hold's pool, which is
  in whole tonnes, reads 9 before the picket and 8 by the time she reconnects. The story asserts "less", not
  a figure. By reading the numbers, not the code; not investigated.
- **`+ship/mind` on a sealed hull answers one sentence of 154 columns** (`MIND_ABOARD`, for
  this ship and licence). The story reads the seal from the claim book and does not assert
  that line; every client line it does assert is checked under 120 columns (`fits`).
- **A player needs staff to reach the M8 rig's counter.** The rig's, as above.

**Not in it.**

- **A second passage and a second dock.** The only passage out of Stickney is back through
  the picket. "Closes nothing more" is held by six fires, a restart and six more fires.
- **The mind outside a bubble.** She is on the corridor she filed from the link loss to her
  berth, so the seat's two acts adrift (make for port, hold) are not asked of this mind.
  MW.8's stories fly them for the house default.
- **A rule program.** The script binds no rule row; MW.13's
  `mw13_two_sealed_minds_of_one_script_differ_by_their_rule_programs` holds that.
- **The story over SSH**, which waits on the space journey over SSH (doc 44 §7.1, "not
  units").
- **A shot that would land.** A sealed ship mind has no fire channel, so "never first" is
  "never" here. Mutation M2 shows the story would see a shot if the seat let one through.

**Test support that moved** (additive): in `domain_field/mw13_ship_mind.rs` eight helpers
became `pub(crate)`; in `domain_field/excursions.rs` `pilot`, `hold_vault` and `pool` became
`pub(crate)` and `two_accounts` is new.

## MW.22

Doc 23's cut S.4c: lease and organisation authority in the title fence, in the Roll and at the
dispatch (doc 44 §7.1, Q-44-39; second review, §4c row 2). The design note is doc 23 §18
(D-23-24), written first.

**Built.**

- **One rule, in the Roll.** `mush_roll::commerce::ic_claims::hull_authority(world, hull, actor)`
  (`crates/mush-roll/src/commerce/ic_claims/hull.rs`) answers by which of three arms a character
  may open a claim on a hull, or why not. The title must be live. Then: the character's own
  unscoped **title**; a **lease**, read from the economy alone (the title carries its holder's
  `Lease` claim, and that contract is Active, lease-shaped, issued by the holder, pledges the
  hull and names the character a party); or an **office** (the hull is an organisation's and
  the character holds a live `roll.holder.org` grant on the title's scope, asked as a read).
- **The Roll's reader captures it.** `TitleFence` (`ic_claims/reader.rs`) keeps the arm with the
  title's revision and re-reads both at the fresh check, so a lease that ended or a grant taken
  back between the read and the commit is stale. The reader's definition says
  `"hull":"current_title_lease_or_office"`.
- **The dispatch calls the same function.** `untitled_refusal` (`surface/dispatch.rs`) refuses at
  `go` exactly what the Open would refuse for want of authority, and names the hull and its
  title's holder: "{mark} is {owner}'s hull; a licensed mind flies a hull titled to you, leased
  to you, or held by an organisation you act for." The dispatch's own copy of the rule
  (`titled_to`) is deleted. The bot's pinned phrase is still inside the sentence.
- **An organisation's owner licence opens a claim** (doc 23 Q-23-19's path, which a House's
  licence on any hull needs). The reader's fence over an owner licence's charter accepted only
  the dispatcher's personal title; it now accepts the arm the owner projection proved, a House's
  charter by the acting character's office. The claim history's audit (`ic_claims/audit.rs`)
  accepted only the charter's historical owner as dispatcher; for a House birth it accepts any
  dispatcher, as the paid arm already does for an organisation's licence.
- **The kernel admits an owner licence's term on a claim.** Found on the CI dispatch: a House's
  own licence passed the reader and all four owners and was refused at the last step,
  "IC captured term bounds". `mush-core`'s audit of a claim row
  (`action_state/terminal_capacity/ic_claim.rs`) capped every claim's captured term at 90 IC
  days; a House's owner licence runs 365 (17g C0 raised the Roll's owner bound, and nothing
  could open an owner licence's claim that long until now). The bound now follows the claim's
  kind: 365 days for a claim dispatched under an owner licence, 90 for a paid seat. The two
  bounds are constants in the kernel (`mush_core::licence_term`) and the Roll's
  `MAX_TERM_SECONDS` and `MAX_OWNER_TERM_SECONDS` are those values.
- **The reader's owner arm says why it refuses**, at debug on the `ic_claim` target, as its paid
  arm did; doc 17h's tier-2 and zero-PC stories print that target.

**Tests.**

- `mush-roll`, six tests in `ic_claims::hull` (`s4c_*`): each arm, the order they are tried in,
  the refusal naming the holder, a title that is not live admitting nobody, a claim that is not
  the titled lessor's, a grant on another scope.
- `mush-core`, `the_captured_term_is_bounded_by_the_claims_kind`: an owner licence's claim with
  a 365-day term parses and a 366-day one does not; a paid seat's with 90 days parses and 91
  does not.
- `tacsim`, in source: `s4b1_a_licensed_mind_needs_the_hull_titled_where_claims_open`
  (`surface/dispatch/tests.rs`), rewritten: nothing is read without the claim path.
- `tacsim`, fast, on the production composition:
  `mw22_the_hull_rule_reads_a_real_lease_a_house_title_and_a_personal_title`
  (`domain_field::m8_world`) asks the rule of the M8 world's own rig as `+tv/lease go` left it
  (the lessee admitted by the lease, a stranger refused naming Occator), then titled to a House
  (refused naming the House; admitted by the office once granted), then titled to the pilot.
- The acceptance story, `#[ignore]`d, on the CI dispatch:
  `s4b2_a_licensed_rig_opens_its_claim_in_the_bay_and_closes_it_home`, rewritten. Through
  registered commands, with a non-GOD pilot: (1) a chartered pilot launches her paid Pin on her
  leased rig, the claim opening in the bay and the open line carrying the claim's seal; (2) the
  one seat refuses the second leased rig; (3) with the Pin's resale accepted while that claim
  is open, the second rig is refused by the pending sentence; (4) the recover lands the Close;
  (5) on a rig titled to a House she holds no office with, she is refused by the sentence,
  naming the rig; (6) on a rig titled to the Coprates House she flies the House's own owner
  licence, the House the holder and she the acting principal, the server restarts with that
  claim open, and it closes at home.
- Doc 17h's stories with the claim path composed, on the CI dispatch:
  `m8g_story_tier2_held_minds_seal_aboard`,
  `m8g_story_zero_pc_procurement_buys_the_best_two_under_the_cap` and
  `m8g_story_the_combined_world_rates_files_lists_and_sells`.

**Results on the CI dispatch (2026-11-12).** Three defects came out of these runs and are
fixed: the kernel's term bound (above), the claim arm's tickets (MW.0, above) and two of the
rewritten story's own assertions. On `def8832c` (run 43125566722, the term bound fixed):
`s4b2_a_licensed_rig_opens_its_claim_in_the_bay_and_closes_it_home` passed all six steps;
`s4b3_a_licensed_mind_flies_a_field_at_256` and `_at_512` passed; both `d23_18_*` stories
passed; `m8g_story_the_combined_world_rates_files_lists_and_sells` and
`m8g_story_zero_pc_procurement_buys_the_best_two_under_the_cap` passed; the leak battery
passed; the M7 stories that had finished passed.
`m8g_story_tier2_held_minds_seal_aboard` failed there at its second claim's Close, which is
the claim arm's defect. The arm's fix followed in its own pull request, which records the story's run over it.

**Divergences.**

- **A change in `mush-core`, outside the row's files.** The term bound above is the kernel's. It
  is one bound in one audit, with its own test, but it is the engine's and not the Roll's or
  the dispatch's. Without it a House's own licence cannot open a claim on any hull, and doc
  17h's tier-2 story and the leak battery fail with the claim path composed.
- **The lease is read from the economy's contract, not from `TvLease`.** Doc 23 Draft 0.10
  sketched a `LeaseFence { lease, lessor, lessee, term_ends_ic }` captured from TacSim's rig
  lease record. The Roll cannot read a TacSim record, and the economy already holds the lease as
  a contract and a claim on the title, so the rule reads those. Doc 23 §18.2 records it.
- **The claim row does not record the arm.** Draft 0.10 has the lease "audited like the title's
  revision". The row keeps the title's revision, as before, and nothing else new: the row's wire
  is mirrored field for field in `mush-core`'s terminal-capacity protocol, and M8c S2 already
  set the precedent that the row records the dispatcher and not the grant.
- **The fence is asked as a read.** `roll.holder.org` is amount-bearing; flying a hull moves no
  money, so a grant with a spending limit admits (M8f's P2a ruling). An organisation's licence
  itself is still asked the way M8c built it.
- **A House is "an organization" in the refusal.** The sentence names the title's holder with
  the port's own helper (`port::claims::holder_name`), which names a character and a lane
  organisation (Occator) and calls any other organisation "an organization". So a pilot refused
  on a House's hull reads "OWP-TV-01 is an organization's hull; ...": the rig is named, the
  House is not. Naming it means deciding which organisations a player may be told the name of.

**Defaults that would be a significant change to undo.**

- **Recording the arm on the claim.** If an inspection or a dispute ever has to say on what
  authority a hull was flown, the claim row, the admission digest and `mush-core`'s mirror of
  the row all change, and every stored claim with them.
- **A forfeit rig lease still admits.** The Roll reads the economy's contract, which stays
  Active after Occator captures the deposit; the launch verb does not refuse a forfeit rig
  either. Refusing a licensed mind there means a TacSim fact in the Roll's rule, or a second
  rule at the dispatch that the Open does not share.

**Not built.** Hull cover still sells only over a hull titled to its dispatcher
(`coverage/reader.rs`); no production launch composes it. The Trust's charter of a ship has the
lease arm's shape but no ship opens a claim (D-23-12). No verb lets a House name which officers
may fly which hull: the grant is the organisation-wide one its licences use.

**Not pinned by a story.**

- A lease that ends, or a grant taken back, **while the claim is open**: the design says the
  claim stays open until the epoch closes; no story ends a lease mid-claim. No in-game verb
  takes a `roll.holder.org` grant back.
- A rig repossessed where it lies on a site with its claim open: not traced (doc 23 §18.4).
- The reader's stale fence (authority lost between the read and the commit) end to end; the
  unit tests pin the rule, not the race.
- A character-held owner licence flown on a leased rig, and an organisation's paid Pin on an
  organisation's hull: the story flies a personal paid Pin on a lease and a House owner licence
  on a House hull; the zero-PC story flies a House's paid Pin on a leased rig.

## MW.23

The Class III end-user entitlement issued by a launch route (doc 44 §7.1; Q-44-40, ruled at its
default; second review, §4c row 3). Q-44-12 (no heirloom exemption) is ruled and is pinned here
by a story.

**Built.**

- **One rule for the entitlement.** `mush_economy::core_integration::end_user::entitlement`
  (`crates/mush-economy/src/core_integration/end_user.rs`) is what the integration's acceptance
  and completion (`authority.rs`) and the delivery (`delivery.rs`) now ask, in place of two
  copies of the same test. The test is unchanged: the instrument
  `tacsim.licence.core3_end_user` is registered as an active, indivisible, untransferable
  entitlement, and the requester holds an `Active` unscoped account in it with a balance. A
  requester without it is refused exactly as before ("Core manufacture is unavailable or
  changed."). `CORE3_ENTITLEMENT` is now a public constant.
- **The registration, in the economy.** `register_end_user(ctx, &EndUserRegistration)` posts
  **one transaction** with four lines: the fee from the holder's own credit account to the
  Registry's account, and one unit of the entitlement from the issuer's source account to the
  holder's. The fee and the right are one journal entry, so a restart keeps both or neither.
  The issuer must be a registered title issuer (`EconomyModule::with_title_issuers`, the
  wiring-frozen table `issue_title` asks). The post runs under an authority that admits
  exactly those four lines, by the holder (`issue_title`'s pattern), because the live org
  bridge refuses any debit of a system account that staff did not type. The first
  registration on a world registers the instrument and opens the issuer's source account;
  each holder's first opens their own account. `check_end_user_registration` makes every
  check and writes nothing; `end_user_standing` reads `Unlisted`, `Registered` or `Suspended`.
- **Once.** A registered holder is refused (`EndUserError::Registered`) with nothing written.
  The idempotency key is the holder and their entitlement account's journal sequence, so two
  calls over one state resolve to one transaction, and a registration staff later reverse can
  be made again.
- **The route, in the launch.** `+enduser` (`crates/tacsim/src/launch/end_user.rs`, registered
  in `commands.rs` beside `+licence`). `+enduser` and `+enduser/show` read the actor's
  standing; `+enduser/apply` previews; `+enduser/apply go` registers. The fee is 2,500cr
  (`END_USER_FEE_MINOR`), paid into `LaunchDeployment::registry`'s account, where the filing
  fee and the levy go; the issuer is the deployment's `registry_issuer`, the principal that
  already issues the launch's charter and core titles. It answers "The Registry takes no
  end-user registrations on this world." with no launch or on an `Initialize` boot. It works
  on a format-1 launch as on a format-2 one.
- **Nothing in the launch file, the bring-up or `deploy/`.** No format bump, no new key, no
  new account for an operator to open, no new grant. `launch.json.example`,
  `launch-bringup.py` and the test rigs are untouched.
- **Help.** `docs/help/hardware.md` has the registration's section, and the sentence that said
  only that Class III "requires the buyer's actual receiving entitlement" now says how to get
  it.

**The route chosen, and why.** Every right a launch issues to an end user is the end user's
own paid act, with the Registry's fee landing in the launch's Registry account and no clerk
or staff on the path: a filing (the fee, then the owner licence), a House Pin and a personal
Pin (quote, fund, grant). The game's one other `Entitlement` instrument, the prospecting
licence, is issued the same way by `+licence/buy go`. Class III follows that kind: a paid
registration with the Registry. It needs no person at a desk, so it works on a dev box nobody
is watching, and it needs nothing from the bring-up.

**Rejected.**

- *A Registry operator's grant* (`@registry/enduser <character>` under a new
  `roll.registry.*` capability). It puts a person on the path of every Class III core, adds
  an eighth grant to the runbook and the bring-up script (a `deploy/` change while MW.1's PR
  is open), and would be the only right in a launch issued at an operator's discretion; the
  operators' verbs recall, revoke, inspect, fine and seize, they do not issue.
- *A typed purchase* (quote, fund, grant as an economy owner participant, like the Pins). It
  is the right shape if the registration ever has a term, seats or a resale, and it is the
  future family named in `legacy-handlers.toml`. It is several thousand lines of protocol for
  a right that today is one account balance read by a legacy-lane snapshot, and nothing in
  Q-44-40 needs it.
- *Naming entitled characters in the launch file.* A format bump, and not a route: a player
  who arrives later could never have a Class III core.

**`ungovern`.** Delivering a Class III core in a launch world needs only the entitlement. The
importer's certified Class III die is integrated at the chartered factory as a governed core;
the ungoverning shop is for the refit that takes a governor off and for heirloom and grey
dies. So nothing was composed for it. The second review's "the example launch has no
`ungovern`" is true of `deploy/instance/launch.json.example` and stays true; the bring-up
script (MW.1) already stands up the Tether back room on the dev launch and names it in the
file it writes, and the M8 test world composes one too.

**Tests.**

- `mush-economy`, `core_integration::end_user::tests` (five, fast): the fee and the unit are
  one transaction, the preview writes nothing, the instrument does not exist before the first
  registration, a second registration is refused with nothing paid or issued; every refusal
  is whole (another character's wallet, an unregistered issuer, a short wallet, a zero fee,
  the payer as its own Registry) and registers nothing; a frozen registration entitles nobody
  and is not issued again; an account with nothing in it entitles nobody and is the one a
  registration fills; a restart from the folded store keeps the registration and still
  issues it once. Four mutations of the module were each caught by a test of their own: the
  rule not asking for a balance, a registered holder not refused, an unregistered issuer
  admitted, a frozen account not refused.
- `tacsim`, in source (two, fast): the help's figure is the fee the verb charges and a tenth
  of the Class III core's price; the route issues the instrument doc 17 names.
- `tacsim`, `domain_field::m8_end_user` (three, fast, on the production composition, the M8
  world booted plain, `Initialize`, `Active` as the bring-up boots it):
  - `mw23_a_registered_end_user_has_a_class_three_core_built_and_delivered`, the acceptance.
    The world has no entitlement instrument. The pilot is told they are not listed; the stock
    agent issues a certified Class III die and offers its integration, and the pilot's
    acceptance is refused with the uniform refusal and nothing charged. `+enduser/apply`
    previews and writes nothing. `+enduser/apply go` takes 2,500cr into the launch's Registry
    account and leaves the pilot one unit and the issuer's source at minus one. A second
    `go`, and a second preview, answer the standing and move nothing. The world restarts
    from its store and the registration stands. The same integration is then accepted,
    worked its 96 hours, completed and delivered into the pilot's hands for 25,000cr, the
    delivery's facts hold, and the title's birth reads Class III, certified. Another
    character with a full wallet is still refused.
  - `mw23_a_heirloom_in_the_back_room_is_under_the_entitlement` (Q-44-12): the pilot's own
    heirloom die is refused at the back room until they register; then the same offer is
    accepted and the heirloom is delivered as a Class III core whose birth reads heirloom.
    This closes doc 17h's "blocked: the heirloom over the launch world".
  - `mw23_the_route_exists_only_on_an_active_launch`: a world with no launch answers the
    not-open line to all three forms and issues nothing.
- **Staff in the stories.** Staff commands mint credits into a wallet and open a second
  character's wallet. None issues the entitlement, the core or the certified die. The
  heirloom story's die is issued by staff, which is how doc 17h seeds heirlooms (its §4 defers
  the back room as a vendor). The stories write no module state by hand; the fiat budget is
  unchanged.

**Results (2026-11-12).**

- **Local (Windows):** the five `mush-economy` unit tests pass; `cargo check -p tacsim --tests`,
  clippy on `mush-economy` and the rustdoc check on `mush-economy` and `tacsim` are clean.
- **conclav.dev (Linux, a throwaway clone of main `330a8b249` with this branch applied):** the
  three `m8_end_user` stories pass (the acceptance in 16 s). `tacsim`'s 46 in-source tests of
  the hardware flow and the launch pass, among them
  `shipped_class_three_requires_actual_entitlement_and_never_charges_on_refusal` and the
  heirloom and grey-die tests, which run the shared rule. `cargo fmt --check`, clippy with
  `-D warnings` over all targets of `mush-economy` and `tacsim`, and the M7 memory-size lints
  are clean. `cargo nextest run -p mush-contract-tests`: 192 passed, 8 skipped, with no budget
  or census re-blessed; the only register that moved is `legacy-handlers.toml`'s new stanza.
- **Mutation checks, on the stories** (a second clone): with the integration no longer asking
  for the entitlement, the acceptance and the heirloom story fail at "refused"; with a
  registered holder no longer refused, the acceptance fails at "once"; with the registration
  issuing no unit, both fail. The unmutated tree passes in the same clone.
- **CI, PR #553:** green on the first push (`0c8e518cd`) and on the code's last commit
  (`14ea5234b`): 28 checks passed each time, the 15 dispatch-only jobs skipped.
- **The three dispatch-only claim stories PR #546 repaired**, run on conclav.dev with
  `--run-ignored only` on this branch, since they share the launch's composition:
  - `s4b2_a_licensed_rig_opens_its_claim_in_the_bay_and_closes_it_home`: passed (639 s).
  - `g4_the_attestation_channel_learns_an_ungoverned_flight_of_a_filed_mind`: passed
    (1,789 s).
  - `m8g_story_tier2_held_minds_seal_aboard`: **failed** (1,782 s), "close_ic_mind by #808
    did not land: that action is not currently offered". It is not this unit's. The same
    story run on main `330a8b249` with none of this branch fails with the same line
    (1,790 s), and run on this branch over PR #554's head `7294d9e9d` it passes (1,788 s).
    PR #554 (the MW.0 follow-up: a rig's second claim in one boot never closes) is the fix,
    and this branch merges with it without conflict.

**What the survey and the brief had wrong.**

- The entitlement was said to be provisioned "in the rigs under `m8_world.rs`". It was not:
  only `tacsim`'s hardware flow rig and `mush-industry`'s delivery fixture issued it, and the
  M8 world had no Class III core in it at all (its grey-die story says so).
- `mush-server`'s `launch_world` rig builds the launch's identities by production commands,
  but it has never driven the hardware: it checks only that `issue-core-die` is offered. So
  the acceptance is in the M8 world, which runs `LaunchComposition::prepare` (the call
  `mush_server::boot` makes) and has the hardware driver.
- Doc 17 §16 says the entitlement is "checked at transaction time the way
  `extraction_authorized` checks an extraction licence". It is stricter: the extraction
  licence asks for an `Active` account, this asks for a balance as well. That is why the
  route has to post a unit and cannot only open an account, as `+licence/buy` does.
- 17f M7.11 says "an org grant is scoped to the org". Nothing checks an organisation's
  entitlement: the rule reads the requester's personal, unscoped account, and an integration
  is always ordered by a character.

**Defaults taken.**

- **The fee is 2,500cr**: a tenth of the Class III core's 25,000cr, and the prospecting
  licence's fee. Provisional.
- **No term.** The registration does not lapse. The entitlement is an account balance the
  economy reads at the act; a lapse needs a record of the term and a gate at the acceptance.
- **One registration a character**, not one a login account: the economy's rule is about the
  requesting character.
- **The fee is not in the `launch_content` definition**, so no definition or digest of an
  existing world moves and a kept store boots as before.
- **The registration is not chronicled.** It is one private journal entry. The prospecting
  licence's fee is a Public contract the canon bridge records; this is not, because a
  contract's settlement cannot carry the entitlement's unit in the same transaction.
- **A principal the Registry has barred** (`@registry/revoke principal=`, 90 IC days) can
  still register. The bar is a Roll row and the verb is a legacy-lane command with no read of
  it.
- **No verb suspends a registration.** `Suspended` is read when the entitlement account is
  not `Active`; today only a boot's drift freeze or a closed account produces that. Staff can
  take a registration back by reversing its transaction (`@econ/reverse`), which also returns
  the fee; that path is not pinned by a test.

**What would be a significant change to undo.**

- **A second power for a registered title issuer.** `register_end_user` is a public function
  of `mush-economy` that debits a system account without staff, under its own injected
  authority. It is bounded (one unit of this one instrument, to the paying character, with
  the fee in the same transaction), but it is a new way value is created, and removing it
  means another route.
- **A legacy-lane verb that decides action legality**, listed in `legacy-handlers.toml` as
  `legacy-post-image` with `+licence` as its precedent. Moving the registration to a typed
  purchase replaces the verb's write path, the idempotency key and the stanza, and the
  entitlement check would have to stop reading the legacy snapshot's accounts first.
- **Adding a term later** changes what a registration is: every registered character would
  hold a right with no end date, and a term needs a migration rule for them.

**Not built, and for whom.**

- **C4's story 4** (MW.5): the hardware story through the launch's governed hardware with a
  refit and a restart. What it needs is reachable: after `+enduser/apply go`,
  `M8World::integrate(requester, payer, DieClass::III)` builds and delivers the core and
  `M8World::refit` fits it; `m8_end_user.rs` has the steps.
- **A fitted Class III on a hull** is not in these stories (they end at delivery). MW.5.
- **A typed action for the web and the TUI, and an offer in `+actions`.** `+enduser` is line
  mode only. Deferred: no unit owns client surfaces for it; MW.5 should take it if story 4
  is to be driven from a client.
- **The bot.** `mush-bot` has no sentence for `+enduser`. MW.19 (the bot's minds lane) if a bot is
  to buy a Class III core.
- **An organisation's entitlement** (17f M7.11). Deferred: no integration is ordered by an
  organisation.
- **`Legality::Restricted`** on the catalogue entry (doc 17h §4). Deferred; nothing reads
  it.
- **The principal bar refusing a registration, and a Registry verb that suspends one.**
  Deferred to doc 17h's follow-ups; both want the registration readable from the Roll's side.
- **`deploy/README.md`** is unchanged: nothing it says is now untrue, and the route asks
  nothing of an operator.

**Not pinned by a story.**

- A world the server binary booted. The stories run on what `prepare` returns in the test
  harness, as MW.0's do.
- `+enduser` on an `Initialize` boot. The verb tests the deployment's mode; the story covers
  a world with no launch, because an initialised world cannot be booted back to `Initialize`.
- A format-1 launch. The route reads only the deployment's mode, Registry account and
  issuer, which both formats have; the stories run on format 2.
- A registration refused by the post after the instrument or an account was written (the
  pinned-queue path). The checks before the first write make it unreachable in a test
  without a fault store.
- A reversal by staff followed by a second registration.

## MW.24

SSH `mind push`, `mind pull` and `mind check` without a presence (doc 44 §7.1; §4c row 23;
Q-44-35 to Q-44-37, whose hand-back at reconnect is why this had to land first, §7.4).

**Built.** A **bare connection**: one bound to a character so that an invoke on it runs as them
and its reply reaches it, and nothing else. It is carried end to end as its own pair of
messages, not as a flag on `ConnUp` (see Divergences).

- **The engine** (`crates/mush-core/src/engine.rs`). `Inbound::BareUp { conn, actor }` and
  `Inbound::BareDown { conn, actor }`. `bare_up` binds the connection in the world
  (`World::bind_connection`, the one thing `handle_invoke` checks) and records it in the engine's
  connection map with `ConnInfo::bare`. It does not touch `conn_count`, `set_live_conns`,
  `set_connected` or `session_start`, announces nothing, and fires no hook and no world event;
  `bare_down` undoes exactly that. So `World::is_connected`, `has_live_conns`, `connected()`,
  `Engine::connections_of` and `WHO` cannot see one. `Engine::actor_of` does answer for it,
  which is what the gateway's readiness probe asks. A `BareDown` for a connection that is not
  bare is ignored, and a `BareUp` on a live connection's id is ignored, so a stray frame cannot
  take a player's real connection down or over. `conn_down` on a bare connection is silent
  (a link that ends takes everything it owned down as `Lost`), and `session_gone` leaves a bare
  connection bound, so an exec that outlives its player's grace still gets its reply.
- **The wire** (`crates/mush-ipc`). `WireInbound::BareUp` and `BareDown`, mirrored field for
  field. Two new `ToServer` arms, so **`IPC_VERSION` 13 → 14**, the first bump inside doc 33's
  window: the golden set `tests/golden/v14/` is recorded (34 `ToServer`, 43 `ToGateway` frames;
  the two new samples are last among the `Inbound` ones, so every older frame keeps its number)
  and the v13 set still decodes. Nothing is gated toward the gateway (no `ToGateway` frame
  changed). **Deploy order is the window's: the server first.** A 14 server serves a 13 gateway,
  which never sends either frame and attaches its exec in full as before; a 14 gateway refuses
  a 13 server. 12 leaves the window, so a gateway that sends no `Hello` is now refused.
- **The server** (`crates/mush-server`). The two frames are classed with the lifecycle
  messages in `world_task/mode.rs` (`ConnectionHooks`, held in the lifecycle slot behind a
  pending commit, allowed in an outage countdown), so they keep `ConnUp`'s ordering against a
  commit and against the invoke behind them. `LinkRegistry::claim_bare` (`links.rs`): the link
  owns the connection for routing and a duplicate id is refused as for any other, but the claim
  does not clear a `SessionGone` the link owes for that actor, and a link that ends with a bare
  connection up does not reap its actor on that account.
- **The session manager** (`crates/mush-session/src/manager.rs`). `attach_bare`,
  `detach_bare`, `bare_conns`, `character_of_conn`. A bare connection lives in its own map:
  no session is made, joined or revived, the link-dead epoch is not read or written, the
  link-dead observer is not called, and it receives nothing addressed to the character, a
  session or everyone. `send_to_conn`, `conn_handle` and the flood bucket work on it by id.
- **The gateway** (`crates/mush-gateway`). `GameBridge::enter_bare`: `attach_bare`, a
  `BareUp`, then the same exact readiness probe a login makes (`ServiceReq::ConnectionReady`);
  a refusal sends a `BareDown` and undoes the attach. `GameBridge::disconnected` ends a bare
  connection with a `BareDown` and nothing else: no `ConnDown`, no reaper. `SshBridge::connect_exec`
  makes the same ownership check as `connect_character` and then `enter_bare`; the exec is
  still watched by a gateway drain (doc 33 §4.6). After a server restart `resync_boundary`
  sends a `BareUp` for each bare connection behind the roster and ahead of the barrier, so the
  invoke the gateway replays from input custody finds its binding and an exec caught by a
  restart completes, as it did when it was an ordinary connection. `ReAnnounce` itself is
  unchanged, and a bare connection is never in it.
- **The SSH transport** (`crates/mush-net-ssh`). `SshHooks::connect_exec`, called by
  `start_mind_exec` in place of `connect_character`. Who may run the exec is untouched: verified
  public key, an owned character.
- **The doorway** (`crates/mush-sshdoorway/src/server.rs`). The pump writes an injected line
  only into a channel whose client asked for a shell (`PumpCtx::shell`, set by `shell_request`
  before it relays, and at once for the menu's channel). Until then the lines wait in the
  queue; in an exec or a subsystem they are never written. The session is still closed at the
  drain's deadline.

**Tests.** All run on Windows and Linux unless marked.

- `mush-core`: `a_bare_connection_is_never_presence` and
  `a_bare_frame_never_touches_a_live_connection` (`tests/module_api.rs`; the toy module now
  records the `revived` and `linkdead` hooks as well): offline, link-dead, grace expiring under
  a bare connection, a `Lost` teardown, and the six hook and event lists compared before and
  after. `engine::typed_invoke_tests::an_invoke_runs_over_a_bare_connection`.
- `mush-session` (`tests/session.rs`): `a_bare_connection_has_no_session_and_leaves_none`,
  `a_bare_connection_does_not_touch_a_linkdead_window` (four minutes in; the expiry instant is
  compared), `a_bare_connection_beside_a_live_session`.
- `mush-ipc`: the round trip and conversion cases gain both frames; the golden suite; the
  version pins moved to 14. `window::tests::a_frame_newer_than_the_link_is_withheld` is now
  written against the window instead of the numbers 12 and 13.
- `mush-server`: `links::tests::a_bare_claim_routes_and_owes_no_reaping`;
  `admit_gateway_pins_the_window_and_the_marker` now pins that a gateway with no `Hello` is
  refused. `tests/split_boundary/ipc_window.rs` is rewritten for the window as it now stands:
  a gateway one step behind is served in full, one at the server's version is served, one with
  no `Hello` and one a step ahead are refused.
- `mush-gateway`: `ssh_bridge::exec_tests` (four, against a link that records every frame):
  an offline exec sends `BareUp`, the invoke and `BareDown` and never `ConnUp`, `ConnDown` or
  `SessionGone`; an exec four minutes into a link loss leaves the expiry instant where it was;
  a server that does not bind it refuses and undoes it; a character the account does not own
  is refused before anything is sent. `resync_orders_full_roster_before_readiness_barrier`
  gains the bare connection's `BareUp` between the roster and the barrier.
- `mushd`: `mw24_a_mind_exec_is_never_presence` (`tests/world_journeys/ssh_e2e.rs`), over real
  SSH on the all-in-one server with a second player watching the room: an offline player's
  `mind pull` answers as them, nobody is told anything and `WHO` has no row; a connected
  player's own exec works; an exec halfway through a real link loss is announced as nothing,
  and the character is disconnected when the original grace ends, less than one grace after
  the exec.
- `mush-sshdoorway`: `a_drain_warns_a_shell_and_writes_nothing_into_an_exec`
  (`tests/e2e.rs`): a spliced shell reads the warning while a `mind pull` through the same
  doorway gets its one result line and no other byte.
- The existing SSH mind stories are unchanged and are the "still works" half:
  `mush-net-ssh` `tests/loopback.rs` (`mind_exec_*`) and `mush-bot`
  `arena_mind_author_story.rs`.

Run on the Windows box: the `mush-core`, `mush-session` and `mush-ipc` tests above; the
doorway's `tests/e2e.rs` (34 passed; with the pump's condition taken out the new test fails on
the warning in the exec's output, so it tests the fix); `mush-net-ssh` `mind_exec` (3); the
gateway's `exec_tests` and the resync test (5); the `mushd` story (with the old full attach put
back it fails on "Nick has connected."); `mush-contract-tests` (35 + 80 + 76); clippy with
warnings denied and rustdoc on the eight changed crates. Left to the Linux CI: everything in
`mush-server`, and the gateway's `tests/split_halves/*` and
`tests/split_boundary/unit_signals.rs` (`cfg(unix)`, so the `Hello` added to
`server_panic_relaunch.rs` was not even type-checked here). CI ran all of them green on the
first full run but one: `mush-server`'s
`ic_slice_loop::ic12_part_two_the_queue_runs_the_return` ("a sale is never negative"), a
trade story that builds no bare connection. It passed when its cell was run again on the
same commit, so it is a flake of that story's own (it ran 93 s on a loaded runner), owed a
look by doc 18's owner and not touched here.

**Divergences.**

- **Two messages, not a flag on `ConnUp`.** The row says "a flag from the SSH hook through the
  gateway bridge and the IPC message to `Engine::conn_up` and `conn_down`". A field on
  `Inbound::ConnUp` would have edited about 110 sites that build one, in fifty files, most of
  them other chains' tests, for a connection that shares none of `ConnUp`'s behaviour. As its
  own variant every exhaustive match had to choose what a bare connection is, which is where
  the server's classes and the link registry's rule came from. **Costly to undo** only in the
  sense that it is now wire vocabulary (version 14).
- **A bare connection takes no slot of the split gateway's lifecycle ledger**
  (`LifecycleCustody`). That ledger counts sessions and the `SessionGone` each may owe, and
  reserving in it converts a link-dead player's owed `SessionGone` into the new connection,
  which is the bug this unit removes. A bare connection is bounded by the SSH accept gates and
  by public-key authentication instead.
- **`SshHooks::connect_exec` has a default** that attaches in full, so the test doubles of four
  crates keep compiling and keep their old behaviour. The gateway is the one production
  implementer and overrides it.
- **A subsystem channel through the doorway gets no banner either.** The rule is "only a
  shell", which is wider than "not an exec" by that one case.
- **Bare frames are refused by a world mode that refuses connections** (booting, recovering,
  integrity-frozen): the exec is then refused at the readiness probe with "Character
  attachment was refused.", as it was before.
- **An exec no longer holds its character connected, so it no longer shields them.** The
  workshop cancels a character's authoring jobs when they go link-dead or disconnect
  (`WorkshopCommands::linkdead`). While the exec was a live connection, a player whose shell
  dropped during their own `mind check` was not link-dead until the exec ended; now they are
  at once, and that check is cancelled with their other jobs. In the other direction, the end
  of an offline player's exec used to fire that hook and now fires nothing: a job the client
  abandoned runs to its own bound and its result is dropped by the connection lease
  (`structured.rs`), which is what already happened for a player connected elsewhere.

**Not built.**

- A story on the split pair (gateway and server as two processes) for the exec, and one for an
  exec caught by a server restart. The resync order is pinned by a unit test; the replayed
  invoke reaching its binding on a fresh server is not run anywhere.
- A story that a ship or a field unit held by a backup mind stays held through an exec. It is
  true by construction (the space seat and the arena bridge act on the `connected`, `revived`
  and `linkdead` hooks and on `has_live_conns`, and none of those moves), and the engine test
  pins exactly those; no tacsim story flies it.
- The exec's refusal still reads "Character attachment was refused." whatever the reason (a
  restart gap, a busy world).
- A second shell channel opened after an exec on the same doorway connection gets no drain
  warning (the inject queue belongs to the first channel). The connection is still closed at
  the deadline.

**Not pinned by a story.** `BareUp` and `BareDown` behind a pending commit (the lifecycle
slot); a duplicate bare id across two gateway generations; a gateway drain's deadline closing
an exec in flight; a link's end with a bare connection up, end to end (the registry half and
the engine half are each pinned).

## MW.17b

The NPC decision call carries its NPC (doc 44 §7.1; first review, finding 8; second review,
§4c row 19; MW.17's first divergence).

**Built.**

- **The lease carries an attribution** (`crates/mush-core/src/decision.rs`).
  `DecisionWorkerLease` gains `attribution: Option<DecisionAttribution>`. A
  `DecisionAttribution` holds three things: the subject a call is charged to (an object id,
  read through `audit_subject()`), that subject's own daily token ceiling when it has one,
  and whether its spend is withheld. That is the whole of what crosses.
- **The owner names it, at the lease** (`crates/mush-core/src/decision_source.rs`,
  `module.rs`). A source's owner registers a `DecisionAttributionAdapter` with
  `Registrar::decision_attribution`, after `decision_source`. The registry asks it on the
  world task in `plan_candidate`, each time it plans a lease, a retry and a lease after a
  restart included, and sets the answer on the planned lease. A source that registers none
  leases unattributed, as before.
- **`mush-npc` reads the profile** (`crates/mush-npc/src/decision.rs`, `attribution_for`):
  the NPC itself, its `daily_tokens` when the profile sets one (the override a dialogue
  turn already passes), and withheld when the NPC is parked, which is the agent tick's own
  test: the profile is disabled or is no longer a valid profile.
- **The provider charges it** (`crates/mush-server/src/decision_llm.rs`, `call_ctx_for`):
  the attribution becomes the call's NPC scope, its per-NPC ceiling and a new
  `CallCtx::parked` flag. `request_for` still takes the prompt alone, so the attribution
  goes to the meter and never to the model.
- **A parked call is refused as disabled** (`crates/mush-llm/src/service.rs`): `call` and
  `stream` treat `ctx.parked` as they treat a kill switch, before the route, the budget or
  any provider, and write the `disabled` audit row against the NPC.
- **`@llm/log` names the NPC** (`crates/mush-worldgen/src/commands.rs`). The row printed
  the triggering player only; a decision has none, so its row named nobody. It now ends
  `actor=<player or -> npc=<npc or ->`, as the dashboard's recent calls already did.

So `@llm/kill` (and `@agent/set <npc>/enabled=no`, the same flag) stops a decision that was
already captured: the call is refused before the model is asked, each attempt leaves a
`npc_decision disabled` row against the NPC, and the decision reaches `npc_idle` when its
attempts are spent (three, each refused at once) or at its deadline, whichever is first. A decision that is answered is counted on its NPC's budget counter and is refused
up front when it would pass the NPC's ceiling.

**What moved in a stored or versioned format: nothing.** The lease is not serialised
anywhere; it is built in memory when a lease is planned and dropped when the worker
answers. The attribution is not written to the pending record, the prompt record or the
terminal receipt (`PendingDecision::SCHEMA_VERSION` stays 2), and the adapter does not
enter the decision-source registry fingerprint, because it selects no durable outcome.
`plan_worker_lease` keeps its signature and plans an unattributed lease; only the
registry attributes one.

**What the survey had wrong.**

- "`@llm/kill` reaches an NPC's dialogue but not its decisions" is half true. `@llm/kill`
  does not use the LLM service's kill switch at all: it sets `enabled = false` in the
  NPC's profile (`llm_toggle_npc`), and `KillSwitch::disable_npc` has no caller. The agent
  tick already refuses a NEW decision request for a disabled NPC (`issue_capture`). What
  nothing reached was a decision already captured: it was still leased, and the provider
  still asked, up to the kernel's attempt limit.
- "The lease names a controller and a decision, not an object" is true of the lease, but
  the pending record it is planned from has always held the acting object
  (`PendingDecision::actor`), so nothing new had to be stored to attribute it.

**Defaults taken.**

- **The owner names the attribution; the kernel does not derive it from the actor.** For
  `npc` the two are the same object. The hook exists because the ceiling and the parked
  flag are the owner's facts, and because a later owner may charge a decision to something
  other than its actor.
- **The subject is an object id, not a hash.** The budget ledger, the per-NPC ceiling,
  `@llm/budget`, the dashboard's top spenders and the audit row all key on an object id
  and print the NPC by it; an opaque key would need a second table to turn it back into a
  name. The type is inert instead: it is not serialisable, it has one reader
  (`call_ctx_for`), it never enters the prompt, and a worker holds no world to resolve it
  against. Reversing this later (a keyed hash) is contained to `mush-llm`'s scopes and the
  staff views, not a refactor of the kernel.
- **Parked is a flag on the call, not the service's per-NPC kill switch.**
  `KillSwitch::disable_npc` would have to be kept in step with the profile by every path
  that writes it, and seeded again at boot. Reading the profile at the lease has no state
  to drift. `KillSwitch::disable_npc` stays as it was, with no caller.
- **A parked decision still runs its lease and its attempts.** The kernel's lifecycle is
  untouched: each attempt is refused at once and audited, and the turn goes to `npc_idle`
  when the attempts are spent or the deadline passes. Nothing cancels the pending record
  early.

**Tests.** No test makes a provider call.

- `decision::tests::lease_carries_only_its_prompt_its_fence_and_an_attribution`
  (`mush-core`): the pin. It destructures the lease and the attribution with no `..`, so a
  field added to either stops the test compiling; it also holds that a lease the kernel
  plans alone names nobody, that attributing a lease leaves the durable post-image the
  same, and that the attribution is no larger than an id, a ceiling and a flag.
- `mw17b_a_decision_is_attributed_to_its_npc_on_the_npcs_present_standing`
  (`crates/mush-npc/tests/agents.rs`): open, capped by `@agent/set`, parked and woken, and
  an object that is no agent.
- `a_parked_npcs_call_is_refused_as_disabled_and_audited_against_it`
  (`crates/mush-llm/tests/service.rs`).
- `decision_llm::tests::mw17b_the_attribution_sets_the_calls_npc_scope` (`mush-server`).
- Three stories through the production world task and the production provider over a mock
  (`crates/mush-server/src/world_task/tests.rs`), each failing before this unit:
  `mw17b_a_decision_is_charged_to_its_npc_and_names_it_in_the_audit` (the counter, the
  row, and the NPC's dbref absent from the request the model was sent);
  `mw17b_llm_kill_stops_a_decision_that_was_already_captured` (the real `@llm/kill`
  between the capture and the lease: no call, no tokens, `disabled` rows against the NPC,
  the turn idles); `mw17b_an_npcs_own_daily_ceiling_holds_for_its_decisions`.
- `llm_log_names_the_npc_a_call_was_charged_to` (`crates/mush-worldgen/tests/worldgen.rs`).

**Not built.**

- A kill that lands while a decision's call is in flight does not recall it. The call
  finishes and is charged; the lease is at most `NPC_DECISION_LEASE_SECS` (60). Dialogue
  has the same window.
- `@agent/show`'s last turn (MW.17) still covers dialogue turns only. A scene-turn
  decision is read in `@llm/log` and in its turn receipt. Successor: MW.18 records the
  ambient turn, which is a dialogue turn; a decision's own line in `@agent/show` has no
  unit yet.
- `KillSwitch::disable_npc` and `enable_npc` remain uncalled.

**Not pinned by a story.** A lease planned after a restart carrying a fresh attribution
(the path is the same `plan_candidate`); a second owner's adapter.

**Limits of the tests.** That the attribution never reaches the model is held by the
shape of the code (`request_for` takes the prompt alone, and no provider is handed a
`CallCtx`); the story's own probe only checks that the NPC's dbref string is absent from
the request. The two refused-decision stories run the production world task on the real
clock, with a five-second deadline and a twenty-second wait.
