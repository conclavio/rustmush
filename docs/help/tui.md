# The Full-Screen TUI (`@tui`)

Most players use rustmush as a classic scrolling line client: you type a
command, the world answers, the text rolls up the screen. That line mode is
the **default** and is not going anywhere.

`@tui` is an optional full-screen alternative for terminal clients, rendered
server-side over your SSH connection. It is built from **numbered screens**
of titled panes: a scene screen with your scrollback, the room and who is
about; a sheet screen; mail; channels; rolls; intel; actions; two
tactical screens — a damage report and a unit view, whose **grid** pane is
the map of the window your scene stands on when one is open (`help map`) —
that show your unit whenever you hold a station on one (they are always in
the strip); on digit **0**, a **cockpit** for a pilot in a simulator
match, which appears the first time a match sends you a frame; and two
**lettered screens**, reached with Alt and the letter where the server
publishes them: **c**, the sky chart, and **q**, the quarry, for a pilot
working rigs on a mining site.
Everything you type still runs the same commands and reaches the same world;
the TUI is only a different way to *see* it.

## Launching and leaving

  @tui        Switch this connection into the full-screen TUI. Your terminal
              flips to its alternate screen and the dashboard paints.
  @notui      Leave the TUI and return to ordinary line mode. Your terminal's
              normal screen (and scrollback) comes back untouched.
  Ctrl-]      A hotkey that also detaches — the same as `@notui`, from any
              mode. `/notui` typed in the box does the same.

Line mode is unaffected by any of this: a connection that never types `@tui`
behaves exactly as it always has, byte for byte. If your connection drops
while the TUI is open, the alternate screen is restored for you, so you are
never left stuck on a blank full-screen buffer.

You need a real interactive terminal (a PTY) for `@tui`; it does nothing over
a non-interactive pipe.

The TUI is laid out for a terminal of at least 120 columns, and 160 to 180
columns is the width it is happiest at. Narrower than that no screen is
drawn: a short message names the floor and your terminal's own width, the
input box stays, and the moment the window is wide enough again the screen
you were on comes back as you left it. Line mode has no hard floor, though
its maps and tables are drawn for 120 columns: a map prints at most 118
columns by 32 rows.

## Two modes: nav and input

The TUI is **modal**. It starts in **nav** mode, where single keys act:
digits switch screens, letters run the screen's own commands, `n` walks
north. Press **Enter** to open the input box; in **input** mode the box is
an ordinary MUSH command line — type, edit, press Enter to send — and it
stays open, so a conversation costs one Enter per line. Enter on an empty
box, or Esc, takes you back to nav mode. The input frame always shows which
mode you are in (`‹nav› Enter to type` when the box is closed).

Every nav hotkey is still reachable while the box is open as **Alt+key**:
Alt+3 switches to the mail screen where a bare `3` did, Alt+n walks north.
(On a terminal whose Alt key is not "Meta", Esc then the key does the same
in two strokes: Esc closes the box and keeps what you typed as a draft, and
the key acts.) F1–F9 also switch screens from either mode; the cockpit is on
`0` and `Alt+0`, which no function key doubles.

## Keys

Nav mode (the box is closed):

  0-9          Switch to that screen (the tab strip along the top). The
               strip reads `1 … 9 0`, which is the order of the digits on
               a keyboard.
  Enter        Open the input box. If the focused pane has a highlighted row
               — a mail message, an action, a channel — the box opens with
               that row's command ready (`@mail 3`, `+act brace`, `pub `).
  " : ; \      Open the box with that speech trigger already typed.
  /            Open the box with the command palette (below).
  Tab / Shift-Tab   Move focus between panes.
  Up / Down    Scroll the focused pane, or move its highlight.
  PgUp / PgDn  Scroll a page.
  g / G        Jump to the top / bottom of the focused pane. On the
               scrollback, `g` (or PgUp already at the top) also asks the
               server for the scene history above it.
  ?            The help overlay for the current screen (Esc closes).
  n e s w u d o   On the scene screen, walk the room's one-letter exits.
               The status line prints each exit with its key (`n·north`).
  <letter>     The screen's own commands, printed on the legend line at the
               bottom (`l look`, `r refresh`, `s send`, …).

Input mode (the box is open):

  Enter        Send the line and stay in the box; an empty box returns to nav.
  Esc          Back to nav mode, keeping the line as a draft for next time.
  Left/Right, Home/End, Ctrl-A/E, Backspace, Delete, Ctrl-W (kill word),
  Ctrl-U (kill to start), Up/Down (history), Tab (complete a command name).
  /            On an empty box, opens the command palette.
  Alt+key      The key's nav action (Alt+0 … Alt+9, Alt+? help, Alt+letter).

