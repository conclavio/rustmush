# Getting Started

Welcome! This game is a MUSH: a shared text world where you play a
character, explore rooms, talk to other players, and (with permission)
build new places.

The basics, in the order you will want them:

  look             See the room you are in: its name and description, then
                   who and what is Here, the Exits, and the Weather outdoors.
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
"@tui" switches to an optional full-screen dashboard of numbered screens —
the scene with your scrollback, the room and who is about; your sheet;
mail; channels; rolls; intel; actions, and two tactical screens — a damage
report and a unit view — that show your unit whenever you hold a station on
one (they are always in the strip) — with single-key navigation and an
input box you open with Enter. "@notui" (or Ctrl-]) returns to the classic
line view, which stays the default. See "help tui". In either view the
server keeps the recent scene lines you already saw: in the line view
"@history" prints the last 20 oldest first ("@history 50" for more,
"@history more" to keep going back); in the dashboard the same history
loads above your scrollback when you scroll to the top.

In the web client, a bell collects notices — new mail, a page, an alert —
first as a brief toast, then in a tray you can open any time; a notice's
button opens the composer with the reply ready. Scroll to the top of the
scene and "Load older history" pulls the lines you already saw back from
the server's recent buffer.

The simulator is through the east corridor out of the Welcome Hall: two
sides fly stock tanks against each other on the proving ground, and none of
it is part of the story. Type "simulator" from the hall, or read "help
simulator" first.

Where to go next: "help movement" and "help communication" cover daily
life; "help chargen" builds your pilot; "help simulator" flies;
"help building" starts you on making rooms of your own; "help
rules-of-conduct" covers expectations for play.
