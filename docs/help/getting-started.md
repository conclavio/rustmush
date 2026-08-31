# Getting Started

Welcome! This game is a MUSH: a shared text world where you play a
character, explore rooms, talk to other players, and (with permission)
build new places.

The basics, in the order you will want them:

  look             See the room you are in: its description, who and what
                   is here, and the obvious exits.
  say <text>       Speak to the room. Shortcut: "<text>
  pose <action>    Act. "pose grins." shows as "YourName grins."
                   Shortcut: :<action>
  <exit name>      Type an exit's name (e.g. "north" or just "n") to walk
                   through it.
  home             Return to your home room from anywhere.
  WHO              See who is connected right now.
  page <who>=<msg> Send a private message to anyone online.
  QUIT             Disconnect cleanly.

Getting help:

  help             List every command and every prose topic.
  help <command>   Usage for one command (e.g. "help page").
  help <topic>     A prose topic like this one (e.g. "help movement",
                   "help communication", "help building").

A few conventions you will see everywhere:

  #123             Objects have numeric ids ("dbrefs"). You can usually
                   use either a name or a #dbref where a command wants a
                   target.
  *Name            A leading * looks a player up globally by name.
  me / here        Your own character / your current room.

Skip the password (SSH keys): register your SSH public key with
"@sshkey/add <the line from your .pub file>" and afterwards
`ssh -i <your key> <account>@game` logs you straight in — no password.
`ssh -i <your key> <account>+<character>@game` goes directly to one of your
characters. See "help sshkey".

A full-screen option (SSH): if you connect over SSH with a normal terminal,
"+tui" switches to an optional full-screen dashboard — your scrollback and
command line in the middle, a live WHO rail, and switchable @mail / +intel
widgets. "+notui" (or Ctrl-]) returns to the classic line view, which stays
the default. See "help tui".

Where to go next: "help movement" and "help communication" cover daily
life; "help building" starts you on making rooms of your own; "help
rules-of-conduct" covers expectations for play.
