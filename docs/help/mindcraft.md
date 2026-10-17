# Mindcraft

Open Minds with F10 in the browser or TUI. The browser has a source editor,
card widgets, profiles, presets, rules, Check, and Save. The TUI offers the
same library and card workflow with a read-only source view; use line editing
or SSH for source changes. Your library is private to the selected character.

The line workflow starts with a working copy of Standard's helper:

  @mind/new Patrol FROM Standard
  @mind/source Patrol
  @mind/card Patrol/Line
  @mind/set Patrol/Line min_odds_permille=350
  @mind/profile Patrol add Cautious FROM Line
  @mind/set Patrol/Cautious withdraw_permille=700
  @mind/check Patrol
  @mind/save Patrol=First flight

Line edits use exact stored values: 350 per mille displays as 35%; a timer
stored in milliseconds uses 1000 for one second. The card labels and widgets
show the display units. A preset copies values into a profile; it does not
replace code or add behavior. The inert Standard terms listed in `help minds`
remain inert unless your authored code uses them.

Check and Save run as jobs. Check compiles the source and exercises six public
pictures, reporting operations, fees, intents, and refusals or faults. It is
a bounded check, not evidence that every possible battle is safe. Save repeats
validation and creates an immutable revision. Wait for its completion before
selecting it. A changed working copy makes an old save request stale; reload
deliberately to discard local edits, or preserve them before retrying.

Source editing commands work outside a paste session:

  @mind/line Patrol 1=fn think(view, card) {
  @mind/insert Patrol 2=  trace("thinking");
  @mind/delete Patrol 2-2
  @mind/profile Patrol rename Cautious=Home
  @mind/preset Patrol/Home save Safe
  @mind/preset Patrol/Line apply Safe
  @mind/rule Patrol/Line 1=OFF
  @mind/save Patrol@trial=Named test revision
  @mind/diff Patrol@1=Patrol@2

For a whole-source replacement, enter `@mind/paste Patrol`, paste the source,
then send a lone dot to finish. `~q` cancels without replacing the draft. Until
you finish or cancel, every line is source, including lines beginning `@mind`.
Source is limited to 32 KiB of UTF-8. Card metadata is bounded separately. Source execution has
no filesystem, network, wall clock, or access to hidden opponents: it sees its
public picture, its own card, and bounded persistent memory. It must stay
within the declared class's operations, contacts, rules, and memory budget.
Rules use the shared cockpit grammar and canonical weapon IDs; an all-weapons
rule binds to the fitted weapons when sealed. Invalid rules are refused.

Use `@mind/list`, `@mind/info Patrol`, and `@mind/source Patrol@2 1-40` to
inspect your library. `@mind/localcopy Patrol@2=PatrolNext` starts a separate
owned draft. Editing or saving any draft leaves existing sortie seals intact.

## Library and version lifecycle

Read the current working copy before a change. The TUI uses its exact edit base;
N renames the loaded mind, X removes it, v renames a named version, and V forgets
one. Each opens a form and requires Enter. In line mode:

  @mind/rename Patrol=PatrolNext
  @mind/version-name PatrolNext@trial=trusted
  @mind/version-forget PatrolNext@trusted
  @mind/remove PatrolNext
  @mind/prune 128

Removing a library entry keeps its named versions and retained history, plus
revision records needed by seals, selections, proves, and benches. Forget any
unwanted named versions before removing the library entry. Forgetting a version
removes that label;
it does not erase a still-referenced revision. Prune removes at most 1–128
unreferenced revisions owned by this character (128 by default), and keeps
immutable source bytes. It is explicit: press g then Enter in the TUI.

## Source over SSH

The SSH exec commands are exactly `mind push`, `mind pull`, and `mind check`.
They require a registered public key whose signature the server verifies and
an owned selected character. Send one typed JSON request on standard input,
then EOF. Pull reads a working copy or saved revision; Push creates, replaces,
or saves; Check runs validation. Each command accepts only its matching
operation. The response includes the current edit/revision base: send that
exact base with a change so simultaneous edits cannot overwrite each other.
The browser uses the same typed authoring service and stale-edit checks.
For example, with a registered key and `account+character` as the SSH user,
send this JSON to `mind pull` and close standard input:

  {"op":"read","name":"Patrol","revision":null}

The reply's `base` contains `edit` and `revision`. To check that same working
copy, send `mind check` a JSON object with `op` set to `check`, the same `name`,
and that exact `base`. For a saved pull, set `revision` to a string such as
`"2"` or the named version. Push is a separate explicit operation: `create`,
`replace` with the complete draft and base, or `save` with base, notes, and an
optional version. Closing standard input is required; the exit status reports
the final operation result. The complete JSON frame is bounded to 272 KiB.

See also: `help minds`, `help proving`, `help cockpit`.
