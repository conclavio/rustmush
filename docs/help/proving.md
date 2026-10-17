# Proving and the replay bench

Proving runs use saved revisions and the real simulation in private arenas:

  @mind/prove Patrol@2/Line VS Standard/Hunter 30 MUSTER 1v1 SEED 1
  @mind/proves
  @mind/proving 1
  @mind/report 1 1
  @mind/cancel 1
  @mind/forget 1

Choose an even 10–100 matches, equal sides from 1v1 through 6v6, and Duel,
Muster, or Hold. Each seed is flown from both sides; 30 matches use 15 seeds.
Quote an entire reference containing spaces, for example
`@mind/prove "my patrol@2/Line" VS mirror 30 MUSTER 1v1 SEED 1`.

The report separates wins, losses, draws, and timeouts, gives a 95% win-rate
interval, and records faults, operations, rules, weapons, damage, and rounds.
It preserves the scenario, chassis, settings, source digests, and seeds used.
Compare those settings before comparing results. Overlapping intervals do
not establish a difference. Proving shares an account quota and fair queue;
cancelling keeps already spent work charged. Forget removes an unpinned
finished result. Your runs and reports are visible only to your character.

The replay bench opens a game from an owned proving run:

  BENCH LOAD 1/1
  BENCH STEP 1
  BENCH BACK 1
  BENCH RUN TO 1:30
  BENCH PIC
  BENCH TRACE
  BENCH CLOSE

See also: `help minds`, `help mindcraft`.

## Retained tapes and inspection

Every completed game contributes to the report. Replay tapes have a separate
byte limit: a game whose tape does not fit is explicitly marked unavailable
for replay. No partial tape is opened by the bench. A large run can therefore
finish with a complete report and only some replayable games.

You can also inspect a kept match you are permitted to read:

  BENCH LOAD MATCH 123 RA

Inspection uses the captured configuration and source revision. Missing,
incomplete, incompatible or tampered recordings are refused. Going back
reconstructs the reference from its start; it does not change a live match.
The picture displays its exact state hash, cursor and simulation time.

Read-only expressions inspect the selected picture with `pic`, card values
with `card`, and permitted own-mind memory with `mem`:

  BENCH EVAL pic.speed_kph
  BENCH WATCH mem.state
  BENCH WATCH OFF

Expressions have a separate 500-operation budget including fees. They cannot
issue station commands or alter the replay. Inspection of memory requires
permission to read that mind; errors never reveal another mind's source.
There is one bench per account, with a 30-minute idle expiry. Stepping shares
the proving allowance; reading a cached picture or report is free.

## Compare and fly a fork

Select another unit only in your private proving run, or compare an owned saved
revision on the reference pictures with fresh independent memory:

  BENCH ACT AS RA
  BENCH DIFF Patrol@3/Line
  BENCH STEP 10
  BENCH DIFF OFF

The comparison reports differing think results without changing the reference's
intents or memory. It keeps at most 64 difference rows and reports omitted rows.
Each compared unit consumes an additional unit's proving allowance.

To test a candidate's effects, start a labelled fork at the settled reference
cursor. You can also possess that unit and issue ordinary station commands:

  BENCH SWAP Patrol@3/Line
  BENCH POSSESS
  BENCH DO SPEED 20
  BENCH STEP 10
  BENCH RELEASE
  BENCH REFERENCE

Release hands the fork back to its mind. Reference discards the fork and returns
to the unchanged reference cursor. Back on a fork reconstructs its command
history and cannot go before the fork's starting time. Candidate class must fit
both the captured execution class and the hull's physical core; use a new proving
run to test a higher class. Comparisons and fork edits require a settled cursor.
