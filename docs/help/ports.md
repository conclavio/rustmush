# Ports

The corridor runs between two ports. **Ceres Freeport** is the Belt's market:
the Co-op's Registry desk, the Exchange, the chandlery and the berths. **Stickney
Yards** sits on Phobos above Mars and is reached only by ship: the Yard builds
hulls, and Kasei Metals buys what the Belt ships in.

## Getting in and back

From Limbo the door `c` takes you to Port Watch Arrivals at Ceres Freeport. The
game remembers the port you last stood in when you step out to Limbo or the OOC
area. A port with no door from Limbo is one you can only reach by ship: the door
will not take you while that port is remembered, and `+ic` puts you back where
you left the corridor.

## Ceres Freeport

You walk it by its exit words. From Arrivals:

  registry      the Registry & Co-op desk, where you sign with the Co-op
  concourse     the Freeport Concourse, the public hall
  exchange      (from the concourse) the Freeport Exchange: metal concentrate
                and water ice for sale, and Freeport Works buying alloy
  chandlery     (from the Exchange) reaction mass, voyage provisions, and the
                Co-op buying the Yard's crated fittings
  arrivals      back to Arrivals from the desk or the concourse
  lobby         (from Arrivals) the Pressure Lobby
  gallery       (from the lobby) the Visiting Berth Gallery, where the Drays
                are cradled

## Stickney Yards

You arrive in the Visiting Berth Gallery. From there:

  lobby         the Pressure Lobby
  customs       the Customs Hall
  concourse     the Yard Concourse
  exchange      (from the concourse) the materials exchange: alloy stock,
                reaction mass, and the Yard's crated fittings
  down          (from the exchange) Kasei's Receiving Bay, where Kasei buys
                concentrate and ice
  commissary    (from the concourse) voyage provisions
  service       (from the concourse) the Yard's own counter
  observation   (from the concourse) the gallery over the drydocks

## The boards

Every trading room has a board, and `look` reads it. Each line is one house's
offer — the price, how much is left, the house and the clerk who signs for it —
or one house's bid: a wanted line, with the price it pays and how much more it
will take. `+market/list` in a port shows that port's listings, each with the
number you quote it by.

Stock is finite and every tonne of it is on record. A line counts down as it
sells; when a board says SOLD OUT, it is. A wanted line takes goods until its
intake is taken, and then the board reads it as not buying.

Money is credits, written like `5600.00cr`. `+credit` shows what is posted to
your account, what is held against deals you have accepted, and what you can
spend now.

See also: `help ships`, `help coop`, `help trade`, `help orders`.
