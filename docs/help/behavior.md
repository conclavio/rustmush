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

## Destruction

Some behaviors **refuse `@destroy`** while attached — a structural host like a
simulation domain or an interior anchor shouldn't be torn down by a stray
command. If `@destroy` is refused, remove the behavior first (or use the relevant
recovery command). This veto is checked before anything is destroyed, and a
`@destroy/override` does **not** bypass it — the escape hatch is
`@behavior/remove`, not force.

## See also

  help building, help @behavior
