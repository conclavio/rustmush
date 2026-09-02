# Object behaviors

A **behavior** is typed, module-defined capability attached to an ordinary
object. A plain crate becomes a `btech.mech`; a room becomes a `sim.domain`.
The behavior is what tells a module "this object is one of mine, treat it
accordingly" — so simulation, vehicles, and other systems ride on ordinary
objects instead of a separate object type.

The binding on the object is deliberately lightweight: it records *that* the
behavior is present, who attached it, and when. The heavy state a behavior needs
lives in the owning module's own storage, not on the object. Removing a behavior
drops the binding.

## Attaching and removing (builders)

  @behavior/add <object>=<id>       Attach a behavior you're allowed to.
  @behavior/remove <object>=<id>    Detach it.
  @behavior/list <object>           Show what an object carries.

You must **control** the object, and each behavior may demand more: some require
a higher rank than Builder, and some may only attach to certain kinds of object
(a map behavior to a room, say). Behaviors can also **conflict** — two that can't
sensibly coexist on the same object are refused. A raw `@set` never attaches a
behavior; the lifecycle-aware commands above are the only way, so the module
always gets to validate.

Behavior ids are namespaced, lowercase, dotted: `sim.domain`, `btech.mech`,
`shadowrun.vehicle`. Exactly one module owns each id.

## Contextual commands

A behavior can contribute **commands that exist only near the object**. A
console offers `startup`; a mech offers `heading` and `fire`. These verbs are
never global — type `startup` with no console reachable and you get the usual
`Huh?` — so a behavior can never shadow a core command, a staff command, or an
exit; those always win.

What counts as "reachable", in priority order:

  1. an object you're **operating** through an assignment (a pilot's mech,
     a rigger's drone), even if it's elsewhere;
  2. yourself;
  3. the thing you're **inside** (a cockpit), when it isn't a room;
  4. things you **carry**;
  5. the **room** you're in — only for commands that opt in, such as a map's
     `view`.

The nearest tier that answers wins. If two objects at the same tier both offer
the verb, the game refuses and names them rather than guessing — drop or set
down one, and it resolves. Verbs must be typed exactly (no abbreviations at this
stage); behaviors declare short aliases where pilots need them. Every such
command is gated like a normal one — switches, rank, powers, locks, and turn
order inside a timestop all apply.

## Destruction

Some behaviors **refuse `@destroy`** while attached — a structural host like a
simulation domain or an interior anchor shouldn't be torn down by a stray
command. If `@destroy` is refused, remove the behavior first (or use the relevant
recovery command). This veto is checked before anything is destroyed, and a
`@destroy/override` does **not** bypass it — the escape hatch is
`@behavior/remove`, not force.

## See also

  help building, help @behavior
