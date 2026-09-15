# Communication

Talking to the people around you — and the ones who aren't.

In your room:

  say <text>        Speak aloud. Shortcut: "<text>
  pose <action>     Emote with your name prefixed: :grins.
  semipose <action> Emote with no space after your name: ;'s eyes narrow.
  @emit <text>      Free-form narration with no name prefix. Needs the
                    SPOOF flag (Royalty and Wizards have it); ask staff.
                    Shortcut: \<text>
  whisper <who>=<message>
                    Quiet words to one person in the same room.
  think <text>      Echo text back to yourself only.

Anywhere on the game:

  page <player>=<message>
                    A private line to any connected player (alias: p).
                    A player set HAVEN, or whose page lock you fail,
                    cannot be paged.

Everything you say, pose, whisper or page carries your name, and a long
line wraps under it, so nobody can make a line look like it came from
someone else or from the game. @emit is the one exception, which is why it
is a granted flag. Below Wizard, an emit still travels with its true sender:
set NOSPOOF on yourself (@set me=NOSPOOF) to see it as "[Name] text".
Royalty and Wizards always see the source.

Channels (the comsys) are the place for out-of-character chat across the
whole game — see "help channels".

Muting someone:

  @ignore <player>  Silently drop that player's pages and whispers to
                    you, filter their say/pose/emit out of what you hear
                    in a room, and mute their channel messages and
                    join/leave notices for you. They are NOT told they
                    are ignored — they see normal delivery
                    confirmations. Staff cannot be ignored.
                    @ignore/list shows your mutes, @ignore/clear
                    <player> lifts one, bare @ignore/clear lifts them
                    all. (Mail has its own gate: @lock/mail.)

Rooms can be locked against speech (@lock/speech); failing that lock
means you cannot speak in that room at all.

## Typing indicator

Players using the web client see a small "is typing" pill in the room band
while someone in the room composes a line, and NPCs show a "thinking" pill
while they decide what to do. The hint is best-effort: it is never shown for
a DARK character, it never counts as activity for idle time, and only people
who could perceive you in the room receive it. To keep your own typing
private, opt out with:

  @set me/pref:composing=off

Set it to anything else (or clear it) to opt back in.
