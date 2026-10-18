# Building

Making new places and things. Building commands generally require the
BUILDER flag (ask staff) — this topic is still worth reading so you know
what the builders around you are doing.

Rooms and exits:

  @dig <room name>[=<exit to it>[;alias...][,<exit back>[;alias...]]]
                    Create a new room, optionally opening an exit from
                    where you stand and a return exit in one stroke:
                    @dig Hall = north;n , south;s
  @open <exit name>[;alias...][=<destination>]
                    Open a new exit in your current room, optionally
                    linked at once. An exit's name or alias may not be
                    a word that is a command in every room (look, l,
                    inv, home ...): the exit would swallow it, so @dig,
                    @open and @name refuse it and say which command.
  @link <exit>=<destination>
                    Link (or relink) an exit to a room; also sets things'
                    homes: @link me=here sets your home.

Objects:

  @create <name>    Create a thing in your inventory.
  @name <object>=<new name>
                    Rename something you control. Exits take
                    semicolon-alias names: @name north=north;n
  @desc <object>=<text>
                    The description shown when someone looks at it.
                    @desc here=... describes the room.
  @set <object>=[!]<flag>
                    Set or clear a flag (DARK, STICKY, ENTER_OK, ...).
  examine <object>  Full technical view of anything you control.
  @destroy <object> Destroy something you control (SAFE objects need
                    /override).
  @find <text>      Find your objects by name.

Organization:

  @parent <object>=<prototype>   Inherit attributes from a prototype.
  @chown <object>=<player>       Give an object away (or claim, with the
                                 CHOWN_ANY power).
  @zone <object>=<zone master>   Group rooms/objects under a Zone Master
                                 Object for shared administration.

Every object you make counts against you: keep names sensible (exits
that shadow common commands like "look" will shadow them for everyone in
the room). See "help locks" for controlling who may use what.
