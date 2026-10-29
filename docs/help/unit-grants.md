# Unit permissions

Unit grants lend one permission at a time. A personal title holder can lend a
unit directly; organization authority must cover the unit and allow delegation.
Owning its carrier or storing a unit aboard does not grant control of it.

Use `@unitgrant/issue #unit=#character <capability>` to lend a permission.
Capabilities are `operate_crew`, `deploy`, `command_assigned_ai`, `recover_load`,
`maintain_refit`, and `transfer_abandon_title`. Each needs its own grant.
Add `delegate` to permit onward lending, `until=<UTC seconds>` to set an expiry,
or `parent=<grant>` to lend through a permission you received. A child cannot
outlast or broaden its parent. Without a parent selection, the command requires
your current personal title.

`@unitgrant/check #unit=<capability>` checks your current permission. Add
`delegate` to check onward authority. `@unitgrant/list` shows grants you issued
or received directly; its continuation line pages through longer lists.
`@unitgrant/revoke <grant>` revokes a grant you issued and invalidates dependent
grants for new admissions. It does not eject an occupant or undo a delivered
order. Title leaving and returning does not revive a previous loan.

The `return-only` issue option records consent for an admitted custodian to stop,
disengage and return to the recorded origin. It grants no firing exception and
does not itself establish physical custody. A delegated grant must inherit that
consent; an organization source must explicitly authorize recover/load.

`@unitgrant/json <draft>` accepts the explicit organization/scope form documented
in the unit-authority design. Current mission-leader grants require an accepted
mission owner; the practice simulator does not provide that authority.

These commands record permissions. Seating, deployment, preparation and mission
orders use their own admission checks. Wait for the acknowledged action result
before treating a new grant or revocation as committed.
