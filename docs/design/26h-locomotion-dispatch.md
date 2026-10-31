# DT.2b2b.3b — Captured ordinary command admission

> **Removed 2026-10-31 (owner ruling).** The code this document describes was
> deleted by PR #PRNUM. The last `main` commit that has it is `61a670ce`; recover a
> file with `git show 61a670ce:<path>`. It was superseded by the owner's 2026-10-29
> correction: [doc 27 FO-16](27-ic-field-operations-design-and-survey.md) requires
> current containment at preparation and launch, with no embarkation intervals or
> movement-history certification. The text below is history, not a plan.

The bounded route proof in [26g](26g-locomotion-admission.md) does not establish
that a character's command reached that route. This slice adds an explicit Core
capture and preparation seam. Normal player input still uses ordinary movement;
no production provider or Sim route opts into this seam yet.

## Actual dispatch provenance

The capture path shares the real dispatcher. Input contexts, prefixes, command
gates, aliases, HOME, whole-line exits and command-table lookup retain their
order. It intercepts a selected movement before relocation or success output.
Aliases run once. Every line that actually passed the command gates is retained
within fixed limits; revalidation checks those lines without replaying aliases.
The transcript allows 32 passed lines and 32 KiB in aggregate, with the existing
16 KiB per-line ingress limit. Exceeding a limit cannot be repaired by a later
short rewrite. Captured exit requests retain the 512-byte bound in 26g; ordinary
dispatch does not acquire this new capture limit. A capture error refuses the
intercepted attempt instead of falling back to movement or another handler.

Only the actual built-in movement entries can supply command-table captures.
A custom command named `goto`, an alias with that name or a similar handler is
not a built-in movement capability. Bare HOME and whole-line exit precedence
remain significant. Revalidation refuses a changed selected branch, including
a new whole-line exit that shadows a previously selected `goto` command.

Capture is not a dry run: an input context, alias or nonmovement handler can
perform its ordinary effects. Those effects retain their existing persistence
contract. Capture refuses prelude mutations or queued deferred work, then drains
that work through the normal Engine path without attempting rollback. Staged
authoritative work also refuses a mixed legacy batch. The explicit Engine method
neither queues a movement action nor announces one.

## Preparation and custody

The opaque capture has no public constructor, clone or serialized form. The
Engine checks its registry identity and actual actor before passing its request
into a registered provider's `PrepareCtx`. The provider must call
`admit_captured_locomotion`; this method accepts no replacement actor, route or
destination. Missing capture, repeated admission, ignored errors and AlreadyHome
refuse the preparation. AlreadyHome has no moving admission to commit.

Core retains both the dispatch capture and the derived admission in the prepared
composition. The provider cannot strip either while keeping its writes. The
observation binds the actual invocation, action, idempotency identity and commit
token, and staging checks that binding against the complete bundle.

Staging and final writer handoff rerun the saved command gates with the actual
Registry, revalidate the route and lock observations, and consult current
MoveAttempt hooks with the live owner-scoped typed lens. Provider and movement
hook owner readiness is checked after callbacks. Refusal aborts the staged
reservation. Installation after acknowledgement checks bounded local preimages
without invoking command gates, aliases, providers or movement hooks again.

## Remaining movement work

This seam supplies no object postimage, Sim placement mutation, crossing receipt
or personnel eligibility. It does not migrate production walking or HOME.
Live routing still needs the ordinary Sim participant and durable, owner-prepared
movement completion for narrative output and mutating Moved observers. Atlas
window placement and script-triggered movement must be joined or refused before
mutation. Legacy callbacks after an acknowledgement cannot establish that
contract, and restart must not infer historical embarkation.

## Verification

The Core suite passes 835 unit tests, 240 integration tests and 13 doctests.
Twenty-nine new tests exercise actual capture, provider preparation, staging,
final handoff and installation with an explicit fixture marker as the only
write. They cover aliases and contexts, custom handler identity, transcript
bounds, hidden deferred persistence, stale gates/hooks/routes, exact bundle
binding and callback-free installation. The 78 Sim unit tests, 17 production DT
integration tests and 32 server composition tests pass. Core Clippy passes for
all targets with warnings denied, alongside formatting and source-policy checks.
