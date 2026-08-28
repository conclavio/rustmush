# Scripting

Objects can carry small scripts written in **Rhai** (a pure-Rust scripting
language). Scripts let a builder give an object behaviour — a greeting, a
room that reacts to arrivals, a lever that does something when used — without
any softcode interpreter. They run sandboxed inside the game, with strict
time and memory budgets, so a runaway script can never stall the world.

A script is just an attribute whose name starts with `script:`. Set one the
usual way, with `@set`:

  @set <object>/script:command:<verb>=<rhai code>
  @set <object>/script:enter=<rhai code>
  @set <object>/script:leave=<rhai code>
  @set <object>/script:hear:<name>=<rhai code>
  @set <object>/script:use=<rhai code>
  @set <object>/script:timer:<name>=<rhai code>

Editing a `script:*` attribute recompiles it immediately; a compile error
disables just that one script and tells you what was wrong.

Hooks (when a script runs):

  script:command:<verb>   Someone in the same room (or holding the object)
                          types <verb>. The text after the verb is `arg`.
                          This is a FALLBACK: it never shadows a built-in
                          command or an exit.
  script:enter/leave      Someone moves into (or out of) the room the script
                          lives on (or a thing in it). The mover is `mover`.
  script:hear:<name>      Someone speaks in the room; the line is `text`.
  script:use              Someone types `use <object>`. The user is `user`.
  script:timer:<name>     Fired by mush::schedule() from another script.

The mush API (every call is permission-checked as the object's OWNER, so a
script can never do what its owner could not):

  mush::me()              the scripted object          mush::here()  its room
  mush::caller()          who triggered the script     mush::owner() its owner
  mush::name(obj)         an object's name             mush::loc(obj)
  mush::get_attr(obj,a)   mush::set_attr(obj,a,val)
  mush::emit(target,text) mush::pemit(player,text)
  mush::move(obj,dest)    mush::matchit("name")        mush::contents(obj)
  mush::schedule(secs,"<timer-name>")  mush::dice("2d6+1")  mush::has_flag(obj,"FLAG")

There is deliberately no way to run an arbitrary command or evaluate code
from a script — scripts act only through the typed mush API above.

Reads honor visibility: mush::loc, mush::contents, and mush::has_flag hide
DARK/OPAQUE objects the owner could not see by looking, and mush::get_attr
withholds staff-only and HIDDEN attributes — a script is never a scry tool.

mush::schedule timers live in memory only and are NOT guaranteed to survive a
server restart; treat them as best-effort.

Budgets and HALTED:

  Each run is capped (operations, call depth, string/array/map size, and a
  wall-clock backstop), and one originating event's whole cascade of reflexes
  shares an aggregate wall-clock budget too. A script that overruns is aborted
  and its owner is told. After three *consecutive* budget overruns (a clean run
  resets the count) the object is auto-HALTED: all of its scripts stop running
  until you fix the offending script and clear the HALTED flag
  (`@set <object>=!HALTED`).

Example — a doorman that greets by name:

  @create doorman
  @set doorman/script:command:greet=mush::pemit(mush::caller(), "Welcome, " + mush::name(mush::caller()) + "!")
  drop doorman

Now anyone in the room can type `greet` and be welcomed.
