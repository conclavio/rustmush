# Channels

Chat channels (the comsys) carry conversation across the whole game,
independent of rooms — the public channel for OOC chatter, staff
channels, plot channels, and so on. This is the TinyMUX-style surface.

Joining and speaking:

  addcom <alias>=<channel>   Join a channel and bind your personal alias
                             for it: addcom pub=Public
  <alias> <message>          Your alias becomes a verb: pub Hello all!
                             Pose flavor works too: pub :waves.
  delcom <alias>             Leave the channel bound to that alias.
  comlist                    List your channels, aliases, and status.
  comtitle <alias>=<title>   Set the title shown before your name on
                             that channel.

Channel output looks like: [Public] Nick: Hello all!

Muting and history:

  @channel/gag <alias>       Stop hearing a channel without leaving it
                             (you may not speak while gagged);
                             @channel/ungag resumes.
  @channel/recall <alias>[=<n>]
                             Replay the last <n> logged messages.
  @ignore <player>           Also silences that player's channel
                             traffic for you (see "help admin").

Administration (staff, or the COMSYS_ADMIN power):

  @channel/list              Every channel on the game.
  @channel/create <name>     Make a channel.
  @channel/destroy <name>    Remove one.
  @channel/set <name>=ic     Mark a channel in-character (a radio, a hive
                             mind, a newsfeed): the screen clients list it
                             apart from OOC chatter and set its lines as
                             prose. =!ic clears it. Every channel is OOC
                             until staff say otherwise.
  (Further @channel switches manage locks, colors, flags, and history —
  see "help @channel".)

Notes:

- Your alias is checked BEFORE exits and normal commands (typed lines
  only — "hello, :pose, and ;pose always stay room speech), and addcom
  warns you when an alias would shadow an exit or a command from where
  you are standing.
- Channels keep scrollback history; the web client shows channels as
  tabs with unread counters.
