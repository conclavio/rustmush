# The Full-Screen TUI (`@tui`)

Most players use rustmush as a classic scrolling line client: you type a
command, the world answers, the text rolls up the screen. That line mode is
the **default** and is not going anywhere.

`@tui` is an optional, full-screen alternative for terminal clients — a
proof-of-concept dashboard rendered server-side over your SSH connection. It
splits the screen into panes: your normal scrollback and command line in the
middle, a live WHO rail down the side, and a switchable widget pane for
`@mail` and `+intel`. Everything you type still runs the same commands and
reaches the same world; the TUI is only a different way to *see* it.

## Launching and leaving

  @tui        Switch this connection into the full-screen TUI. Your terminal
              flips to its alternate screen and the dashboard paints.
  @notui      Leave the TUI and return to ordinary line mode. Your terminal's
              normal screen (and scrollback) comes back untouched.
  Ctrl-]      A hotkey that also detaches — the same as `@notui`.

Line mode is unaffected by any of this: a connection that never types `@tui`
behaves exactly as it always has, byte for byte. If your connection drops
while the TUI is open, the alternate screen is restored for you, so you are
never left stuck on a blank full-screen buffer.

You need a real interactive terminal (a PTY) for `@tui`; it does nothing over
a non-interactive pipe.

## The layout

  Main frame   The center: your scrollback (the same room, say, pose, page,
               channel, and system lines you see in line mode) with your
               command input line at the bottom. Type here exactly as you
               would in line mode; Enter submits the command to the world.
  WHO rail     A narrow column listing who is connected, with idle times. It
               refreshes on its own every few seconds — a read-only peek that
               does **not** touch your idle time (it never types `WHO`).
  Widget pane  A switchable pane for the `@mail` and `+intel` widgets (below).

## Moving around

  Enter        Submit the command on the input line to the world.
  Backspace    Erase the last character. Ctrl-U clears the whole line.
  PgUp / PgDn  Scroll the main-frame scrollback up / down a page.
  Up / Down    Scroll the scrollback one line.
  Tab          Cycle the widget pane: off -> @mail -> +intel -> off.
  Ctrl-]       Detach (return to line mode), same as `@notui`.

Password and hidden prompts (login, chargen) are honored on the TUI input
line just as in line mode — your typing is masked or not echoed as the game
asks.

## The widgets

Press **Tab** to open the widget pane and cycle through the widgets. Opening a
widget pulls its data fresh with a read-only query (it does not run a command,
and does not stamp your idle time):

  @mail    Your inbox: each message's number, an unread/urgent marker, the
           sender, and the subject. To actually *read* a message, submit the
           normal `@mail <n>` command on the input line — reading marks a
           message read, which is a real change, so it stays a command.
  +intel   Your own informant intel, newest first: the subject, the kind of
           event, and a short detail. You only ever see intel filed for you —
           there is no way to see another player's (see `help intel`).

On a narrow terminal the widget pane and WHO rail drop out automatically so
the main frame always has room; widen the window to bring them back.

## Status: proof of concept

The TUI is deliberately a first cut. The plumbing is real — it is a genuine
third renderer of the same event stream that drives line mode and the web
client — but the layout, styling, key bindings, and widget set are expected to
grow and change. Line mode remains the supported default; treat `@tui` as a
preview.

## See also

  help getting-started, help mail, help intel, WHO
