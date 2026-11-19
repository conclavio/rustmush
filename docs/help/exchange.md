# Selling and bidding on the exchange

Any player may sell goods they hold, or post a bid for goods they want, on the
market board of the venue they trade in. Nobody needs to be online for a deal:
a listing is your standing agreement to sell on its terms, and a bid is your
standing agreement to buy on its terms, so trades settle while you are away.

The board is local. A listing or a bid stands in one venue (`market=<venue>`,
or the venue of the room you stand in), and goods change hands where they are.

## Listing a lot

You sell one exact lot at a time, from a vault you hold. Split it to the size
you want to sell first:

`@inv/split <lot>=<n>` leaves a lot of n units beside the rest.

`+market/post <lot> price=<n> [to=<player>] [market=<venue>] [days=<n>]`

lists the lot at a price per unit, in minor units of the venue's currency (a
price of 700 is 7.00cr). From that moment the lot is held for the listing, so
you cannot also use it or list it twice. A listing stands for 14 days unless
you name `days=`, and up to 90.

A buyer quotes it with `+market/quote`, and takes it up with `+market/accept`
into a vault of their own. A lot is sold whole: a quote names all of it, and
`+market/quote <listing>` with no quantity does. The buyer's payment is held
first, the lot itself moves to them, and you are paid once it has moved. If it
never moves, the buyer's money comes back within the hour and the lot stays
yours. You do not need to be there.

## Listing a hull

`+market/post #<hull> price=<n> [to=<player>] [market=<venue>] [days=<n>]`

lists a hull whose title you hold, at one price for the whole of her. Her hold
must be empty (her fuel and stores go with her), she must be out of the bay,
with nothing in flight and no leased rig aboard. A hull with a governed core
fitted cannot be listed yet: the core sells only through its own sale, which
is not open.

While she is listed, and while a purchase of her is settling, the bay will not
strip her, scrap her or fit anything to her. A buyer is quoted on her as she
stands; if she changes after that, the quote no longer stands, and a purchase
made on it does not settle, and its price goes back to the buyer a day later.
When the sale settles her title passes to the buyer, and her hold goes with
her.

## Listing a facility

`+market/post #<facility> price=<n> [to=<player>] [market=<venue>] [days=<n>] [as=<org>]`

lists a port's refinery, works or yard whose title you hold, or with `as=<org>`
one an organization you act for holds; the organization is paid. It is sold only
when idle: no order open there and nothing in its stores. The relay foundry,
the workshops and the medical rooms are not for sale, nor is a facility whose
stores a vendor trades from.

When the sale settles, the facility passes to the buyer with its stores, and
its takings are paid to the buyer from then on. The buyer sets who may order
there and its margin with `@facility/terms <facility>=public|owner
[margin=<permille>]`. Its running costs stay as it was built. A sold yard
stays open to every customer.

A fee is fixed when an order is booked. If the owner raises it before the work
starts, the order is canceled, and its booking fee and bill go back to the
customer.

## Posting a bid

`+market/bid <ruleset>:<code> qty=<n> price=<n> into=<vault> [to=<player>] [market=<venue>] [days=<n>]`

posts a bid for a good (a port good reads `tacsim:<good>`, as `tacsim:metals`).
The whole price, quantity times price, is held from your account at once, so
every seller can see the bid is good. Sellers fill it with `+market/sell
<listing>=<qty>`; each sale draws its share of the hold and the goods land in
the vault you named. You hold one bid for a good at a time.

Bulk goods are poured into a ship's hold only from its port's own counters, so
a bid for bulk goods receives into a vault, not a hold.

## Private offers

Add `to=<player>` to a listing or a bid to make it a private offer. Only that
player can see it, quote it or take it up (or, for a bid, sell into it); to
everyone else it does not exist. A private offer is how two players trade
something between themselves at an agreed price.

## Taking it down

`+market/withdraw <listing>` takes one of your listings or bids down. A listed
lot is yours to use again; a bid gives back whatever it still holds. A bid also
gives its hold back on its own when it lapses.

