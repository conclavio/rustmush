# Movement

Getting around the world.

  <exit>           Type an exit's name (or one of its aliases) to walk
                   through it: "north", "n", "out". Exits are matched
                   before commands, so an exit named "look" wins over the
                   look command — builders, name responsibly.
  goto <exit>      The explicit form of the same thing (alias: go).
  home             Return to your home room from wherever you are.
  look             Re-read your current room, its contents, and exits.

Exits are matched in this order: exits of your current room, then exits
of your room's zone, then global exits on the master room. An exit can be
locked (@lock) — failing its lock blocks the traversal, usually with a
message from the builder.

Staff and privileged movement:

  @teleport [<what>=]<where>   Jump directly to a room (or send an object
                   somewhere). Ordinary players can teleport themselves
                   only to rooms that are JUMP_OK or that they control;
                   the TEL_ANYWHERE / TEL_ANYTHING powers and Wizard rank
                   lift those limits. Destination @lock/teleport applies.

Related topics: "help building" (digging rooms and opening exits),
"help locks" (locking exits).
