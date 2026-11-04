# Interactive harness phase debugging

Use the cast TUI to inspect a coordinated experiment one phase at a time:

```powershell
cargo run -p mush-bot -- --harness bots/harness/organized-party.json --output .mush-bot/party-debug-001 --tui --harness-debug bots/debug/phase-gates.json
```

The debugger holds **phase dispatch**. Bot SSH sessions, independent bot goals,
listening, scripts and the game world continue running. Observations, global
assertions, disconnections and server failures remain active. Use explicit
`/pause` controls in the authored experiment when particular bots should stop
autonomous play. A phase hold does not send `/pause` or `/resume` to characters.

## Controls

| Key | Action |
| --- | --- |
| F2 while running | Request a hold at the next phase boundary |
| F2 while held | Continue dispatching phases |
| F2 while a hold is requested or while stepping | Cancel the upcoming manual hold and continue |
| F5 while held before a phase | Dispatch one complete phase, then hold at the next boundary |
| Tab | Select a bot |
| F9 | Cycle the selected bot's inspection panels |
| PgUp / PgDn | Scroll its game session |
| Ctrl+PgUp / Ctrl+PgDn | Scroll inspection details |
| Q | Cancel the owned cast and save reports; close the display after completion |

A serial group phase counts as one phase: stepping waits for every targeted
member's acknowledgment and required observations or probe grade. It does not
stop between those members. Failed assertions or grades still fail the run.
After stepping the final phase, the debugger holds before normal cleanup so the
final state can be inspected. F2 continues to save reports and finish.

Requesting a hold during a phase lets that phase finish. Its timeout keeps
running. At a held boundary, the overall harness active-time budget excludes the
hold, while a separate hold deadline remains active. This lets a person inspect
state without consuming the next phase's timeout. Individual bot budgets and
world time are unaffected.

## JSON settings

```json
{
  "version": 1,
  "start_held": true,
  "before_phases": [2, 4],
  "max_hold_ms": 300000
}
```

`start_held` holds after player connections are ready and before initial
`actor.start` controls or the first phase. `before_phases` contains one-based
phase numbers, each reached once per run; nonexistent phase numbers are rejected
before launching bots. Settings are applied afresh to each matrix run.
`max_hold_ms` is the deadline for each individual hold, from 100 to 3,600,000 ms;
the default is five minutes. An expired hold fails the run and shuts down owned
processes. It never silently continues the experiment.

Validate the settings without loading dotenv or connecting:

```powershell
cargo run -p mush-bot -- --check-harness-debug bots/debug/phase-gates.json
```

This validates the settings schema. A phase number's existence is checked against
the chosen manifest at launch. With `--tui` alone, the cast starts running and F2
can request a hold. `--harness-debug` requires `--harness --tui`; omit the TUI for
an unattended CI run. Settings use JSON; they are separate from character debug
settings such as per-bot command breakpoints.

## Evidence and reports

Controller commands include the current matrix run and debugger revision.
Commands queued against an older boundary or run are rejected. Repeated F5
presses cannot accidentally release later boundaries. A reliable latest-state
channel keeps the displayed debugger mode and revision current, even when the
bounded ordinary display event queue skips an update.

Accepted and rejected controls and debugger transitions are retained in the
joined JSONL trace. Reports mark debugger-enabled runs and retain cumulative hold
time. `elapsed_ms` remains wall time; `active_elapsed_ms` excludes phase holds.
Run timing starts once actor processes have been launched, as in unattended
reports. Joined-trace timestamps also include the earlier launch/readiness
prelude and use one continuous clock throughout that trace.
The selected bot's observations, captured prompt, costs and memory remain local
to that bot's inspection panels.

Analysis distinguishes debugged, unattended and unclassified legacy runs, and
reports both timing measures. Regression gates treat interactive debug runs as
inconclusive, even if both inputs were debugged or no hold was requested. Legacy
summaries without a debug marker are also inconclusive. Use a fresh unattended
run for acceptance checks; wall-clock game changes during human inspection cannot
be removed by subtracting hold duration.

Disconnecting the controller or cancelling the display shuts down the owned cast.
It cannot turn an unfinished phase into success. Native terminal interaction has
not been automated; key handling and layouts use Ratatui's test backend, and the
controller is tested against actual bot binaries over public SSH.