`+market/mine` lists your listings and bids, and how many more you may post,
then what you listed for each organization you act for.

## Limits

You may hold at most 32 listings and bids at once, and the whole board holds
at most 1,024. When the board is full, try later.

## Where your goods go

A purchase at a port goes into the hold of a hull of yours that is docked
there. It need not be your first hull, only one whose hold you may trade from.

With no hull of yours docked there, it goes into your locker at that market.
`+locker/open` opens one where you stand, and `+locker` shows what it holds and
owes. A locker holds 20 t and 40 m3, and costs 1 cr an IC day for each tonne
it holds, taken from your own account. A day you cannot pay is owed as arrears,
and paid once your account is funded.

`@inv/move <vault>/<good>:<n>=<vault>` moves goods between your own stores
where you stand in a port: the hold of a hull of yours docked there, and your
locker there. The goods must fit where they go. Cargo in a chartered hold stays
aboard, since it is sold from there and the Co-op's share is paid on it. A hull
you have listed for sale takes nothing more into her hold.

A market that belongs to no port, such as a counter, trades only with people
standing at it.

## Chartered cargo

Goods in a Co-op chartered hold may go on a personal ask. When they sell, the
Co-op takes its share of the margin over its reference price, as it does on a
sale into a house's bid, and you are paid the rest. The pilot lists them as
themself, never `as=<org>`.

## Fees

At a port, the port's authority takes a listing fee of 1% of what you list (at
least 10 cr), once, when you post it, and 3% of each sale. A further 1% of each
sale is a levy to the port's treasury. A sale into a player's bid pays the same
3% and 1%, out of what the seller receives. A house's own listings pay none of
these.

## Berths

A hull docked at a port with an authority owes it 5 cr an IC day, from her
title holder's account. A day you cannot pay is owed as arrears, and paid once
your account is funded; a long absence stops adding to it after about ten real
days. Arrears never keep a hull at her berth, but they are paid out of her next
sale at that port. A house's own hulls at their home port owe nothing.

## The job board

  +market/jobs [retrieval|passage|survey|haul|escort] [market=<venue>]

Lists the jobs posted at the port you stand in, or at the market you name.
Each job says what it pays, any bond you would escrow, when it is due, and the
command that takes it:

- **Retrieval:** bring a hull to a port for its title holder. Take it with
  +salvage/accept. You then have the retrieval window to deliver her.
- **Passage:** carry a waiting traveler to their destination. The fare shown is
  their opening offer; +passage/offer opens the haggling. The fare is paid on
  arrival.
- **Survey:** Occator's commissions for fresh soundings of one block. Take one
  with +survey/accept; you have 28 IC days from accepting.
- **Crew:** a player offering to crew for a wage, while they stand in the port.
  The pay shown is the scale wage they ask, by the IC day; page them to agree
  terms. List yourself with +sortie/available (see help detachments).
- **Haul:** carry a shipper's goods from their locker here to their locker at
  another port. Take one with +haul/accept while your hull is docked here; you
  have 28 IC days from accepting.
- **Escort:** stay near a hull through her passage or her field sortie. Take
  one with +escort/accept and the hull or rig you escort with.

A job paying more than 10,000 cr asks for a bond of a fifth of its pay. The bond
is escrowed from your account when you take the job. It comes back with the pay
when the work is done, and goes to the poster if the work is not done in time.

To take a retrieval for your organization, add as=<org>:
+salvage/accept <job> as=<org> go. You need the right to spend the
organization's money. The pay goes to its account, and its bond comes from
there.

## House prices

A house's ask and bid move with its stock, once an IC week (seven real hours).
An ask whose storefront runs short rises, and one that holds more than its
usual stock falls. A bid falls as it fills, and rises again when the house
wants more. Each step is at most 5% of the price, and a price stays between half
and twice the house's authored price. A bid's reach premium, for a port that is
far from its suppliers, is added on top.

A house that has gone into debt raises its asks 10% an IC week, whatever its
stock, and stops posting bids until it is solvent again.

