# The Co-op's First Captain programme

The Belt Carriers' Cooperative stakes new captains. Sign with it and you fly a
Co-op Dray on a working charter, with working cash to fill the first hold; the
Co-op takes a share of your trading margin until you buy the hull out. `+coop`
reads the terms and how many Drays are free, from anywhere.

## The terms

`+coop` shows them as they stand on this world:

  the stake       the working cash the Co-op puts in your account
  the share       the part of your trading margin over the Co-op's reference
                  prices that goes to the Co-op on every sale
  the buyout      the price at which the hull becomes yours, fixed on your
                  papers when you sign
  once, ever      one enrolment per account, ever: no other character on
                  your account can enrol after you

## Signing

`+coop/enrol <name>` at the Registry & Co-op desk in Ceres Freeport (`registry`
from Arrivals). In one act Ceres Clearing opens a credit account in your name,
the Co-op charters you its next free Dray under the name you give it (its
registry mark stays), stakes you, and opens the ship's hold in your name. A name
is 1 to 32 letters, digits, spaces, hyphens, apostrophes or full stops, starting
with a letter, and no two Drays share one.

While the programme is still being set up on a world, the desk answers that the
Co-op is not enrolling yet; try again later.

## The charter

The ship stays the Co-op's while you hold the charter. You fly it, load it, sell
from it and file its sailing plan; nobody else can board it on your charter.
`+coop/statement` shows your charter's terms, its sales so far, the Co-op's
share of them, and your cash against the buyout.

A charter ends in one of three ways: you buy the hull out, you hand it back, or
you go a week without a departure and the Co-op takes it back. Whichever way it
ends, the Co-op enrols each pilot once, ever: you cannot enrol again.

## Buying the hull out

`+coop/buyout` at the Registry & Co-op desk, with your Dray docked at Ceres
Freeport. The price is the buyout on your papers, fixed when you signed; you need
that much available in your account. Nothing may be in motion: no clearance
filed, no step of your sailing plan at the counter, and no goods still moving in
or out of the hold. The Registry holds the price against the transfer and closes
the Co-op's charter; the title passes to you at the next settlement, a few moments
later, and the Registry tells you when it does. Until then a step of your sailing
plan that comes due waits, and runs once the title is yours.

From then on the Dray is yours. You fly her on your own title, and the Co-op's
share stops: your sales pay the Co-op nothing. `+coop` and `+ship` say so.

## The yard's refit

`+coop/refit prospector` at the Registry & Co-op desk, with your Dray docked at
Ceres Freeport, her hold empty but for her tank, and nothing filed or at the
counter: the Co-op's yard refits her as a Dray-P. She keeps her name, her mark,
her charter and her papers; she gains a two-rig vehicle bay aft of the hold, and
her hold drops to 86 t and 103.2 m³, which takes bulk and ore. Provisions count
as cargo here: draw the locker down first. `+coop/refit standard` seals the bay
and gives her back her 100 t and 120 m³, once the bay is empty. `help ships` says
what the bay is for.

## Handing the hull back

`+coop/release` at the desk, with your Dray docked at Ceres Freeport and her cargo
hold empty, and nothing filed or at the counter. You hand her papers back and the
charter ends. The stake stays yours. The reaction mass and stores aboard stay with
her, and anyone aboard is seen ashore to the berth gallery. A Dray-P goes back
with an empty bay: return any rig to Occator first. The buyout waits for an empty
bay too. If the Co-op takes a Dray-P back (a week without a departure, or staff),
Occator takes its rigs back out of the bay first: a rig within its paid term
returns your deposit, one past it keeps it.

## A week without a departure

A charter is for working the lane. If your Dray goes a week of real time without a
departure, the Co-op takes her back: when she is docked at Ceres Freeport with an
empty cargo hold, her charter ends, anyone aboard is seen ashore, and her
reaction mass and stores stay with her. If she is anywhere else, or still carries
cargo, the Co-op stops her sailing plan instead, and it stays stopped until you
restart it with `+ship/order resume`; she is taken back once she lies docked at
Ceres with an empty cargo hold while the week is still unbroken. Any departure,
by hand or from your plan, resets the week. The stake stays yours, and you are
told the next time you are on watch.

## What the share is, and is not

The share is taken from **margin**, not from profit: for each sale, the price
you got over the Co-op's reference price for that good, times the tonnes. Sell
at or under the reference price and the Co-op takes nothing. Your fuel, your
stores and your buying costs are yours alone; the share does not come off them.
The share is taken inside the sale: the sale line names it, and you receive the
rest when the goods are in.

See also: `help trade`, `help ships`, `help ports`.