From either mode:

  Ctrl-K       Open or close the command palette.
  Ctrl-P       Cycle the sticky mode: none → say → pose → emit → ooc. With a
               sticky mode set (shown in the input frame as `‹say›`), a line
               without an explicit prefix is sent as that kind of speech. A
               line starting with `"`, `:`, `;` or `\`, or with a channel
               alias, is always sent as typed.
  Ctrl-R       Open the box with `+roll ` ready (Tab completes sheet traits).
  Ctrl-N       On the channel screen, highlight the next channel of the
               focused list (wrapping); Ctrl-O swaps between the IC and OOC
               lists. Elsewhere both do nothing.
  Ctrl-]       Detach to line mode.

A single key in nav mode never sends anything that changes the world: it
looks, switches, scrolls, or opens the box with a line ready for you to
confirm with Enter. The one exception is walking an exit on the scene
screen.

## The palette

`/` (or Ctrl-K) opens the command palette above the input box: every
command you may use, your channel aliases and the room's exits, filtered as
you type after the slash. Enter (or Tab) puts the highlighted entry in the
box — a game command's usage line, ready to edit — and Esc closes the
palette keeping your text. A few commands are the TUI's own and keep their
slash: `/clear` hides what is on screen, `/help` opens the key help,
`/history` loads older scene history, `/notui` detaches.

## Scene history

Entering the TUI fills the scrollback with the last of the scene you were
in, so a reconnect drops you into the conversation rather than a blank
frame. `/history` — or `g`, or PgUp once you are already at the top — asks
for the page above what you are reading; older lines appear above your
place without moving it, and a dim line at the very top says when there is
nothing more (`no older history`, or `older history has rolled off the
server's buffer` when the server has dropped it).

`/clear` draws a floor under what you have read rather than throwing it
away: the screen empties and new lines appear below the floor, but nothing
older is loaded into that view again — `@notui` then `@tui` gives you a
fresh one, primed from the newest as usual. It has to work that way: the
client cannot tell a line it has already shown from the same line coming
back inside a page, so a cleared view that went on asking would paint your
last few minutes twice.

In line mode the same history is `@history` (or `+history`): `@history`
prints the last 20 lines oldest first, `@history 50` asks for that many (up
to 100), and `@history more` continues from where the last page stopped.
Like `@tui`, it is answered by your connection rather than the world, so it
never counts as activity — reading history back leaves you idle in WHO.
That is also why it is read wherever you type it, including inside a prompt
the game is waiting on: `@history` at a `Mail>` prompt or a chargen question
reads the scene back, and is not taken as the line you were composing.

