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
into a vault of their own. Their payment is held first, the lot itself moves to
them, and you are paid once the deal settles. You do not need to be there.

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
starts, the order is cancelled, and its booking fee and bill go back to the
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

`+market/mine` lists your listings and bids, and how many more you may post.

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

A market that belongs to no port, such as a counter, trades only with people
standing at it.

## Chartered cargo

Goods in a Co-op chartered hold may go on a personal ask. When they sell, the
Co-op takes its share of the margin over its reference price, as it does on a
sale into a house's bid, and you are paid the rest.

## Fees

At a port, the port's authority takes a listing fee of 1% of what you list (at
least 10 cr), once, when you post it, and 3% of each sale. A further 1% of each
sale is a levy to the port's treasury. A house's own listings pay none of these.

## Berths

A hull docked at a port with an authority owes it 5 cr an IC day, from her
title holder's account. A day you cannot pay is owed as arrears, and paid once
your account is funded; a long absence stops adding to it after about ten real
days. Arrears never keep a hull at her berth, but they are paid out of her next
sale at that port. A house's own hulls at their home port owe nothing.

## The job board

  +market/jobs [retrieval|passage|survey] [market=<venue>]

Lists the jobs posted at the port you stand in, or at the market you name.
Each job says what it pays, any bond you would escrow, when it is due, and the
command that takes it:

- **Retrieval:** bring a hull to a port for its title holder. Take it with
  +salvage/accept. You then have the retrieval window to deliver her.
- **Passage:** carry a waiting traveller to their destination. The fare shown is
  their opening offer; +passage/offer opens the haggling. The fare is paid on
  arrival.
- **Survey:** Occator's commissions for fresh soundings of one block. Take one
  with +survey/accept; you have 28 IC days from accepting.

A job paying more than 10,000 cr asks for a bond of a fifth of its pay. The bond
is escrowed from your account when you take the job. It comes back with the pay
when the work is done, and goes to the poster if the work is not done in time.

To take a retrieval for your organization, add as=<org>:
+salvage/accept <job> as=<org> go. You need the right to spend the
organization's money. The pay goes to its account, and its bond comes from
there.