In real time, a price moves at most about 22% in a real day (four steps).
From the usual price it can reach the ceiling or the floor in about four real
days.

  +market/history <catalog>:<code> [market=<venue>]

Shows a house's ask and bid for a good at this venue, an IC week a row, newest
first, for the last 24 IC weeks (about a real week). A good reads as
tacsim:<good>, for example +market/history tacsim:metals.

## What the houses use up

Some houses use up what they buy. Syrtis Bonded at Deimos and the Selk
Townships Board at Titan build off-map, so each IC day they use up some of the
structural alloys and instrument boards their bids took in. The Lunar Habitat
Board feeds its people from its own provisions. A house that uses its stores up
wants more, so its bid stays open and its price comes back up.

Off-map buyers pay these houses back for what they use, so their bids stay
funded. That is the only money that comes into the lane after it opens: it
reaches you when you sell into their bids.

## Haulers

The Vesta Carrying Company of Ceres runs two Drays between the lane's ports. A
hauler buys a lot from one port's house and sells it to a house one route
away, whenever the price gap pays for the trip's fuel and stores with some
margin left. Its trips are real passages: a hauler bound from Ceres to Stickney
meets the Kirkwood picket like any other ship, and can be stopped there.

Deimos makes nothing of its own. What reaches it, from the haulers or from you,
is used up there, and its houses bid for more.

## Running costs

A port facility (a refinery, works, yard or workshop) costs its owner money
every IC day. An hour a slot is working costs its full share of the facility's
daily cost, and the fee you pay covers it. An idle hour costs a tenth of that. A
refinery nobody can book pays for each batch it refines instead. The money goes
out of the game, to nobody.

An owner who cannot pay goes into debt, and the facility is mothballed. It
takes no orders of its own, but your order still runs, and its fee pays the
debt first. The owner's prices rise and its bids stop, as for any house in
debt. It reopens once the debt is paid and the owner has seven IC days of
running costs in hand.

## Used parts

Each port's workshop bids for every part its bay fits, at 60% of the part's
reference price, four at a time. Sell one from your docked hold with
+market/sell <bid>=1, one part a sale. The workshop buys sound parts only: a
damaged or jammed part is scrap, and its bid refuses it. A part flagged hot
under a salvage claim stays where it lies, and nobody buys it.

The parts a workshop takes in, from its bids and from the hulls it scraps, go
up on its board as lot asks at the reference price, one of each part at a time.
A worn part is not listed, and no yard order or bay replace takes one from the
stores. A yard order buys its parts from the workshop's stores at its port.

## Hauls

  +haul/post <qty> <good> to=<port> fee=<cr> [value=<cr>] [go]

Posts a job to carry goods from your locker at this port to your locker at
another. The fee is escrowed from your account and paid on delivery. You
declare the goods' value, at most their reference price; the hauler escrows a
bond of a quarter of it, which is yours if the goods are not delivered.

  +haul/accept <job> [as=<org>] [go]

Takes a haul with your hull docked at its origin. The goods load into your
hold held for the job: you cannot sell, burn or move them. Deliver them with
+haul/deliver <job> go at the destination within 28 IC days, and the fee and
your bond follow. Goods still aboard when the term ends go back to the
shipper at your next port. +haul lists your hauls; +haul/withdraw takes back
one nobody has taken.

## Escorts

  +escort/post <hull> to=<port> fee=<cr> [go]
  +escort/post <hull> field=<site> fee=<cr> [go]

Posts a job to escort your hull on her next passage from the port she lies at,
or on her next sortie in a field. Whoever takes it with +escort/accept <job>
with=<hull> must stay near her for nine tenths of the journey: on the same
passage or in the same bubble in space, in the same field in a sortie. A link
drop inside the linkdead grace does not count against the escort, and neither
does an escort still at the port of departure in the hour after she leaves. The
job fails if she is wrecked or captured, or the escort leaves her. If her
journey has not begun within 28 IC days of the acceptance, the job is called
off: the fee and the escort's bond come back. A field= sortie names a port
with a field. +escort lists your escorts.
