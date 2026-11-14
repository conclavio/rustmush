# 47. The close-out — cut notes

One section a unit of [doc 47](47-close-out.md) §6. A unit's session replaces its own "Not built."
line with what it built, under these headings: **Survey**, **Built**, **Tests**, **Divergences**,
**Not built**. Each session edits only its own section, so two units' notes never touch the same
lines.

## CO.1

Not built.

## CO.2

Absent units answer fire under attack (R-47-2, R-47-3; doc 47 §3). Paths are under
`crates/tacsim/src/` unless given.

**Survey** (written before the cut).

- **Where `RETURN`'s "never fires" lives.** Not in the gate. `arena::absent::permits` has no
  arm for a posture: it lets an absent unit's shot through at a unit whose fire has arrived on
  it under every posture, and `GUARD`'s widening is the only posture-read input (`guards`).
  - **On the ground `RETURN` already answers** (MW.7's stories 2 and 3, AP.6's
    `ap6_a_seat_left_at_fire_hold_does_not_bind_the_mind`). What it does not do is keep going
    home: the go-home order is `Withdraw`, and the Standard Doctrine's `Withdraw` body
    (`arena/stock_mind.rs`, `withdraw_stand`) answers by a **cornered stand**: it stops
    (`Goal(Hold)` and a facing `Heading`), fires, dwells `STAND_TICKS` thinks, and stands
    again at the next think while the unit that fired is inside `stand_cm`. So a `RETURN` unit
    under fire stops its return until its attacker leaves range or it is hurt below
    `stand_min_permille`. The doctrine is the native stock mind, the oracle its Rhai twin is
    held to byte for byte, under `STOCK_VERSION` (`stock=3` in `MIND_ABI`): changing its body
    moves the ABI, every seal and the twin lane.
  - **In space `RETURN` never fires**, in three places: `space/backup.rs` `answers_of` answers
    `None` on `RETURN` (AP.9's default 2), so the seat is `FireRule::Never` and the view is
    not narrowed; `standing` reads her player's other hulls only off `RETURN`; and the house
    default (`content/minds/linkdead_return.rhai`) asks to answer only when
    `v.posture != "return"`. The applier (`space/applier.rs`) has no posture in it.
  - **A sealed mind in space never answers**: `fire_rule(_, sealed)` is `Never` for a seal
    (AP.9 default 1).
- **The think host and a second source.** `space/thinks.rs` collects one `Ask` a hull with
  one job **per source** (sealed mind, store house default, compiled house default), and the
  pool runs **every** job: `settle` keeps the first clean answer and drops the rest. So the
  house default's think on a sealed hull is already run on the same view and contacts; a
  second source for fire costs no think, only keeping that answer (one more `Option` on
  `Answer`) and landing its fire channel. Whose lock stands: a bubble's fire commands carry
  their target (`fire_offensive_lasers(shooter, target)`), so the answer can be fired at the
  house default's chosen ship without moving her lock: the sealed mind's sensor lock stands.
- **The ABI and the baselines.** The cut needs no getter, no intent and no doctrine change:
  the ground's rule is a kernel rule at the landing (the arena lands a house mind's stop as
  the drive home), and the space script already reads `v.posture`. **`MIND_ABI` stays
  `rhai=1.26.0;api=9;stock=3`.** The house default's source changes, so its digest does; it
  is pinned nowhere (AP.9 found the same). No gate-harness baseline can move: no unit of the
  harness or of a practice match nobody practises in is flown for an absent player, and every
  change here is behind that flag (the arena) or the backup seat (space).

**Built.**

Not yet.

## CO.3

Not built.

## CO.4

Not built.

## CO.5

Not built.

## CO.6

Not built.

## CO.7

Not built.

## CO.8

Not built.

## CO.9

Not built.

## CO.10

Not built.

## CO.11

Not built.