## The screens

  1 scene    Your scrollback with a speaker gutter (who said it, or the time),
             the WHO rail, the current scene, and the actions offered to you.
             The status line under the tab strip shows the room, its zone,
             the weather, the in-character clock and the exits.
  2 sheet    Your character sheet, walked from the ruleset's schema.
  3 mail     Your inbox: number, unread marker, sender, subject. Enter on a
             row opens the box with `@mail <n>` ready; `s` starts a send,
             `d` a delete of the highlighted message.
  4 chan     Your channels on the left, in-character ones (green frame) above
             the out-of-character ones (yellow frame): alias, name, unread
             count. The highlighted channel's recent lines fill the right, in
             a pane named for that channel; `b` asks for more backlog. Enter
             opens the box with the highlighted channel's alias ready — and
             on a server with no in-character channels, where the focus lands
             on the empty IC list, with the alias of the channel the right
             pane is showing. Ctrl-N walks the focused list, Ctrl-O swaps
             lists.
  5 rolls    Your vitals as widgets on the left — tracks as boxes in threes
             with the level named, pools as gauges, readouts with a condition
             mark — and the dice you have rolled on the right. Ctrl-R opens
             the box with `+roll ` ready; Tab then completes a rating from
             your sheet (`+roll gun` → `+roll gunnery`, `+roll pil` →
             `+roll piloting`).
  6 intel    Your own informant intel, newest first.
  7 actions  The actions currently offered to you; Enter prefills `+act`.
  8 damage   The after-action report for the unit you hold a station on:
             its sections with a condition mark and a gauge for every part
             the ruleset tracks, the newest damage report's before/after
             table beside it, and the journal those tables come from — one
             line per thing that changed, newest report first. `r` asks
             for a fresh reading; `d` opens the box with `@unit/damage ` and
             `u` with `@unit/show ` ready (neither sends until you press
             Enter, and both are staff commands).
  9 unit     The same unit at a glance: its sections one line each, its
             components grouped by where they are installed with the
             capabilities each one gives, and its unit-wide readings with
             the current mobility mode. `r` refreshes, `c` runs `+contacts`,
             `d` opens `@unit/damage `.
  0 cockpit  Your simulator cockpit, the whole of what the panel tells you,
             updating by itself as the arena sends it. The arena's ground
             fills the middle, drawn around your own tank (`@`) with your
             contacts on it — `x` hostile, `+` friendly, `?` not told apart
             yet — and the line under it says which glyph is which. On the
             left: your callsign, your power, speed and heading, your heat,
             your scope, your lock, and what is left of your tank section by
             section. On the right: your weapons with their cycle, rounds
             and chance, your contacts — bearing, band, range, how old the
             plot is, and the sections of any you have had a close look
             at — and the notices: what the arena tells you, and the answer
             to anything you type while you are in the tank. At the
             120-column floor every pane fits side by side, no status,
             weapon or contact row wraps, and the damage pane shows a
             tank's five sections whole from 30 rows; a row longer than its
             pane (a contact's scan) wraps onto the next line, and on a
             shorter terminal the damage pane is the first to go.
             `c` `t` `w` `r` `b` `n` run CONTACTS, STATUS, WEAPONSTATUS,
             RANGE, BEARING and SCAN, and their answers land on the notices
             pane; `k` `f` `h` `m` open the
             box with `lock `, `fire `, `heading ` and `speed ` ready, and
             none of those is sent until you press Enter. `p` pulls the
             panel again, and the map too if it has gone dark. On the contacts
             pane, Enter opens the box with `lock <contact>` ready and `s`
             with `scan <contact>`; neither goes in until you press Enter
             on it. The tab appears the first time a match sends you a
             frame, and the screen opens itself then; the tab goes when you
             leave the tank, and if you were on it you are put back on the
             scene screen.
             After a match, `SIM REPLAY <match>` opens the replay on this
             screen, and the tab with it: the ground with both sides drawn
             on it (`o` red, `*` blue, `%` a wreck), one keyframe at a time.
             The left says which match and how it ended, which keyframe,
             whose tank the map is centred on and that both sides are shown;
             the damage pane lists the tanks wrecked so far, the weapons
             pane the shots fired since the keyframe before, and the
             contacts pane every tank of both sides where it stood. `,` and
             `.` step back and on, `u` moves the map to the next tank (Enter
             on a row of the contacts pane moves it to that one), `x`
             closes, and `p` pulls the replay again. Positions are the
             arena's own, every 5 s, and the TUI draws nothing between them.
             Sitting in a tank closes the replay; a replay never opens over
             a cockpit you are flying.
             Staff who `@arena/watch` a running match see it live on the
             same screen (`Watching 12 - 1:30 (live)`), every tank with its
             object number, at most once an arena second; only `u` and `p`
             work there, and it closes when the watch ends.
  c chart    (Alt+c) The sky as a flat map: the Sun, every orbit, the bodies
             by letter and the listed routes, you at `@`; beside it the
             passages now and each destination's window. `r` asks again,
             `w` opens the box with `+chart ` ready, `m` prints the flat map.
  q quarry   (Alt+q) Your rigs on a mining site: their status on the left,
             the map in the middle — the drive view around a rig, then
             (`z`) the whole site and the body's ground chart — your own
             readings, the other rigs and the log on the right. `s` `d` `x`
             `m` open the box with `+tv/<n> scan`, `drive `, `sound` or
             `mine` for the rig you picked (`t` picks the next); nothing
             goes until you add `go` (and, for a drive, the target first)
             and press Enter. `l` shows another kind of ore, `r` asks
             again; the map follows your rigs while they move.

The two tactical screens are always in the tab strip, and they are honest
when there is nothing to show: with no station and no unit of your own they
read `No tactical unit: take a station, or @unit/show #n if you control
one.` A damage report that reaches you about a unit you hold no station on
— staff damaging someone else's — still fills the journal and the
before/after table, with each line naming the unit it is about. Screen 9's
contacts pane is the table `+contacts` prints, when the server publishes it
(Enter on a row prefills `+contact <id>`); it also carries panes for things
no server publishes yet — a grid, weapons, comms — each saying what it is
waiting for. Those are the first panes to go when the window is too short
for them all, so a real pane never loses its place to an empty one. Colour and condition marks
on both screens come from the ruleset's own judgement, never from the
client's reading of a word.

Above the input box, when your ruleset publishes vitals, a strip of gauges
shows your health track, pools and readouts; without one the prompt shows
as in line mode. Your own character's gauges come first, ahead of any unit
or vehicle you are also reading, and the strip is one row: when there are
more gauges than fit, the ones that did not fit are counted at the right
(`+4 more`) rather than dropped in silence — screen 5 shows them all. The
chrome drops out below 12 rows, so a short window always keeps the
scrollback and the box.

Password and hidden prompts (login, chargen) open the box on their own and
mask or hide your typing, just as in line mode. A line you were part-way
through typing is put aside rather than carried into the password field: it
comes back the next time you open the box on an empty line.

When the game asks you something — a confirmation, a pick from a few
options, a short line — an overlay opens over the screen. Pick an option and
press Enter, or type the answer in the box for a free-text ask; Esc declines
and the game takes its default. A notice (new mail, a page, an alert) prints
into the scrollback as `[category] text`, with the reply command in
parentheses; while the scene tail is off-screen the status line counts what
arrived (`! 3 new`, or a bell where the terminal draws one), and the count
clears once you scroll back to the tail.

## See also

  help getting-started, help mail, help channels, help intel, WHO
