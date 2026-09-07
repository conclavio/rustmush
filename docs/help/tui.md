# The Full-Screen TUI (`@tui`)

Most players use rustmush as a classic scrolling line client: you type a
command, the world answers, the text rolls up the screen. That line mode is
the **default** and is not going anywhere.

`@tui` is an optional full-screen alternative for terminal clients, rendered
server-side over your SSH connection. It is built from **numbered screens**
of titled panes: a scene screen with your scrollback, the room and who is
about; a sheet screen; mail; channels; rolls; intel; actions; and two
tactical screens — a damage report and a unit view — that show your unit
whenever you hold a station on one (they are always in the strip).
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
the key acts.) F1–F9 also switch screens from either mode.

## Keys

Nav mode (the box is closed):

  1-9          Switch to that screen (the tab strip along the top).
  Enter        Open the input box. If the focused pane has a highlighted row
               — a mail message, an action, a channel — the box opens with
               that row's command ready (`@mail 3`, `+act brace`, `pub `).
  " : ; \      Open the box with that speech trigger already typed.
  /            Open the box with the command palette (below).
  Tab / Shift-Tab   Move focus between panes.
  Up / Down    Scroll the focused pane, or move its highlight.
  PgUp / PgDn  Scroll a page.
  g / G        Jump to the top / bottom of the focused pane.
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
  Alt+key      The key's nav action (Alt+1 … Alt+9, Alt+? help, Alt+letter).

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
slash: `/clear` empties the scrollback, `/help` opens the key help,
`/notui` detaches.

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
             count. The highlighted channel's recent lines fill the right;
             `b` asks for more backlog. Enter on a channel opens the box with
             its alias. Ctrl-N walks the focused list, Ctrl-O swaps lists.
  5 rolls    Your vitals as widgets on the left — tracks as boxes in threes
             with the level named, pools as gauges, readouts with a condition
             mark — and the dice you have rolled on the right. Ctrl-R opens
             the box with `+roll ` ready; Tab then completes a trait from your
             sheet (`+roll str` → `+roll strength`, `+br` → `+brawl`).
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

The two tactical screens are always in the tab strip, and they are honest
when there is nothing to show: with no station and no unit of your own they
read `No tactical unit: take a station, or @unit/show #n if you control
one.` A damage report that reaches you about a unit you hold no station on
— staff damaging someone else's — still fills the journal and the
before/after table, with each line naming the unit it is about. Screen 9's
contacts pane is the table `+contacts` prints, when the server publishes it
(Enter on a row prefills `+contact <id>`); it also carries panes for things
no server publishes yet — a grid, weapons, comms — each saying what it is
waiting for. Those are the first panes to go when the window narrows, so a
real pane never loses its place to an empty one. Colour and condition marks
on both screens come from the ruleset's own judgement, never from the
client's reading of a word.

Above the input box, when your ruleset publishes vitals, a strip of gauges
shows your health track, pools and readouts; without one the prompt shows
as in line mode. The WHO rail drops out below 100 columns and the chrome
below 12 rows, so a small window always keeps the scrollback and the box.

Password and hidden prompts (login, chargen) open the box on their own and
mask or hide your typing, just as in line mode.

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
