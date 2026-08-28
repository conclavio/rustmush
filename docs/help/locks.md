# Locks

Locks control who may do what with an object. Every object can carry one
lock per lock type; someone "passes" a lock when its expression evaluates
true for them.

Setting and clearing:

  @lock <object>=<expression>          Set the basic lock.
  @lock/<type> <object>=<expression>   Set a specific lock type.
  @unlock <object>  /  @unlock/<type> <object>

Lock types:

  basic      The default: who may pick the thing up / pass the exit.
  enter      Who may enter (ENTER_OK things, vehicles).
  use        Who may use the object.
  page       Who may page you (set it on yourself).
  teleport   Who may teleport into the room.
  speech     Who may speak in the room.
  zone       On a Zone Master Object: passing grants control over the
             zone's members (delegated building administration).
  drop       Who may drop the object.
  give       Who may give the object away / give to this object.
  mail       Who may @mail you.

Expression syntax (combinable with parentheses):

  me, *Player, #123   That exact object passes.
  +#123               Passing requires CARRYING object #123 (a key).
  $#123               Anything OWNED by #123 passes.
  flag^WIZARD         Anyone with that flag (or module marker flag).
  attr:glob           Attribute match: sex:m*le passes anyone whose sex
                      attribute glob-matches "m*le".
  @#123/enter         Indirect: evaluate that object's enter lock.
  !expr               Negation.
  a & b               Both must pass.
  a | b               Either passes.
  #TRUE / #FALSE      Always pass / always fail.

Examples:

  @lock north=flag^WIZARD | +#42     Wizards, or anyone carrying #42.
  @lock/page me=!*Spammer            Everyone but Spammer may page you.
  @lock/speech here=$#10             Only #10's objects may speak here.

Wizards bypass most lock failures where the surrounding command says so
(e.g. teleport locks). Locks never hide the object — see DARK and
@lock-adjacent flags under "help building".
