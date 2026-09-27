# Macros

A macro is a word of your own that stands for a line you type often. Name
it once, and from then on typing the word types the line. The server
expands it before it looks for a command, so the same macro works from the
line client, the TUI and the web client.

  @macro <name>=<line>   Make a macro (or replace one you already have).
  @macro                 List yours.
  @macro/del <name>      Delete one.

    @macro lk=lock $1
    @macro fc=fire cannon
    @macro hey=say hey there, $1

Arguments: `$1`, `$2` and `$3` in the line are the first, second and third
word you type after the macro's name, and the last one the line uses takes
the rest of what you typed, so nothing you type is lost: after the third
example, `hey Bly, good to see you` says `hey there, Bly, good to see you`.
A word you do not type is empty: `lk` alone is `lock`, which the cockpit
answers as it would had you typed it. A macro whose line uses none of
`$1`, `$2` and `$3` takes nothing after its name, and tells you so. `$$`
puts a plain `$` in the line.

The rules:

  - A name is 1 to 16 letters, digits, `-`, `_` or `.`, and case does not
    matter: `LK` and `lk` are the same macro.
  - A name cannot be a word that already means something when you type it:
    a command's name or one of its short forms (`l` is `look`'s, which is
    why the example is `lk`), `home`, `quit`, `respond`, or one of your
    channel aliases. It can be a word that only shortens a command (`f` is
    not `fire`'s name, so `@macro f=fire cannon` is allowed, and the macro
    wins for you). It can be an exit's name too; then typing just the name
    takes the exit, as the exit keys and buttons do, and the macro runs
    when you type words after it (so such a macro wants a `$1`).
  - A macro's line starts with a word: `say`, `pose`, `semipose` or
    `@emit`, not the `"`, `:`, `;` or `\` shorthand for them. That first
    word is written out, with no `$` in it; the words after it can use them.
  - A macro's line cannot be `@wait`, an account command, or another of
    your macros: a macro is expanded once, and never expands into another.
  - Up to 20 macros per character, each line up to 200 characters.

A macro runs as you, with exactly what you may do: it cannot reach a
command you could not type yourself, and a refusal reads as it would had
you typed the line. It is kept on your character, so it is there after a
reconnect and after the server restarts, and each of your characters has
its own.

Two things to know. A command or channel alias that arrives after you made
a macro of the same name is hidden by your macro until you delete it;
`@macro` marks such a row. And in the TUI or the web client with a sticky
say, pose, emit or ooc mode set, a line is sent as speech, so type a macro
with the mode set to none.

Lines the game never sees cannot be macro lines: `@tui`, `@notui`, `QUIT`,
`@history`, the `+act` family and `respond` are answered by your client or
the connection before the game reads them.

See also: `help cockpit`, `help channels`, `help tui`, `help getting-started`.
