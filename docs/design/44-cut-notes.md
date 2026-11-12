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

Not built.

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

Not built.

## MW.10

Not built.

## MW.11

Not built.

## MW.12

Not built.

## MW.13

Not built.

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

Not built.

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

Not built.

## MW.19

Not built.

## MW.20

Not built.

## MW.21

Not built.

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

RESULTS_PLACEHOLDER

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

Not built.

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
