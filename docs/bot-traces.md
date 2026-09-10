# Offline trace inspection

Open a saved cast or individual bot transcript to investigate its recorded
requests, replies, commands, observations and probe grades:

```powershell
cargo run -p mush-bot -- --inspect-trace .mush-bot/party-001/run-0001/joined.jsonl --tui
cargo run -p mush-bot -- --inspect-trace .mush-bot/party-001/run-0001/scout/trace.jsonl --tui
```

This viewer reads one explicit JSONL file. It does not load dotenv or bot
configuration, resolve paths embedded in records, contact a model, connect to
SSH, or send controls. The displayed detail is the actual selected record;
there is no reconstructed historical memory or inferred game state. Opening a
trace does not change the original run's interactive-debug classification.

## JSON reports and saved queries

Without `--tui`, inspection prints a JSON report. Use a new `.json` path to save
it, and reuse an authored JSON query across experiments:

```powershell
cargo run -p mush-bot -- --inspect-trace .mush-bot/party-001/run-0001/joined.jsonl --trace-query bots/trace/requests.json --report requests-index.json
```

The default report contains source path, exact input SHA-256, trace format,
normalized query, total/matching/returned counts, pagination and event metadata.
Each entry retains its one-based source line, actor, event kind, recorded clocks
and explicit request/command IDs. Add `--include-content` to include each
selected entry's original parsed JSON under `raw`. This preserves JSON values;
the file hash identifies the exact original bytes, including whitespace.

Payloads are omitted by default. Source paths, actor names, event kinds and
authored query text remain in metadata reports. Full-content exports and the
TUI show recorded conversations, prompts and private briefings when present.

```json
{
  "version": 1,
  "actors": ["scout"],
  "kinds": ["model-input", "model-reply", "probe-result"],
  "contains": "Marco",
  "from_ms": 1000,
  "to_ms": 60000,
  "offset": 0,
  "limit": 200
}
```

Empty actor/kind arrays allow every value; populated arrays use exact matching.
Different fields combine with AND. `contains` is a case-insensitive literal
substring of the serialized original record, including its data. It is not a
regular expression or a semantic search. Omit it or use `null` for no text filter.

Time bounds are inclusive and apply to the joined trace's `elapsed_ms` only.
Standalone transcripts have an absolute `timestamp_ms`, but no common harness
clock; an elapsed-time filter therefore matches no standalone records. Missing
IDs and clocks remain null. Records stay in source order even if absolute wall
timestamps move backward.

`offset` skips matching records, and `limit` controls JSON export pagination
(1–1,000, default 200). `pagination.next_offset` is null when the result is
exhausted. Filters allow at most 64 distinct actors and kinds, 256 bytes per
value, and 4,096 bytes of text. Invalid versions, unknown fields, duplicates,
reversed time bounds and invalid pagination are rejected before writing a report.

The TUI uses the query as its initial scope and starts at its offset. It lets
you navigate all matching records; the export limit does not hide later events.
The TUI cannot be combined with `--report` or `--include-content`.

## Timeline controls

| Key | Action |
| --- | --- |
| Up / Down, PgUp / PgDn | Move through matching records |
| Home / End | Select the first or last matching record |
| F2 | Cycle the actor filter |
| F3 | Cycle the event-kind filter |
| `/` | Edit an additional literal text filter |
| Enter / Esc while editing | Apply / cancel the edit |
| Ctrl+U while editing | Clear the search draft |
| F4 | Toggle records sharing the selected event's explicit IDs |
| F6 | Clear all UI filters, retaining the authored query |
| Ctrl+PgUp / Ctrl+PgDn | Scroll the selected record's detail |
| Q, or Ctrl+C at any time | Close the viewer |

UI filters narrow the authored query. Cycling filters back to all or clearing
the local search restores that authored scope. Terminal escape/control sequences
are sanitized for display. Long details are bounded and labeled; use a
full-content JSON report or the original trace to read a complete large record.
The local search editor accepts up to 1,024 UTF-8 bytes. While editing, Q is
ordinary search text; Ctrl+C still closes the viewer.

Related records share the selected record's actor and either its non-null
`request_id` or its non-null `command_sequence`. The view makes only that direct
association; it never crosses actors, uses proximity to guess missing IDs, or
recursively follows a chain of associations. Runner IDs can remain attached to
later observations. Sharing an ID is therefore a recorded association, **not
proof that a model request caused a command or a world event**.

## Integrity and limits

The loader accepts the bot's standalone `trace.jsonl` format and the harness's
`joined.jsonl` envelopes, with one format per file. Unknown event kinds are
retained. It validates record envelopes and metadata rather than treating a
stored success event as proof of a passing experiment. Use the summary analysis
and regression tools for acceptance checks, and capture/probe validators for
their respective payloads.

Loading is bounded to 64 MiB, 100,000 events and 4 MiB per record. A malformed
record, duplicate object key, mixed format, invalid metadata or truncated final
JSON causes a line-specific error; no partial report is produced. A complete final JSON record
does not require a newline. Incomplete sessions with valid records can still be
inspected; no final result record is required. The file is read once rather than
followed as a live tail. An incomplete concurrent write is rejected in the same
way as any other malformed record.

The library exposes `trace::load`, `Trace::matching`, `Trace::report` and
`Trace::related`. Loading/querying succeeds independently of whether the recorded
experiment passed. The CLI exits 0 for a successful inspection and 1 for an
invalid input, incompatible options or an unavailable interactive terminal.

See [harness analysis](bot-analysis.md), [regression gates](bot-regression.md)
and [live phase debugging](bot-cast-debug.md) for the surrounding workflow.
